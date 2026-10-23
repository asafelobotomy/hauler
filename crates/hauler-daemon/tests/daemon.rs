use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hauler_daemon::{Config, run};
use hauler_ipc::{AddParams, Client, ClientError, RpcError, SettingsPatch, TaskId, TaskInfo, TaskState};
use hauler_testserver::{Config as ServerConfig, fill, spawn};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const MIB: u64 = 1024 * 1024;

struct Daemon {
    socket: PathBuf,
    stop: CancellationToken,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Daemon {
    async fn start(dir: &Path) -> Self {
        let socket = dir.join("run/hauler.sock");
        let downloads = dir.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let config = Config {
            socket: socket.clone(),
            db: Some(dir.join("hauler.db")),
            download_dir: downloads,
            system_integration: false,
        };
        let stop = CancellationToken::new();
        let handle = tokio::spawn(run(config, stop.clone()));
        for _ in 0..100 {
            if Client::connect(&socket).await.is_ok() {
                return Self { socket, stop, handle };
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("daemon did not start");
    }

    async fn client(&self) -> Client {
        Client::connect(&self.socket).await.unwrap()
    }

    async fn stop(self) {
        self.stop.cancel();
        self.handle.await.unwrap().unwrap();
    }
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("haulerd-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha256_of_content(size: u64) -> Vec<u8> {
    let mut data = vec![0u8; size as usize];
    fill(0, &mut data);
    Sha256::digest(&data).to_vec()
}

fn assert_file(path: &Path, size: u64) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(Sha256::digest(&data).to_vec(), sha256_of_content(size), "{}", path.display());
}

async fn wait_for(client: &mut Client, id: TaskId, what: &str, f: impl Fn(&TaskInfo) -> bool) -> TaskInfo {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let t = client.get(id).await.unwrap();
        if f(&t) {
            return t;
        }
        assert!(t.state != TaskState::Failed, "download failed: {:?}", t.error);
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {t:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_concurrent_downloads_complete_intact() {
    let size = 2 * MIB + 333;
    let server = spawn(ServerConfig { size, bytes_per_sec: Some(MIB), ..Default::default() }).await.unwrap();
    let dir = tempdir("fifty");
    let daemon = Daemon::start(&dir).await;
    let mut client = daemon.client().await;
    client
        .set_settings(&SettingsPatch { max_active: Some(50), connections: Some(4), ..Default::default() })
        .await
        .unwrap();

    let mut ids = Vec::new();
    for i in 0..50 {
        let t = client
            .add(&AddParams { url: server.url(), filename: Some(format!("f{i:02}.bin")), ..Default::default() })
            .await
            .unwrap();
        ids.push(t.id);
    }

    let started = Instant::now();
    let mut peak_running = 0;
    loop {
        let status = client.status().await.unwrap();
        peak_running = peak_running.max(status.running);
        let tasks = client.list().await.unwrap();
        if let Some(failed) = tasks.iter().find(|t| t.state == TaskState::Failed) {
            panic!("download failed: {failed:?}");
        }
        if tasks.iter().all(|t| t.state == TaskState::Completed) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "not done in 60 s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(peak_running, 50, "all 50 should run at once");
    for i in 0..50 {
        assert_file(&dir.join(format!("downloads/f{i:02}.bin")), size);
    }
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_resume_and_survive_a_daemon_restart() {
    let size = 16 * MIB;
    let server = spawn(ServerConfig { size, bytes_per_sec: Some(MIB), ..Default::default() }).await.unwrap();
    let dir = tempdir("restart");
    let daemon = Daemon::start(&dir).await;
    let mut client = daemon.client().await;
    let id = client.add(&AddParams { url: server.url(), connections: Some(2), ..Default::default() }).await.unwrap().id;

    wait_for(&mut client, id, "progress", |t| t.received > 2 * MIB).await;
    client.pause(id).await.unwrap();
    let paused = wait_for(&mut client, id, "pause", |t| t.state == TaskState::Paused && t.connections == 0).await;
    assert!(paused.received < size);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(client.get(id).await.unwrap().received, paused.received, "no progress while paused");

    client.resume(id).await.unwrap();
    wait_for(&mut client, id, "more progress", |t| t.received > paused.received + MIB).await;

    // Stop the daemon mid-download; the next one must pick the task back up.
    drop(client);
    daemon.stop().await;
    let daemon = Daemon::start(&dir).await;
    let mut client = daemon.client().await;
    let t = client.get(id).await.unwrap();
    assert!(matches!(t.state, TaskState::Queued | TaskState::Running), "{t:?}");
    assert!(t.received > paused.received);

    let done = wait_for(&mut client, id, "completion", |t| t.state == TaskState::Completed).await;
    assert_eq!(done.name.as_deref(), Some("file.bin"));
    assert_file(&dir.join("downloads/file.bin"), size);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_limit_and_priority_order() {
    let size = 2 * MIB;
    let server = spawn(ServerConfig { size, ..Default::default() }).await.unwrap();
    let dir = tempdir("limit");
    let daemon = Daemon::start(&dir).await;
    let mut client = daemon.client().await;
    client
        .set_settings(&SettingsPatch { max_active: Some(1), global_limit: Some(MIB), ..Default::default() })
        .await
        .unwrap();

    let low = client
        .add(&AddParams { url: server.url(), filename: Some("low.bin".into()), paused: true, ..Default::default() })
        .await
        .unwrap();
    let high = client
        .add(&AddParams {
            url: server.url(),
            filename: Some("high.bin".into()),
            priority: Some(5),
            paused: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let started = Instant::now();
    client.resume_all().await.unwrap();

    let first = wait_for(&mut client, high.id, "high first", |t| t.state == TaskState::Completed).await;
    assert_ne!(client.get(low.id).await.unwrap().state, TaskState::Completed, "low priority finished first");
    let elapsed = started.elapsed();
    // 2 MiB at 1 MiB/s, less the limiter's 256 KiB burst.
    assert!(elapsed >= Duration::from_millis(1500), "limit not applied: {elapsed:?}");
    assert_eq!(first.name.as_deref(), Some("high.bin"));

    client.set_settings(&SettingsPatch { global_limit: Some(0), ..Default::default() }).await.unwrap();
    wait_for(&mut client, low.id, "low", |t| t.state == TaskState::Completed).await;
    daemon.stop().await;
}

#[tokio::test]
async fn rejects_bad_requests_and_removes_with_files() {
    let server = spawn(ServerConfig { size: MIB, ..Default::default() }).await.unwrap();
    let dir = tempdir("errors");
    let daemon = Daemon::start(&dir).await;
    let mut client = daemon.client().await;

    let bad = |r: Result<TaskInfo, ClientError>| match r {
        Err(ClientError::Rpc(e)) => e.code,
        other => panic!("expected an error, got {other:?}"),
    };
    let add =
        |url: &str, dir: Option<&str>| AddParams { url: url.into(), dir: dir.map(Into::into), ..Default::default() };
    assert_eq!(bad(client.add(&add("ftp://x/y", None)).await), RpcError::INVALID_PARAMS);
    assert_eq!(bad(client.add(&add("not a url", None)).await), RpcError::INVALID_PARAMS);
    assert_eq!(bad(client.add(&add(&server.url(), Some("relative/dir"))).await), RpcError::INVALID_PARAMS);
    assert_eq!(bad(client.get(9999).await), RpcError::NOT_FOUND);

    let id = client.add(&add(&server.url(), None)).await.unwrap().id;
    wait_for(&mut client, id, "completion", |t| t.state == TaskState::Completed).await;
    let file = dir.join("downloads/file.bin");
    assert!(file.exists());
    assert_eq!(bad(client.pause(id).await), RpcError::INVALID_STATE);
    client.remove(id, true).await.unwrap();
    assert!(!file.exists());
    assert!(client.list().await.unwrap().is_empty());
    daemon.stop().await;
}
