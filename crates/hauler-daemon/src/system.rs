//! Desktop integration over the system D-Bus: keep the machine awake while
//! downloading, and hold downloads while offline or on a metered connection.
//! Both degrade to doing nothing when the services are missing.

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zbus::zvariant::OwnedFd;

use crate::manager::Manager;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1 {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    /// NMMetered: 0 unknown, 1 yes, 2 no, 3 guess yes, 4 guess no.
    #[zbus(property)]
    fn metered(&self) -> zbus::Result<u32>;
    /// NMConnectivityState: 0 unknown, 1 none, 2 portal, 3 limited, 4 full.
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;
}

/// Hold a logind sleep inhibitor whenever at least one download runs.
/// Closing the returned fd releases it.
pub async fn inhibit_sleep(mut active: watch::Receiver<u32>, shutdown: CancellationToken) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return info!("no system bus, sleep inhibition off: {e}"),
    };
    let login = match Login1Proxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => return info!("logind unavailable, sleep inhibition off: {e}"),
    };
    let mut held: Option<OwnedFd> = None;
    loop {
        let running = *active.borrow_and_update();
        if running > 0 && held.is_none() {
            match login.inhibit("sleep", "hauler", "Downloads in progress", "block").await {
                Ok(fd) => {
                    debug!("holding sleep inhibitor");
                    held = Some(fd);
                }
                Err(e) => warn!("could not inhibit sleep: {e}"),
            }
        } else if running == 0 && held.take().is_some() {
            debug!("released sleep inhibitor");
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            changed = active.changed() => if changed.is_err() { return },
        }
    }
}

/// Follow NetworkManager's connectivity and metered state.
pub async fn watch_network(manager: Arc<Manager>, shutdown: CancellationToken) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return info!("no system bus, network monitoring off: {e}"),
    };
    let nm = match NetworkManagerProxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => return info!("NetworkManager unavailable, network monitoring off: {e}"),
    };
    let mut connectivity_changes = nm.receive_connectivity_changed().await;
    let mut metered_changes = nm.receive_metered_changed().await;
    loop {
        // Unknown connectivity (checks disabled) counts as online.
        let online = matches!(nm.connectivity().await.unwrap_or(0), 0 | 4);
        let metered = matches!(nm.metered().await.unwrap_or(0), 1 | 3);
        manager.set_network(online, metered);
        tokio::select! {
            _ = shutdown.cancelled() => return,
            Some(_) = connectivity_changes.next() => {}
            Some(_) = metered_changes.next() => {}
            else => return,
        }
    }
}
