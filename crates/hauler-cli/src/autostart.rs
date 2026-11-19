//! Starting `haulerd` when a command finds nobody listening, so the settings page, the launcher and scripts work
//! without the Hauler app (or `systemctl --user start hauler.socket`) having been started first.
//!
//! First the systemd user socket is tried (the package installs it, and the daemon it starts is the one the
//! session already knows); when systemd has no such unit, `haulerd` is started on its own, detached from the caller.
//! `HAULER_NO_AUTOSTART=1` turns this off.

use std::fs::{DirBuilder, OpenOptions};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hauler_ipc::{Client, ClientError};

/// How long a fresh daemon gets to start listening.
const WAIT: Duration = Duration::from_secs(6);

/// Connects to the daemon at `path`, starting it first when it isn't there.
pub async fn connect(path: &Path) -> Result<Client, ClientError> {
    let first = match Client::connect(path).await {
        Ok(client) => return Ok(client),
        Err(e @ ClientError::Connect { .. }) => e,
        Err(e) => return Err(e),
    };
    if std::env::var_os("HAULER_NO_AUTOSTART").is_some() {
        return Err(first);
    }
    let _lock = lock(path);
    // Someone else may have started it while we waited for the lock.
    if let Ok(client) = Client::connect(path).await {
        return Ok(client);
    }
    if !start(path) {
        return Err(first);
    }
    let deadline = Instant::now() + WAIT;
    loop {
        match Client::connect(path).await {
            Ok(client) => return Ok(client),
            Err(_) if Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(60)).await,
            Err(_) => return Err(first),
        }
    }
}

/// One starter at a time (the settings page runs several commands at once). Held until dropped.
fn lock(socket: &Path) -> Option<std::fs::File> {
    let dir = socket.parent()?;
    DirBuilder::new().recursive(true).mode(0o700).create(dir).ok()?;
    let file = OpenOptions::new().create(true).append(true).open(dir.join("autostart.lock")).ok()?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive).ok()?;
    Some(file)
}

/// True when something was started (it may still take a moment to listen).
fn start(socket: &Path) -> bool {
    if default_socket(socket) && systemd_socket() {
        return true;
    }
    let mut cmd = Command::new(haulerd());
    if !default_socket(socket) {
        cmd.arg("--socket").arg(socket);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory; it keeps the daemon alive when the caller (a
    // settings page command) is stopped or its process group is killed.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    // The child is left running on purpose; it ends with the session.
    cmd.spawn().is_ok()
}

fn default_socket(path: &Path) -> bool {
    std::env::var_os("HAULER_SOCKET").is_none() && path == hauler_ipc::default_socket_path()
}

/// Asks the user's systemd to listen on the socket; true when it did (the daemon starts on the first call).
fn systemd_socket() -> bool {
    Command::new("systemctl")
        .args(["--user", "--no-block", "start", "hauler.socket"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `haulerd` beside this program, else the one on PATH.
fn haulerd() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(sibling) = exe.parent().map(|d| d.join("haulerd"))
        && sibling.is_file()
    {
        return sibling;
    }
    PathBuf::from("haulerd")
}
