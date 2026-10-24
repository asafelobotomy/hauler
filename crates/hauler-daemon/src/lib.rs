//! `haulerd`: the download daemon. It owns the queue, runs downloads with
//! hauler-engine, and serves the JSON-RPC API on a Unix socket.

use std::path::PathBuf;

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use tracing::info;

mod desktop;
pub mod manager;
mod rpc;
mod socket;
pub mod store;
mod system;

pub use manager::{Manager, default_download_dir};

pub struct Config {
    pub socket: PathBuf,
    /// SQLite database; `None` keeps everything in memory.
    pub db: Option<PathBuf>,
    /// Default download directory for a fresh database.
    pub download_dir: PathBuf,
    /// Sleep inhibition and NetworkManager monitoring over D-Bus.
    pub system_integration: bool,
}

/// `$XDG_STATE_HOME/hauler/hauler.db`, else `~/.local/state/hauler/hauler.db`.
pub fn default_db_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into()).join(".local/state")
    });
    state.join("hauler").join("hauler.db")
}

/// Run until `shutdown` fires, then stop downloads cleanly so they resume on
/// the next start.
pub async fn run(config: Config, shutdown: CancellationToken) -> anyhow::Result<()> {
    let store = match &config.db {
        Some(path) => {
            if let Some(dir) = path.parent() {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
            }
            store::Store::open(path).with_context(|| format!("opening {}", path.display()))?
        }
        None => store::Store::in_memory()?,
    };
    let manager = Manager::new(store, &config.download_dir)?;

    let (listener, owns_socket) = match socket::from_systemd() {
        Some(l) => {
            info!("using socket from systemd");
            (l, false)
        }
        None => {
            let l = socket::bind(&config.socket).with_context(|| format!("binding {}", config.socket.display()))?;
            info!("listening on {}", config.socket.display());
            (l, true)
        }
    };
    let listener = tokio::net::UnixListener::from_std(listener)?;

    if config.system_integration {
        tokio::spawn(system::inhibit_sleep(manager.active_watch(), shutdown.clone()));
        tokio::spawn(system::watch_network(manager.clone(), shutdown.clone()));
        tokio::spawn(desktop::run(manager.clone(), shutdown.clone()));
    }
    let scheduler = tokio::spawn(manager.clone().run(shutdown.clone()));
    rpc::serve(listener, manager.clone(), shutdown.clone()).await;

    manager.shutdown().await;
    let _ = scheduler.await;
    if owns_socket {
        let _ = std::fs::remove_file(&config.socket);
    }
    info!("stopped");
    Ok(())
}
