use std::fs::{self, File, OpenOptions};
use std::io::IsTerminal;
use std::os::fd::AsFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use clap::Parser;
use hauler_daemon::{Config, default_db_path, default_download_dir, run};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

/// The hauler download daemon.
#[derive(Parser)]
#[command(name = "haulerd", version)]
struct Args {
    /// Socket to listen on (ignored under systemd socket activation).
    #[arg(long)]
    socket: Option<PathBuf>,
    /// SQLite database for the queue.
    #[arg(long)]
    db: Option<PathBuf>,
    /// Skip D-Bus sleep inhibition and network monitoring.
    #[arg(long)]
    no_system: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let db = args.db.unwrap_or_else(default_db_path);
    init_logging(&db);
    let config = Config {
        socket: args.socket.unwrap_or_else(hauler_ipc::default_socket_path),
        db: Some(db),
        download_dir: default_download_dir(),
        system_integration: !args.no_system,
    };

    let shutdown = CancellationToken::new();
    let on_signal = shutdown.clone();
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        on_signal.cancel();
    });
    run(config, shutdown).await
}

/// Log to stderr when it is the journal or a terminal. Started by the desktop
/// app, stderr is whatever the app had, often a pipe nobody reads; then log
/// to `haulerd.log` beside the database, keeping the previous run's log as
/// `haulerd.log.old`.
fn init_logging(db: &Path) {
    let filter = || tracing_subscriber::EnvFilter::try_from_env("HAULER_LOG").unwrap_or_else(|_| "info".into());
    let stderr = std::io::stderr();
    if !(stderr.is_terminal() || stderr_is_journal())
        && let Some(file) = db.parent().and_then(|dir| open_log(dir).ok())
    {
        tracing_subscriber::fmt().with_env_filter(filter()).with_ansi(false).with_writer(Mutex::new(file)).init();
        return;
    }
    tracing_subscriber::fmt()
        .with_env_filter(filter())
        .without_time() // journald adds timestamps
        .with_ansi(stderr.is_terminal())
        .with_writer(std::io::stderr)
        .init();
}

/// systemd sets `JOURNAL_STREAM` to the device and inode of the stream it
/// connected; stderr is the journal only if it is still that stream.
fn stderr_is_journal() -> bool {
    let Some((dev, ino)) = std::env::var("JOURNAL_STREAM").ok().and_then(|v| {
        let (dev, ino) = v.split_once(':')?;
        Some((dev.parse::<u64>().ok()?, ino.parse::<u64>().ok()?))
    }) else {
        return false;
    };
    let Ok(fd) = std::io::stderr().as_fd().try_clone_to_owned() else { return false };
    File::from(fd).metadata().is_ok_and(|m| m.dev() == dev && m.ino() == ino)
}

/// URLs in the log can carry download keys, so only the user may read it.
/// The running daemon holds a lock on its log; a second one (which is about
/// to find the socket taken) leaves that log and its predecessor alone.
fn open_log(dir: &Path) -> std::io::Result<File> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let path = dir.join("haulerd.log");
    if let Ok(current) = File::open(&path)
        && !lock(&current)
    {
        return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "another haulerd is logging"));
    }
    let _ = fs::rename(&path, dir.join("haulerd.log.old"));
    let file = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    lock(&file);
    Ok(file)
}

/// Take an exclusive lock; false only when another process holds one.
fn lock(file: &File) -> bool {
    use rustix::fs::{FlockOperation, flock};
    !matches!(flock(file, FlockOperation::NonBlockingLockExclusive), Err(rustix::io::Errno::WOULDBLOCK))
}

#[cfg(test)]
mod tests {
    use super::open_log;

    #[test]
    fn a_second_daemon_leaves_the_running_ones_log_alone() {
        let dir = std::env::temp_dir().join(format!("haulerd-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("haulerd.log"), b"last run").unwrap();
        let running = open_log(&dir).unwrap();
        assert!(open_log(&dir).is_err());
        assert_eq!(std::fs::read(dir.join("haulerd.log.old")).unwrap(), b"last run", "rotated once, by the first");
        drop(running);
        // Once it stops, the next start rotates as usual.
        let _next = open_log(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
