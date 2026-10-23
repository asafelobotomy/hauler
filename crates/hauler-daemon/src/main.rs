use std::io::IsTerminal;
use std::path::PathBuf;

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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("HAULER_LOG").unwrap_or_else(|_| "info".into()))
        .without_time() // journald adds timestamps
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let config = Config {
        socket: args.socket.unwrap_or_else(hauler_ipc::default_socket_path),
        db: Some(args.db.unwrap_or_else(default_db_path)),
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
