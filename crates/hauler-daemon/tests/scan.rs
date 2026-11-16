//! Scanning finished downloads, with a stand-in for `oyst-cli` that follows oysterAV's documented contract.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hauler_daemon::{Config, run};
use hauler_ipc::{AddParams, Client, ScanState, SettingsPatch, TaskId, TaskInfo, TaskState};
use hauler_testserver::{Config as ServerConfig, spawn};
use tokio_util::sync::CancellationToken;

/// Says "threat" for a file with `bad` in its name, "busy" once for a file with `busy` in its name, else clean.
const FAKE: &str = r#"#!/bin/sh
file="$3"
case "$file" in
  *busy*) if [ ! -e "$file.tried" ]; then touch "$file.tried"; echo "another job" >&2; exit 3; fi ;;
esac
echo '{"event":"progress","stage":"clamav","percent":100.0}'
case "$file" in
  *bad*) echo '{"event":"result","clean":false,"findings":[{"pack":"clamav","path":"'"$file"'","threat_name":"Test.Threat"}],"pack_errors":[]}'; exit 1 ;;
  *) echo '{"event":"result","clean":true,"findings":[],"pack_errors":[]}'; exit 0 ;;
esac
"#;

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("haulerd-scan-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn wait_for(client: &mut Client, id: TaskId, what: &str, f: impl Fn(&TaskInfo) -> bool) -> TaskInfo {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let t = client.get(id).await.unwrap();
        if f(&t) {
            return t;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {t:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn scanned(t: &TaskInfo) -> Option<ScanState> {
    t.scan.as_ref().map(|r| r.state).filter(|s| *s != ScanState::Scanning)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_are_scanned_when_asked_and_when_the_setting_is_on() {
    let dir = tempdir("flow");
    let fake = dir.join("oyst-cli");
    std::fs::write(&fake, FAKE).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: this test binary has one test, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("HAULER_OYST_CLI", &fake) };

    let server = spawn(ServerConfig { size: 4096, ..Default::default() }).await.unwrap();
    let socket = dir.join("run/hauler.sock");
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let stop = CancellationToken::new();
    let handle = tokio::spawn(run(
        Config {
            socket: socket.clone(),
            db: Some(dir.join("hauler.db")),
            download_dir: downloads.clone(),
            system_integration: false,
        },
        stop.clone(),
    ));
    let mut client = loop {
        if let Ok(c) = Client::connect(&socket).await {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let add = |name: &str| AddParams { url: server.url(), filename: Some(name.into()), ..Default::default() };

    // Off by default: a download completes and nothing is scanned.
    let first = client.add(&add("plain.bin")).await.unwrap().id;
    let t = wait_for(&mut client, first, "completion", |t| t.state == TaskState::Completed).await;
    assert!(t.scan.is_none(), "{t:?}");

    // On demand, then the verdict.
    client.scan(first).await.unwrap();
    let t = wait_for(&mut client, first, "a verdict", |t| scanned(t).is_some()).await;
    assert_eq!(scanned(&t), Some(ScanState::Clean));

    // A download that is not finished can't be scanned.
    let paused = client.add(&AddParams { paused: true, ..add("later.bin") }).await.unwrap().id;
    assert!(client.scan(paused).await.is_err());

    // With the setting on, completion starts the scan: a threat names itself.
    client.set_settings(&SettingsPatch { scan_downloads: Some(true), ..Default::default() }).await.unwrap();
    let bad = client.add(&add("bad.bin")).await.unwrap().id;
    let t = wait_for(&mut client, bad, "a threat", |t| scanned(t).is_some()).await;
    assert_eq!(scanned(&t), Some(ScanState::Threat));
    assert_eq!(t.scan.unwrap().detail, "Test.Threat");
    assert!(Path::new(&downloads.join("bad.bin")).exists(), "a scan never removes the file");

    // The verdict survives a restart.
    stop.cancel();
    handle.await.unwrap().unwrap();
    let stop = CancellationToken::new();
    let handle = tokio::spawn(run(
        Config {
            socket: socket.clone(),
            db: Some(dir.join("hauler.db")),
            download_dir: downloads,
            system_integration: false,
        },
        stop.clone(),
    ));
    let mut client = loop {
        if let Ok(c) = Client::connect(&socket).await {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let t = client.get(bad).await.unwrap();
    assert_eq!(scanned(&t), Some(ScanState::Threat), "{t:?}");
    stop.cancel();
    handle.await.unwrap().unwrap();
}
