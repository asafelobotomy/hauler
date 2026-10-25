//! Servers that turn downloads away, and recovering from a bad checksum.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use hauler_engine::{CancellationToken, Error, Options, Outcome, client, prepare};
use hauler_testserver::{Config, content_byte, spawn};

const MIB: u64 = 1024 * 1024;

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hauler-edges-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts() -> Options {
    Options { min_split: 256 * 1024, save_interval: Duration::from_millis(200), ..Options::default() }
}

#[tokio::test]
async fn a_first_probe_sent_to_a_busy_page_is_refused_not_saved() {
    // One download at a time; the second is sent to /busy.html.
    let server = spawn(Config { size: 4 * MIB, bytes_per_sec: Some(MIB), max_streams: Some(1), ..Config::default() })
        .await
        .unwrap();
    let dir = tempdir("busy-first");
    let first = prepare(client().unwrap(), &server.url(), &dir, Options { connections: 1, ..opts() }).await.unwrap();
    let cancel = CancellationToken::new();
    let running = tokio::spawn(first.run(cancel.clone()));
    while server.stats.streaming.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let second = prepare(client().unwrap(), &server.url(), &dir, Options { max_retries: 0, ..opts() }).await;
    let err = second.err().expect("a busy page must not be saved as the file");
    assert!(err.is_turned_away() && err.to_string().contains("/busy.html"), "{err}");
    assert!(!dir.join("busy.html").exists());
    assert!(!dir.join("busy.html.part").exists());

    cancel.cancel();
    let _ = running.await;
}

#[tokio::test]
async fn after_a_checksum_mismatch_the_next_run_starts_over() {
    let size = 4 * MIB;
    // Slow enough that resume state is saved several times on the way.
    let server = spawn(Config { size, bytes_per_sec: Some(2 * MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("badsum-retry");
    let wrong = format!("sha256:{}", "0".repeat(64)).parse().unwrap();
    let o = Options { connections: 2, checksum: Some(wrong), ..opts() };
    let task = prepare(client().unwrap(), &server.url(), &dir, o).await.unwrap();
    let err = task.run(CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");

    // Retrying must fetch the data again rather than re-check the same bytes.
    let before = server.stats.requests.load(Ordering::SeqCst);
    let task = prepare(client().unwrap(), &server.url(), &dir, opts()).await.unwrap();
    assert_eq!(task.target(), dir.join("file.bin"), "the retry reuses the same name");
    assert_eq!(task.resumed_bytes(), 0);
    let Outcome::Completed(path) = task.run(CancellationToken::new()).await.unwrap() else { panic!() };
    assert!(server.stats.requests.load(Ordering::SeqCst) > before + 1, "only probed, never downloaded");
    let data = std::fs::read(path).unwrap();
    assert!(data.len() as u64 == size && (0..size).all(|i| data[i as usize] == content_byte(i)));
}
