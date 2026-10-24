//! The queue: which downloads run, when, and with what limits.
//!
//! All task state lives under one mutex that is never held across an await.
//! Running downloads are tokio tasks driven by the engine; they report back
//! through [`Manager::on_prepared`] and [`Manager::on_finished`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hauler_engine::{self as engine, CancellationToken, Limiter, Progress};
use hauler_ipc::{Event, NetworkState, RpcError, Settings, TaskId, TaskInfo, TaskState};
use tokio::sync::{Notify, broadcast, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::store::{Store, TaskSpec};

mod ops;
mod schedule;

/// Progress is written to the database every this many ticks.
const SAVE_EVERY_TICKS: u64 = 5;

struct Entry {
    info: TaskInfo,
    spec: TaskSpec,
    limiter: Arc<Limiter>,
    run: Option<Running>,
}

struct Running {
    cancel: CancellationToken,
    /// Set once the engine has probed the URL.
    progress: Option<Arc<Progress>>,
    /// State to take when the engine stops on our request.
    after_stop: TaskState,
    handle: Option<JoinHandle<()>>,
    last_received: u64,
}

struct Inner {
    tasks: BTreeMap<TaskId, Entry>,
    settings: Settings,
    online: bool,
    metered: bool,
    shutting_down: bool,
    ticks: u64,
}

impl Inner {
    fn network(&self) -> NetworkState {
        let open = self.online && !(self.metered && self.settings.pause_on_metered);
        NetworkState { online: self.online, metered: self.metered, open }
    }

    fn active(&self) -> u32 {
        self.tasks.values().filter(|e| e.run.is_some()).count() as u32
    }
}

pub struct Manager {
    inner: Mutex<Inner>,
    store: Store,
    client: engine::Client,
    global: Arc<Limiter>,
    events: broadcast::Sender<Event>,
    active: watch::Sender<u32>,
    wake: Notify,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`, else `~/Downloads`.
pub fn default_download_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into());
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("XDG_DOWNLOAD_DIR=") {
                let v = v.trim().trim_matches('"');
                let path = match v.strip_prefix("$HOME") {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(v),
                };
                if path.is_absolute() {
                    return path;
                }
            }
        }
    }
    home.join("Downloads")
}

impl Manager {
    pub fn new(store: Store, default_dir: &Path) -> anyhow::Result<Arc<Self>> {
        let settings = store.load_settings()?.unwrap_or_else(|| Settings {
            max_active: 3,
            connections: 8,
            global_limit: 0,
            download_dir: default_dir.to_string_lossy().into_owned(),
            pause_on_metered: true,
        });

        let mut tasks = BTreeMap::new();
        for (mut info, spec) in store.load_all()? {
            // Whatever was running when the daemon stopped goes back in the queue.
            if matches!(info.state, TaskState::Running | TaskState::Waiting) {
                info.state = TaskState::Queued;
                store.update(&info, &spec)?;
            }
            let limiter = Limiter::new(info.limit);
            tasks.insert(info.id, Entry { info, spec, limiter, run: None });
        }
        info!("loaded {} downloads", tasks.len());

        let (events, _) = broadcast::channel(1024);
        let (active, _) = watch::channel(0);
        Ok(Arc::new(Self {
            global: Limiter::new(settings.global_limit),
            inner: Mutex::new(Inner { tasks, settings, online: true, metered: false, shutting_down: false, ticks: 0 }),
            store,
            client: engine::client()?,
            events,
            active,
            wake: Notify::new(),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Number of running downloads, for sleep inhibition.
    pub fn active_watch(&self) -> watch::Receiver<u32> {
        self.active.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn save(&self, entry: &Entry) {
        if let Err(e) = self.store.update(&entry.info, &entry.spec) {
            warn!("could not save download {}: {e}", entry.info.id);
        }
    }

    /// Persist, broadcast and refresh derived state after changing an entry.
    fn changed(&self, inner: &Inner, id: TaskId) {
        if let Some(entry) = inner.tasks.get(&id) {
            self.save(entry);
            self.emit(Event::TaskUpdated { task: entry.info.clone() });
        }
        self.active.send_replace(inner.active());
    }
}

/// Ask a running download to stop; it takes `state` once the engine returns.
fn stop(entry: &mut Entry, state: TaskState) {
    if let Some(run) = entry.run.as_mut() {
        run.after_stop = state;
        run.cancel.cancel();
    }
    entry.info.state = state;
}

/// A file name with no directory parts, so joining it to a folder stays there.
fn is_plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(_)), None))
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(RpcError::INTERNAL, e.to_string())
}
