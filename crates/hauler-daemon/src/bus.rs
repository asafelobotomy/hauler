//! `org.hauler.Hauler1` on the session bus: a small status and control
//! interface for desktop shells (uQ's bar widget), plus a launcher provider
//! (`org.uq.Launcher1.Provider`) that offers "Download with hauler" for a
//! typed or pasted http(s) URL.
//!
//! Both objects live on one connection that owns `org.hauler.Hauler1` while
//! the daemon runs:
//!
//! - `/org/hauler/Hauler1`, interface `org.hauler.Hauler1` ([`Hauler1`]):
//!   properties `ActiveCount u`, `QueuedCount u`, `BytesDone t`,
//!   `BytesTotal t`; methods `PauseAll()`, `ResumeAll()`, `Add(s url)`,
//!   `Queue() -> a{sv}`; signal `Finished(s id, s name, b ok)`.
//! - `/org/uq/Launcher1`, interface `org.uq.Launcher1.Provider`
//!   ([`provider::LauncherProvider`]).
//!
//! `PropertiesChanged` goes out at most once a second, with only the
//! properties that changed. Anyone on the user's session bus may call these
//! methods; that is the same trust as the JSON-RPC socket, which only the
//! user can open.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use hauler_ipc::{Event, TaskId, TaskInfo, TaskState};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zbus::Variant;
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;

use crate::manager::Manager;

mod hauler1;
pub mod provider;
#[cfg(test)]
mod tests;

pub use hauler1::Hauler1;

/// The well-known name the daemon owns while it runs.
pub const BUS_NAME: &str = "org.hauler.Hauler1";
/// Where [`Hauler1`] lives.
pub const PATH: &str = "/org/hauler/Hauler1";
pub const INTERFACE: &str = "org.hauler.Hauler1";
/// Where the launcher provider lives, as uQ expects.
pub const PROVIDER_PATH: &str = "/org/uq/Launcher1";
/// `PropertiesChanged` is sent at most this often.
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// The aggregate the properties show, over the downloads still to finish:
/// running, queued and waiting ones. Paused, completed and failed downloads
/// are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Running downloads.
    pub active: u32,
    /// Queued downloads, and those held for the network.
    pub queued: u32,
    /// Bytes received so far.
    pub bytes_done: u64,
    /// Their total size; 0 while any size is unknown.
    pub bytes_total: u64,
}

pub fn aggregate<'a>(tasks: impl IntoIterator<Item = &'a TaskInfo>) -> Totals {
    let mut t = Totals::default();
    let mut known = true;
    for task in tasks {
        match task.state {
            TaskState::Running => t.active += 1,
            TaskState::Queued | TaskState::Waiting => t.queued += 1,
            _ => continue,
        }
        t.bytes_done = t.bytes_done.saturating_add(task.received);
        match task.size {
            Some(size) if size > 0 => t.bytes_total = t.bytes_total.saturating_add(size),
            _ => known = false,
        }
    }
    if !known {
        t.bytes_total = 0;
    }
    t
}

impl Totals {
    /// The properties that differ from `before`, by D-Bus name.
    fn changes(&self, before: &Totals) -> HashMap<&'static str, Variant<'static>> {
        let mut out = HashMap::new();
        if self.active != before.active {
            out.insert("ActiveCount", Variant::from(self.active));
        }
        if self.queued != before.queued {
            out.insert("QueuedCount", Variant::from(self.queued));
        }
        if self.bytes_done != before.bytes_done {
            out.insert("BytesDone", Variant::from(self.bytes_done));
        }
        if self.bytes_total != before.bytes_total {
            out.insert("BytesTotal", Variant::from(self.bytes_total));
        }
        out
    }
}

/// Downloads that reached Completed or Failed since `states` was taken;
/// `states` is brought up to date.
fn newly_finished(states: &mut HashMap<TaskId, TaskState>, tasks: &[TaskInfo]) -> Vec<TaskInfo> {
    let mut out = Vec::new();
    for task in tasks {
        let before = states.insert(task.id, task.state);
        if task.state.is_finished() && before.is_some_and(|s| !s.is_finished()) {
            out.push(task.clone());
        }
    }
    out
}

/// Connect to the session bus and serve until `shutdown`. Without a session
/// bus, or with the name taken, the daemon carries on without it.
pub async fn run(manager: Arc<Manager>, shutdown: CancellationToken) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => return info!("no session bus, {BUS_NAME} off: {e}"),
    };
    if let Err(e) = serve(&conn, manager, shutdown).await {
        warn!("could not serve {BUS_NAME}: {e}");
    }
}

/// Register both objects on `conn`, take [`BUS_NAME`], and keep the
/// properties and signals current until `shutdown`; then give the name up.
pub async fn serve(conn: &zbus::Connection, manager: Arc<Manager>, shutdown: CancellationToken) -> zbus::Result<()> {
    // Objects first, so whoever sees the name appear can call them at once.
    let server = conn.object_server();
    server.at(PATH, Hauler1::new(manager.clone())).await?;
    server.at(PROVIDER_PATH, provider::LauncherProvider::new(manager.clone())).await?;
    let events = manager.subscribe();
    conn.request_name(BUS_NAME).await?;
    info!("serving {BUS_NAME} on the session bus");
    publish(conn, &manager, events, &shutdown).await?;
    let _ = conn.release_name(BUS_NAME).await;
    Ok(())
}

/// Turn queue events into `PropertiesChanged` (coalesced, at most once per
/// [`MIN_INTERVAL`]) and `Finished` (at once).
async fn publish(
    conn: &zbus::Connection,
    manager: &Manager,
    mut events: broadcast::Receiver<Event>,
    shutdown: &CancellationToken,
) -> zbus::Result<()> {
    let emitter = SignalEmitter::new(conn, PATH)?;
    let interface = InterfaceName::from_static_str_unchecked(INTERFACE);
    let tasks = manager.list();
    let mut states: HashMap<TaskId, TaskState> = tasks.iter().map(|t| (t.id, t.state)).collect();
    let mut sent = aggregate(&tasks);
    let mut dirty = false;
    let mut next = Instant::now();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            _ = tokio::time::sleep_until(next), if dirty => {
                dirty = false;
                let now = aggregate(&manager.list());
                let changed = now.changes(&sent);
                if changed.is_empty() {
                    continue;
                }
                let sent_ok = zbus::fdo::Properties::properties_changed(
                    &emitter,
                    interface.clone(),
                    changed.iter().map(|(k, v)| (*k, v.clone())).collect(),
                    Cow::Borrowed(&[]),
                )
                .await;
                if let Err(e) = sent_ok {
                    debug!("PropertiesChanged: {e}");
                }
                sent = now;
                next = Instant::now() + MIN_INTERVAL;
            }
            received = events.recv() => {
                let finished = match received {
                    Ok(Event::TaskAdded { task } | Event::TaskUpdated { task }) => {
                        newly_finished(&mut states, std::slice::from_ref(&task))
                    }
                    Ok(Event::TaskRemoved { id }) => {
                        states.remove(&id);
                        Vec::new()
                    }
                    Ok(Event::Progress { .. }) => Vec::new(),
                    Ok(Event::Settings { .. } | Event::Network { .. }) => continue,
                    // Missed some: compare against the whole list instead.
                    Err(RecvError::Lagged(_)) => {
                        let tasks = manager.list();
                        states.retain(|id, _| tasks.iter().any(|t| t.id == *id));
                        newly_finished(&mut states, &tasks)
                    }
                    Err(RecvError::Closed) => return Ok(()),
                };
                dirty = true;
                for task in finished {
                    let name = task.name.clone().unwrap_or_else(|| task.url.clone());
                    let ok = task.state == TaskState::Completed;
                    if let Err(e) = Hauler1::finished(&emitter, &task.id.to_string(), &name, ok).await {
                        debug!("Finished: {e}");
                    }
                }
            }
        }
    }
}
