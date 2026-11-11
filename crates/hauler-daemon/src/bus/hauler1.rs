//! The `org.hauler.Hauler1` interface at `/org/hauler/Hauler1`.

use std::collections::HashMap;
use std::sync::Arc;

use hauler_ipc::{AddParams, RpcError, TaskInfo, TaskState};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, Variant};

use super::aggregate;
use crate::manager::Manager;

/// At most this many entries come back from `Queue`.
pub const QUEUE_MAX: usize = 50;

pub struct Hauler1 {
    manager: Arc<Manager>,
}

impl Hauler1 {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager }
    }
}

/// Map a queue error onto a D-Bus error: bad input is `InvalidArgs`.
pub(super) fn to_fdo(e: RpcError) -> zbus::fdo::Error {
    if e.code == RpcError::INVALID_PARAMS {
        zbus::fdo::Error::InvalidArgs(e.message)
    } else {
        zbus::fdo::Error::Failed(e.message)
    }
}

/// Unfinished downloads in the order a queue view lists them: running, then
/// held for the network, queued, paused and failed; by priority, then age.
pub fn queue_order(mut tasks: Vec<TaskInfo>) -> Vec<TaskInfo> {
    let rank = |s: TaskState| match s {
        TaskState::Running => 0,
        TaskState::Waiting => 1,
        TaskState::Queued => 2,
        TaskState::Paused => 3,
        TaskState::Failed => 4,
        TaskState::Completed => 5,
    };
    tasks.retain(|t| t.state != TaskState::Completed);
    tasks.sort_by_key(|t| (rank(t.state), -i64::from(t.priority), t.id));
    tasks.truncate(QUEUE_MAX);
    tasks
}

fn entry(t: &TaskInfo) -> OwnedVariant {
    let mut m: HashMap<&str, Variant<'_>> = HashMap::new();
    m.insert("id", Variant::from(t.id.to_string()));
    m.insert("name", Variant::from(t.name.clone().unwrap_or_else(|| t.url.clone())));
    m.insert("state", Variant::from(t.state.as_str()));
    m.insert("received", Variant::from(t.received));
    m.insert("size", Variant::from(t.size.unwrap_or(0)));
    m.insert("speed", Variant::from(t.speed));
    // A dict of basic values always converts.
    OwnedVariant::try_from(Variant::from(m)).expect("a{sv} converts")
}

#[zbus::interface(name = "org.hauler.Hauler1")]
impl Hauler1 {
    /// Running downloads.
    #[zbus(property)]
    fn active_count(&self) -> u32 {
        aggregate(&self.manager.list()).active
    }

    /// Downloads waiting for a slot or for the network.
    #[zbus(property)]
    fn queued_count(&self) -> u32 {
        aggregate(&self.manager.list()).queued
    }

    /// Bytes received across running and queued downloads.
    #[zbus(property)]
    fn bytes_done(&self) -> u64 {
        aggregate(&self.manager.list()).bytes_done
    }

    /// Their total size; 0 while any of them has an unknown size.
    #[zbus(property)]
    fn bytes_total(&self) -> u64 {
        aggregate(&self.manager.list()).bytes_total
    }

    /// Pause every running, queued or waiting download.
    fn pause_all(&self) {
        self.manager.pause_all();
    }

    /// Resume every paused or failed download.
    fn resume_all(&self) {
        self.manager.resume_all();
    }

    /// Queue `url` with the default settings. The URL is checked exactly as
    /// `hauler add` has it checked: an http or https URL, else `InvalidArgs`.
    fn add(&self, url: String) -> zbus::fdo::Result<()> {
        self.manager.add(AddParams { url, ..Default::default() }).map(|_| ()).map_err(to_fdo)
    }

    /// Unfinished downloads, at most 50, keyed by their position ("000",
    /// "001", …) so the keys sort in queue order. Each is an a{sv} with `id`
    /// (s), `name` (s), `state` (s), `received` (t), `size` (t, 0 when
    /// unknown) and `speed` (t, bytes per second).
    fn queue(&self) -> HashMap<String, OwnedVariant> {
        queue_order(self.manager.list()).iter().enumerate().map(|(i, t)| (format!("{i:03}"), entry(t))).collect()
    }

    /// A download completed (`ok`) or failed.
    #[zbus(signal)]
    pub async fn finished(emitter: &SignalEmitter<'_>, id: &str, name: &str, ok: bool) -> zbus::Result<()>;
}
