//! One app per user. A second launch, from the app menu or a link a browser
//! opens with `hauler-desktop <url>`, hands its links to the running app and
//! exits, instead of starting another window with another tray icon.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Held by the running app: the lock that makes it the one, and the socket
/// later launches talk to until the Backend takes it over.
static LOCK: Mutex<Option<File>> = Mutex::new(None);
static LISTENER: Mutex<Option<UnixListener>> = Mutex::new(None);

/// A message from a later launch is a few links, never more than this.
const MAX_MESSAGE: u64 = 64 * 1024;

/// http(s) links among `args`, which is where the `.desktop` file's `%U` puts them.
pub fn links(args: impl IntoIterator<Item = String>) -> Vec<String> {
    args.into_iter().filter(|a| a.starts_with("http://") || a.starts_with("https://")).collect()
}

/// Beside the daemon's socket, in a directory only this user can enter.
fn dir() -> PathBuf {
    let socket = hauler_ipc::default_socket_path();
    socket.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir)
}

/// Whether this process should go on to be the app. If another one is
/// running, it gets `urls` (or, with none, is asked to show its window).
pub fn claim(urls: &[String]) -> bool {
    match claim_in(&dir(), urls) {
        Ok(primary) => primary,
        Err(e) => {
            eprintln!("could not check for a running Hauler, starting anyway: {e}");
            true
        }
    }
}

fn claim_in(dir: &Path, urls: &[String]) -> std::io::Result<bool> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    hauler_ipc::check_private_dir(dir)?;
    let lock =
        OpenOptions::new().create(true).write(true).truncate(false).mode(0o600).open(dir.join("desktop.lock"))?;
    let socket = dir.join("desktop.sock");
    // SAFETY: flock(2) on a descriptor we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        // Only the lock holder binds, so a socket already there is stale.
        let _ = std::fs::remove_file(&socket);
        *LISTENER.lock().unwrap() = Some(UnixListener::bind(&socket)?);
        *LOCK.lock().unwrap() = Some(lock);
        return Ok(true);
    }
    // The running app may still be starting up and not listening yet.
    for _ in 0..50 {
        if let Ok(mut stream) = UnixStream::connect(&socket) {
            stream.write_all(urls.join("\n").as_bytes())?;
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(std::io::Error::other("the running Hauler does not answer"))
}

/// Serve later launches on a thread of its own: `on_launch` gets their links,
/// one per line, or an empty string for "show the window".
pub fn serve(on_launch: impl Fn(String) + Send + 'static) {
    let Some(listener) = LISTENER.lock().unwrap().take() else { return };
    let spawned = std::thread::Builder::new().name("hauler-single".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Some(message) = read_message(stream) {
                on_launch(message);
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("cannot listen for later launches: {e}");
    }
}

fn read_message(stream: UnixStream) -> Option<String> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut text = String::new();
    stream.take(MAX_MESSAGE).read_to_string(&mut text).ok()?;
    Some(links(text.lines().map(str::to_owned)).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_hands_its_links_to_the_first() {
        let dir = std::env::temp_dir().join(format!("hauler-single-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(claim_in(&dir, &[]).unwrap(), "the first launch is the app");

        let (tx, rx) = std::sync::mpsc::channel();
        serve(move |m| tx.send(m).unwrap());
        let urls = vec!["https://example.com/a.iso".to_string(), "https://example.com/b.iso".to_string()];
        assert!(!claim_in(&dir, &urls).unwrap(), "a later launch hands over");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), urls.join("\n"));
        assert!(!claim_in(&dir, &[]).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "", "no links: show the window");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_web_links_are_taken_from_arguments() {
        let args = ["--minimized", "https://a.example/x", "file:///etc/passwd", "http://b.example/y"];
        assert_eq!(links(args.map(String::from)), ["https://a.example/x", "http://b.example/y"]);
    }
}
