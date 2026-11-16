//! The two guards a settings page turns on: https only, and a free-space reserve.
//! Plus leftover partial downloads, which the storage page offers to clear.

use std::path::PathBuf;
use std::time::Duration;

use hauler_engine::{CancellationToken, Error, Options, client, free_space, partials, prepare};
use hauler_testserver::{Config, spawn};

const MIB: u64 = 1024 * 1024;

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hauler-safety-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts() -> Options {
    Options { min_split: 256 * 1024, save_interval: Duration::from_millis(100), ..Options::default() }
}

#[tokio::test]
async fn https_only_refuses_a_plain_http_address_before_any_request() {
    let server = spawn(Config { size: MIB, ..Config::default() }).await.unwrap();
    let dir = tempdir("https");
    let err = prepare(client().unwrap(), &server.url(), &dir, Options { https_only: true, ..opts() }).await;
    assert!(matches!(err.err(), Some(Error::Insecure(_))));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing was created");
    assert!(prepare(client().unwrap(), &server.url(), &dir, opts()).await.is_ok(), "off by default");
}

#[tokio::test]
async fn a_reserve_that_would_not_stay_free_refuses_the_download_and_leaves_no_file() {
    let server = spawn(Config { size: MIB, ..Config::default() }).await.unwrap();
    let dir = tempdir("space");
    let free = free_space(&dir).unwrap();
    assert!(free > 0);
    // A reserve of everything free can't hold a 1 MiB file as well.
    let err = prepare(client().unwrap(), &server.url(), &dir, Options { min_free: free, ..opts() }).await;
    assert!(matches!(err.err(), Some(Error::NoSpace { .. })));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no empty .part left behind");
    assert!(prepare(client().unwrap(), &server.url(), &dir, Options { min_free: 1, ..opts() }).await.is_ok());
}

#[tokio::test]
async fn partials_lists_unfinished_downloads_nobody_is_running() {
    let server = spawn(Config { size: 4 * MIB, bytes_per_sec: Some(MIB), ..Config::default() }).await.unwrap();
    let dir = tempdir("partials");
    std::fs::write(dir.join("notes.part"), b"not a download").unwrap();
    let task = prepare(client().unwrap(), &server.url(), &dir, Options { connections: 1, ..opts() }).await.unwrap();
    let name = task.target().file_name().unwrap().to_string_lossy().into_owned();
    let cancel = CancellationToken::new();
    let running = tokio::spawn(task.run(cancel.clone()));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(partials(&dir).unwrap().is_empty(), "held open by a running download");
    cancel.cancel();
    running.await.unwrap().unwrap();
    let found = partials(&dir).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].name, name);
    assert!(found[0].part.ends_with(format!("{name}.part")) && found[0].state.exists());
    assert!(found[0].bytes >= 4 * MIB, "counts the preallocated part file");
}
