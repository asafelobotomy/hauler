use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use hauler_engine::header::{self, HeaderMap};
use hauler_engine::{CancellationToken, Error, Options, Outcome, client, prepare, sha256_file};
use hauler_testserver::{Config, content_byte, fill, spawn};
use sha2::{Digest, Sha256};

const MIB: u64 = 1024 * 1024;

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hauler-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn expected_sha256(size: u64) -> String {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; MIB as usize];
    let mut pos = 0;
    while pos < size {
        let n = MIB.min(size - pos) as usize;
        fill(pos, &mut buf[..n]);
        hasher.update(&buf[..n]);
        pos += n as u64;
    }
    hex::encode(hasher.finalize())
}

fn assert_content(path: &Path, size: u64) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len() as u64, size);
    if let Some(i) = (0..size).find(|&i| data[i as usize] != content_byte(i)) {
        panic!("byte {i} differs");
    }
}

fn opts() -> Options {
    Options { min_split: 256 * 1024, save_interval: Duration::from_millis(200), ..Options::default() }
}

#[tokio::test]
async fn segmented_download_is_byte_exact_and_verifies_checksum() {
    let size = 12 * MIB + 12345;
    let server = spawn(Config { size, ..Config::default() }).await.unwrap();
    let dir = tempdir("basic");
    let checksum = format!("sha256:{}", expected_sha256(size)).parse().unwrap();
    let task =
        prepare(client().unwrap(), &server.url(), &dir, Options { checksum: Some(checksum), ..opts() }).await.unwrap();
    assert!(task.resumable());
    let progress = task.progress();
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };

    assert_eq!(path, dir.join("file.bin"));
    assert_content(&path, size);
    assert!(!dir.join("file.bin.part").exists());
    assert!(!dir.join("file.bin.hauler").exists());
    assert_eq!(progress.received.load(Ordering::Relaxed), size);
}

#[tokio::test]
async fn wrong_checksum_fails_and_keeps_partial_file() {
    let server = spawn(Config { size: 2 * MIB, ..Config::default() }).await.unwrap();
    let dir = tempdir("badsum");
    let checksum = format!("sha256:{}", "0".repeat(64)).parse().unwrap();
    let task =
        prepare(client().unwrap(), &server.url(), &dir, Options { checksum: Some(checksum), ..opts() }).await.unwrap();
    let err = task.run(CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
    assert!(dir.join("file.bin.part").exists());
}

#[tokio::test]
async fn server_without_ranges_uses_one_stream() {
    let size = 3 * MIB + 7;
    let server = spawn(Config { size, ranges: false, ..Config::default() }).await.unwrap();
    let dir = tempdir("noranges");
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert!(!task.resumable());
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert_content(&path, size);
}

#[tokio::test]
async fn pause_then_resume_continues_from_disk() {
    let size = 16 * MIB;
    // 4 connections x 2 MiB/s: about 2 s for the whole file.
    let server = spawn(Config { size, bytes_per_sec: Some(2 * MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("resume");
    let o = Options { connections: 4, ..opts() };

    let task = prepare(client().unwrap(), &server.url(), &dir, o.clone()).await.unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        stopper.cancel();
    });
    assert!(matches!(task.run(cancel).await.unwrap(), Outcome::Paused));
    assert!(dir.join("file.bin.hauler").exists());

    let task = prepare(client().unwrap(), &server.url(), &dir, o).await.unwrap();
    let resumed = task.resumed_bytes();
    assert!(resumed > MIB && resumed < size, "resumed {resumed}");
    assert_eq!(task.target(), dir.join("file.bin"));
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert_content(&path, size);
}

#[tokio::test]
async fn changed_remote_file_restarts_instead_of_resuming() {
    let size = 8 * MIB;
    let server = spawn(Config { size, bytes_per_sec: Some(2 * MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("changed");
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stopper.cancel();
    });
    task.run(cancel).await.unwrap();

    server.stats.version.fetch_add(1, Ordering::SeqCst);
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(task.resumed_bytes(), 0);
}

#[tokio::test]
async fn change_during_download_is_detected_by_if_range() {
    let size = 8 * MIB;
    // One slow connection guarantees idle workers split it, sending new range
    // requests after the file has changed.
    let server =
        spawn(Config { size, bytes_per_sec: Some(MIB), slow_every: Some((4, 10)), ..Config::default() }).await.unwrap();
    let dir = tempdir("midchange");
    let o = Options { connections: 4, min_split: 64 * 1024, ..opts() };
    let task = prepare(client().unwrap(), &server.url(), &dir, o).await.unwrap();
    let stats = server.stats.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stats.version.fetch_add(1, Ordering::SeqCst);
    });
    let err = task.run(CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, Error::Changed), "{err}");
}

#[tokio::test]
async fn idle_connections_split_slow_segments() {
    let size = 16 * MIB;
    // Every 3rd request is 10x slower; dynamic splitting and restarts should
    // keep the total time close to the fast connections' pace.
    let server = spawn(Config { size, bytes_per_sec: Some(4 * MIB), slow_every: Some((3, 10)), ..Config::default() })
        .await
        .unwrap();
    let dir = tempdir("slow");
    let o = Options { connections: 4, slow_grace: Duration::from_millis(500), ..opts() };
    let task = prepare(client().unwrap(), &server.url(), &dir, o).await.unwrap();
    let progress = task.progress();
    let started = std::time::Instant::now();
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    let elapsed = started.elapsed();
    assert_content(&path, size);
    assert!(progress.splits.load(Ordering::Relaxed) > 0);
    // Static 4-way split would take 4 MiB / 0.4 MiB/s = 10 s on the slow part.
    assert!(elapsed < Duration::from_secs(6), "took {elapsed:?}");
    assert_eq!(sha256_file(&path).unwrap(), expected_sha256(size));
}

#[tokio::test]
async fn a_connection_cap_keeps_the_connections_it_allows() {
    let size = 8 * MIB;
    // Like hosts that allow two downloads per client and send everything
    // else to a "too many requests" page.
    let server =
        spawn(Config { size, bytes_per_sec: Some(4 * MIB), max_streams: Some(2), ..Config::default() }).await.unwrap();
    let dir = tempdir("capped");
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert_content(&path, size);
    assert!(server.stats.refused.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn being_sent_to_another_page_on_resume_keeps_the_progress() {
    let size = 8 * MIB;
    let server = spawn(Config { size, bytes_per_sec: Some(MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("busy-resume");
    let o = Options { connections: 4, filename: Some("file.bin".into()), ..opts() };
    let task = prepare(client().unwrap(), &server.url(), &dir, o.clone()).await.unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        stopper.cancel();
    });
    assert!(matches!(task.run(cancel).await.unwrap(), Outcome::Paused));

    let busy = format!("http://{}/busy.html", server.addr);
    let redirect = spawn(Config { redirect_to: Some(busy), ..Config::default() }).await.unwrap();
    let err = prepare(client().unwrap(), &redirect.url(), &dir, Options { max_retries: 0, ..o.clone() })
        .await
        .err()
        .expect("a busy page must not replace the download");
    assert!(matches!(err, Error::Refused(_)), "{err}");
    assert_eq!(std::fs::metadata(dir.join("file.bin.part")).unwrap().len(), size);

    let task = prepare(client().unwrap(), &server.url(), &dir, o).await.unwrap();
    assert!(task.resumed_bytes() > 0);
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert_content(&path, size);
}

#[tokio::test]
async fn existing_file_gets_a_new_name() {
    let server = spawn(Config { size: MIB, ..Config::default() }).await.unwrap();
    let dir = tempdir("rename");
    std::fs::write(dir.join("file.bin"), b"keep me").unwrap();
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(task.target(), dir.join("file (1).bin"));
    task.run(CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read(dir.join("file.bin")).unwrap(), b"keep me");
}

#[tokio::test]
async fn credentials_stay_with_the_origin_they_were_given_for() {
    let size = 2 * MIB;
    let target = spawn(Config { size, ..Config::default() }).await.unwrap();
    let redirect = spawn(Config { redirect_to: Some(target.url()), ..Config::default() }).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, "session=secret".parse().unwrap());
    headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());

    // Redirected to another origin: the target never sees the credentials.
    let dir = tempdir("redirect");
    let o = Options { headers: headers.clone(), ..opts() };
    let task = prepare(client().unwrap(), &redirect.url(), &dir, o).await.unwrap();
    task.run(CancellationToken::new()).await.unwrap();
    assert_content(&dir.join("file.bin"), size);
    assert!(redirect.stats.cookie_requests.load(Ordering::SeqCst) > 0);
    assert_eq!(target.stats.cookie_requests.load(Ordering::SeqCst), 0);

    // Same origin: they are sent on every request.
    let before = target.stats.requests.load(Ordering::SeqCst);
    let dir = tempdir("same-origin");
    let task = prepare(client().unwrap(), &target.url(), &dir, Options { headers, ..opts() }).await.unwrap();
    task.run(CancellationToken::new()).await.unwrap();
    let requests = target.stats.requests.load(Ordering::SeqCst) - before;
    assert_eq!(target.stats.cookie_requests.load(Ordering::SeqCst), requests);
}

#[tokio::test]
async fn concurrent_downloads_never_share_a_part_file() {
    let size = 4 * MIB;
    let server = spawn(Config { size, bytes_per_sec: Some(2 * MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("shared");
    let a = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let b = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(a.target(), dir.join("file.bin"));
    assert_eq!(b.target(), dir.join("file (1).bin"));

    let (ra, rb) = tokio::join!(a.run(CancellationToken::new()), b.run(CancellationToken::new()));
    let (Outcome::Completed(pa), Outcome::Completed(pb)) = (ra.unwrap(), rb.unwrap()) else { panic!() };
    assert_content(&pa, size);
    assert_content(&pb, size);
}

#[tokio::test]
async fn a_paused_download_resumes_while_a_duplicate_takes_a_new_name() {
    let size = 8 * MIB;
    let server = spawn(Config { size, bytes_per_sec: Some(MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("dup-resume");
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        stopper.cancel();
    });
    task.run(cancel).await.unwrap();

    let resumed = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let duplicate = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(resumed.target(), dir.join("file.bin"));
    assert!(resumed.resumed_bytes() > 0);
    assert_eq!(duplicate.target(), dir.join("file (1).bin"));
    assert_eq!(duplicate.resumed_bytes(), 0);
}

#[tokio::test]
async fn pausing_a_non_resumable_download_keeps_its_name() {
    let size = 4 * MIB;
    let server =
        spawn(Config { size, ranges: false, bytes_per_sec: Some(2 * MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("stream-pause");
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        stopper.cancel();
    });
    assert!(matches!(task.run(cancel).await.unwrap(), Outcome::Paused));

    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(task.target(), dir.join("file.bin"));
    assert!(!task.resumable());
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert_content(&path, size);
    let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    left.sort();
    assert_eq!(left, ["file.bin"], "no orphaned partial files");
}

#[tokio::test]
async fn certificate_errors_fail_without_retrying() {
    let server = spawn(Config::default()).await.unwrap();
    // Plain HTTP on the other end: the TLS handshake can never succeed.
    let url = server.url().replace("http://", "https://");
    let dir = tempdir("tls");
    let started = std::time::Instant::now();
    let err = match prepare(client().unwrap(), &url, &dir, opts()).await {
        Ok(_) => panic!("TLS against a plain HTTP server succeeded"),
        Err(e) => e,
    };
    assert!(!matches!(err, Error::RetriesExhausted { .. }), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
}
