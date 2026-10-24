//! The queue: which downloads run, when, and with what limits.
//!
//! All task state lives under one mutex that is never held across an await.
//! Running downloads are tokio tasks driven by the engine; they report back
//! through [`Manager::on_prepared`] and [`Manager::on_finished`].

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt;
use hauler_engine::header::{HeaderMap, HeaderName, HeaderValue};
use hauler_engine::{self as engine, CancellationToken, Checksum, Limiter, Outcome, Progress, Task};
use hauler_ipc::{
    AddParams, Event, NetworkState, ProgressUpdate, RpcError, Segment, SegmentMap, Settings, SettingsPatch, Status,
    TaskId, TaskInfo, TaskState,
};
use tokio::sync::{Notify, broadcast, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::store::{Store, TaskSpec};

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

    // -- RPC operations ----------------------------------------------------

    pub fn add(&self, p: AddParams) -> Result<TaskInfo, RpcError> {
        let invalid = |m: String| RpcError::new(RpcError::INVALID_PARAMS, m);
        let url = engine::Url::parse(&p.url).map_err(|e| invalid(format!("bad URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(invalid("only http and https URLs are supported".into()));
        }
        for (name, value) in &p.headers {
            HeaderName::from_bytes(name.as_bytes()).map_err(|e| invalid(format!("bad header name: {e}")))?;
            HeaderValue::from_str(value).map_err(|e| invalid(format!("bad header value: {e}")))?;
        }
        if let Some(c) = &p.checksum {
            c.parse::<Checksum>().map_err(|e| invalid(e.to_string()))?;
        }
        if let Some(c) = p.connections
            && !(1..=32).contains(&c)
        {
            return Err(invalid("connections must be 1 to 32".into()));
        }

        let mut inner = self.inner.lock().unwrap();
        let dir = p.dir.unwrap_or_else(|| inner.settings.download_dir.clone());
        let dir_path = Path::new(&dir);
        if !dir_path.is_absolute() || !dir_path.is_dir() {
            return Err(invalid(format!("{dir} is not an existing absolute directory")));
        }
        let mut info = TaskInfo {
            id: 0,
            url: url.to_string(),
            dir,
            // The requested name shows until the probe settles the real one.
            name: p.filename.as_deref().map(engine::sanitize_filename).filter(|n| !n.is_empty()),
            state: if p.paused { TaskState::Paused } else { TaskState::Queued },
            size: None,
            received: 0,
            speed: 0,
            connections: 0,
            limit: p.limit.unwrap_or(0),
            priority: p.priority.unwrap_or(0),
            resumable: None,
            error: None,
            added_at: now(),
            finished_at: None,
        };
        let spec = TaskSpec {
            filename: info.name.clone(),
            headers: p.headers,
            checksum: p.checksum,
            connections: p.connections,
        };
        info.id = self.store.insert(&info, &spec).map_err(internal)?;
        let limiter = Limiter::new(info.limit);
        inner.tasks.insert(info.id, Entry { info: info.clone(), spec, limiter, run: None });
        drop(inner);
        self.emit(Event::TaskAdded { task: info.clone() });
        self.wake.notify_one();
        Ok(info)
    }

    pub fn list(&self) -> Vec<TaskInfo> {
        self.inner.lock().unwrap().tasks.values().map(|e| e.info.clone()).collect()
    }

    pub fn get(&self, id: TaskId) -> Result<TaskInfo, RpcError> {
        let inner = self.inner.lock().unwrap();
        inner.tasks.get(&id).map(|e| e.info.clone()).ok_or_else(|| RpcError::not_found(id))
    }

    pub fn pause(&self, id: TaskId) -> Result<TaskInfo, RpcError> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.tasks.get_mut(&id).ok_or_else(|| RpcError::not_found(id))?;
        match entry.info.state {
            TaskState::Running => stop(entry, TaskState::Paused),
            TaskState::Queued | TaskState::Waiting => entry.info.state = TaskState::Paused,
            TaskState::Paused => {}
            s => return Err(RpcError::new(RpcError::INVALID_STATE, format!("cannot pause a {} download", s.as_str()))),
        }
        let info = entry.info.clone();
        self.changed(&inner, id);
        Ok(info)
    }

    pub fn resume(&self, id: TaskId) -> Result<TaskInfo, RpcError> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.tasks.get_mut(&id).ok_or_else(|| RpcError::not_found(id))?;
        match entry.info.state {
            TaskState::Paused | TaskState::Failed => {
                entry.info.state = TaskState::Queued;
                entry.info.error = None;
                entry.info.finished_at = None;
                // Still winding down from a pause: requeue instead of pausing.
                if let Some(run) = entry.run.as_mut() {
                    run.after_stop = TaskState::Queued;
                }
            }
            TaskState::Completed => {
                return Err(RpcError::new(RpcError::INVALID_STATE, "download already completed"));
            }
            _ => {}
        }
        let info = entry.info.clone();
        self.changed(&inner, id);
        drop(inner);
        self.wake.notify_one();
        Ok(info)
    }

    pub async fn remove(&self, id: TaskId, delete_files: bool) -> Result<(), RpcError> {
        let (entry, handle) = {
            let mut inner = self.inner.lock().unwrap();
            let mut entry = inner.tasks.remove(&id).ok_or_else(|| RpcError::not_found(id))?;
            let handle = entry.run.as_mut().and_then(|r| {
                r.cancel.cancel();
                r.handle.take()
            });
            self.active.send_replace(inner.active());
            (entry, handle)
        };
        // Let the engine flush and close the file before touching it.
        if let Some(h) = handle {
            let _ = h.await;
        }
        // Only ever a single plain name inside the task's folder.
        if delete_files && let Some(name) = entry.spec.filename.as_deref().filter(|n| is_plain_name(n)) {
            let target = Path::new(&entry.info.dir).join(name);
            for suffix in ["", ".part", ".hauler"] {
                let mut p = target.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(p);
            }
        }
        self.store.delete(id).map_err(internal)?;
        self.emit(Event::TaskRemoved { id });
        self.wake.notify_one();
        Ok(())
    }

    pub fn set_limit(&self, id: TaskId, bytes_per_sec: u64) -> Result<TaskInfo, RpcError> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.tasks.get_mut(&id).ok_or_else(|| RpcError::not_found(id))?;
        entry.limiter.set_rate(bytes_per_sec);
        entry.info.limit = bytes_per_sec;
        let info = entry.info.clone();
        self.changed(&inner, id);
        Ok(info)
    }

    pub fn set_priority(&self, id: TaskId, priority: i32) -> Result<TaskInfo, RpcError> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.tasks.get_mut(&id).ok_or_else(|| RpcError::not_found(id))?;
        entry.info.priority = priority;
        let info = entry.info.clone();
        self.changed(&inner, id);
        drop(inner);
        self.wake.notify_one();
        Ok(info)
    }

    pub fn pause_all(&self) -> u32 {
        let ids: Vec<_> = {
            let inner = self.inner.lock().unwrap();
            let active = [TaskState::Running, TaskState::Queued, TaskState::Waiting];
            inner.tasks.values().filter(|e| active.contains(&e.info.state)).map(|e| e.info.id).collect()
        };
        ids.into_iter().filter(|&id| self.pause(id).is_ok()).count() as u32
    }

    pub fn resume_all(&self) -> u32 {
        let ids: Vec<_> = {
            let inner = self.inner.lock().unwrap();
            let stopped = [TaskState::Paused, TaskState::Failed];
            inner.tasks.values().filter(|e| stopped.contains(&e.info.state)).map(|e| e.info.id).collect()
        };
        ids.into_iter().filter(|&id| self.resume(id).is_ok()).count() as u32
    }

    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, patch: SettingsPatch) -> Result<Settings, RpcError> {
        let invalid = |m: &str| RpcError::new(RpcError::INVALID_PARAMS, m);
        let mut inner = self.inner.lock().unwrap();
        let mut s = inner.settings.clone();
        if let Some(v) = patch.max_active {
            if v == 0 {
                return Err(invalid("max_active must be at least 1"));
            }
            s.max_active = v;
        }
        if let Some(v) = patch.connections {
            if !(1..=32).contains(&v) {
                return Err(invalid("connections must be 1 to 32"));
            }
            s.connections = v;
        }
        if let Some(v) = patch.download_dir {
            if !Path::new(&v).is_absolute() || !Path::new(&v).is_dir() {
                return Err(invalid("download_dir must be an existing absolute directory"));
            }
            s.download_dir = v;
        }
        if let Some(v) = patch.global_limit {
            s.global_limit = v;
        }
        if let Some(v) = patch.pause_on_metered {
            s.pause_on_metered = v;
        }
        self.store.save_settings(&s).map_err(internal)?;
        self.global.set_rate(s.global_limit);
        let network_before = inner.network();
        inner.settings = s.clone();
        let network = inner.network();
        drop(inner);
        self.emit(Event::Settings { settings: s.clone() });
        if network != network_before {
            self.emit(Event::Network { network });
        }
        self.wake.notify_one();
        Ok(s)
    }

    pub fn status(&self) -> Status {
        let inner = self.inner.lock().unwrap();
        let count = |states: &[TaskState]| inner.tasks.values().filter(|e| states.contains(&e.info.state)).count();
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            running: count(&[TaskState::Running]) as u32,
            queued: count(&[TaskState::Queued, TaskState::Waiting]) as u32,
            speed: inner.tasks.values().filter(|e| e.run.is_some()).map(|e| e.info.speed).sum(),
            network: inner.network(),
        }
    }

    /// The segment map: live from the engine while running, from the resume
    /// file while paused, one full segment once complete.
    pub fn segments(&self, id: TaskId) -> Result<SegmentMap, RpcError> {
        let (info, name, live) = {
            let inner = self.inner.lock().unwrap();
            let e = inner.tasks.get(&id).ok_or_else(|| RpcError::not_found(id))?;
            let live = e.run.as_ref().and_then(|r| r.progress.as_ref()).map(|p| p.segments());
            (e.info.clone(), e.spec.filename.clone(), live)
        };
        let size = info.size;
        let whole = |done: u64| match size {
            Some(s) if s > 0 => vec![Segment { start: 0, end: s, done: done.min(s), active: false }],
            _ => Vec::new(),
        };
        let segments = match live {
            Some(v) if !v.is_empty() => {
                v.into_iter().map(|s| Segment { start: s.start, end: s.end, done: s.done, active: s.active }).collect()
            }
            _ if info.state == TaskState::Completed => whole(size.unwrap_or(0)),
            _ => {
                let saved = name
                    .filter(|n| is_plain_name(n))
                    .map(|n| Path::new(&info.dir).join(format!("{n}.hauler")))
                    .and_then(|p| engine::ResumeState::load(&p).ok().flatten());
                match saved {
                    Some(state) => {
                        let mut v: Vec<_> = state
                            .segments
                            .iter()
                            .map(|s| Segment { start: s.start, end: s.end, done: s.written, active: false })
                            .collect();
                        v.sort_by_key(|s| s.start);
                        v
                    }
                    None => whole(info.received),
                }
            }
        };
        Ok(SegmentMap { size, segments })
    }

    /// Report connectivity from the system's network monitor.
    pub fn set_network(&self, online: bool, metered: bool) {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.network();
        inner.online = online;
        inner.metered = metered;
        let after = inner.network();
        drop(inner);
        if before != after {
            info!(
                "network: online={online} metered={metered}; downloads {}",
                if after.open { "allowed" } else { "held" }
            );
            self.emit(Event::Network { network: after });
            self.wake.notify_one();
        }
    }

    // -- Scheduling --------------------------------------------------------

    /// Start and stop downloads until `shutdown`. Ticks once a second only
    /// while something runs, so an idle daemon uses no CPU.
    pub async fn run(self: Arc<Self>, shutdown: CancellationToken) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            self.schedule();
            let busy = self.inner.lock().unwrap().active() > 0;
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = self.wake.notified() => {}
                _ = tick.tick(), if busy => self.tick(),
            }
        }
    }

    fn schedule(self: &Arc<Self>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.shutting_down {
            return;
        }
        if !inner.network().open {
            let running: Vec<_> =
                inner.tasks.values().filter(|e| e.info.state == TaskState::Running).map(|e| e.info.id).collect();
            for id in running {
                stop(inner.tasks.get_mut(&id).unwrap(), TaskState::Waiting);
                self.changed(&inner, id);
            }
            return;
        }

        let slots = inner.settings.max_active.saturating_sub(inner.active()) as usize;
        if slots == 0 {
            return;
        }
        let mut ready: Vec<_> = inner
            .tasks
            .values()
            .filter(|e| e.run.is_none() && matches!(e.info.state, TaskState::Queued | TaskState::Waiting))
            .map(|e| (e.info.state != TaskState::Waiting, -e.info.priority, e.info.id))
            .collect();
        ready.sort();
        for (_, _, id) in ready.into_iter().take(slots) {
            self.start(&mut inner, id);
        }
    }

    fn start(self: &Arc<Self>, inner: &mut Inner, id: TaskId) {
        let default_connections = inner.settings.connections;
        let entry = inner.tasks.get_mut(&id).unwrap();
        let cancel = CancellationToken::new();
        entry.info.state = TaskState::Running;
        entry.info.error = None;

        let mut headers = HeaderMap::new();
        for (name, value) in &entry.spec.headers {
            if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
                headers.append(n, v);
            }
        }
        let opts = engine::Options {
            connections: entry.spec.connections.unwrap_or(default_connections).into(),
            headers,
            filename: entry.spec.filename.clone(),
            checksum: entry.spec.checksum.as_deref().and_then(|c| c.parse().ok()),
            limiter: vec![entry.limiter.clone(), self.global.clone()],
            ..engine::Options::default()
        };
        let handle = tokio::spawn(self.clone().drive(
            id,
            entry.info.url.clone(),
            PathBuf::from(&entry.info.dir),
            opts,
            cancel.clone(),
        ));
        entry.run = Some(Running {
            cancel,
            progress: None,
            after_stop: TaskState::Queued,
            handle: Some(handle),
            last_received: entry.info.received,
        });
        self.changed(inner, id);
    }

    async fn drive(
        self: Arc<Self>,
        id: TaskId,
        url: String,
        dir: PathBuf,
        opts: engine::Options,
        cancel: CancellationToken,
    ) {
        // A panic would end this task silently and leave the download marked
        // running forever; report it as a failure instead.
        let attempt = self.clone().attempt(id, url, dir, opts, cancel);
        let result = AssertUnwindSafe(attempt).catch_unwind().await.unwrap_or_else(|panic| {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            error!("download {id} crashed: {msg}");
            Err(std::io::Error::other(format!("internal error: {msg}")).into())
        });
        self.on_finished(id, result);
    }

    async fn attempt(
        self: Arc<Self>,
        id: TaskId,
        url: String,
        dir: PathBuf,
        mut opts: engine::Options,
        cancel: CancellationToken,
    ) -> engine::Result<Outcome> {
        loop {
            let prepared = tokio::select! {
                _ = cancel.cancelled() => break Ok(Outcome::Paused),
                r = engine::prepare(self.client.clone(), &url, &dir, opts.clone()) => r,
            };
            let task = match prepared {
                Ok(t) => t,
                Err(e) => break Err(e),
            };
            self.on_prepared(id, &task);
            match task.run(cancel.clone()).await {
                Err(engine::Error::Changed) if !opts.restart => {
                    info!("download {id}: file changed on the server; starting over");
                    opts.restart = true;
                }
                r => break r,
            }
        }
    }

    fn on_prepared(&self, id: TaskId, task: &Task) {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.tasks.get_mut(&id) else { return };
        let name = task.target().file_name().map(|n| n.to_string_lossy().into_owned());
        entry.info.name = name.clone();
        entry.spec.filename = name;
        entry.info.size = task.size();
        entry.info.resumable = Some(task.resumable());
        entry.info.received = task.resumed_bytes();
        if let Some(run) = entry.run.as_mut() {
            run.progress = Some(task.progress());
            run.last_received = entry.info.received;
        }
        self.changed(&inner, id);
    }

    fn on_finished(&self, id: TaskId, result: engine::Result<Outcome>) {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.tasks.get_mut(&id) else { return };
        let Some(run) = entry.run.take() else { return };
        if let Some(p) = &run.progress {
            entry.info.received = p.received.load(Ordering::Relaxed);
        }
        entry.info.speed = 0;
        entry.info.connections = 0;
        match result {
            Ok(Outcome::Completed(path)) => {
                entry.info.state = TaskState::Completed;
                entry.info.finished_at = Some(now());
                entry.info.name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                if let Some(size) = entry.info.size {
                    entry.info.received = size;
                }
            }
            Ok(Outcome::Paused) => entry.info.state = run.after_stop,
            Err(_) if run.cancel.is_cancelled() => entry.info.state = run.after_stop,
            Err(e) => {
                warn!("download {id} failed: {e}");
                entry.info.state = TaskState::Failed;
                entry.info.error = Some(e.to_string());
                entry.info.finished_at = Some(now());
            }
        }
        self.changed(&inner, id);
        drop(inner);
        self.wake.notify_one();
    }

    fn tick(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.ticks += 1;
        let save = inner.ticks.is_multiple_of(SAVE_EVERY_TICKS);
        let mut updates = Vec::new();
        for entry in inner.tasks.values_mut() {
            let Some(run) = entry.run.as_mut() else { continue };
            let Some(p) = &run.progress else { continue };
            let received = p.received.load(Ordering::Relaxed);
            let delta = received.saturating_sub(run.last_received);
            run.last_received = received;
            entry.info.speed = (entry.info.speed + delta) / 2;
            entry.info.received = received;
            entry.info.connections = p.connections.load(Ordering::Relaxed) as u32;
            updates.push(ProgressUpdate {
                id: entry.info.id,
                received,
                speed: entry.info.speed,
                connections: entry.info.connections,
            });
        }
        drop(inner);
        if updates.is_empty() {
            return;
        }
        if save {
            let rows: Vec<_> = updates.iter().map(|u| (u.id, u.received)).collect();
            if let Err(e) = self.store.update_received(&rows) {
                warn!("could not save progress: {e}");
            }
        }
        self.emit(Event::Progress { updates });
    }

    /// Stop every download, keeping them queued for the next start.
    pub async fn shutdown(&self) {
        let handles: Vec<_> = {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            inner
                .tasks
                .values_mut()
                .filter_map(|e| {
                    let run = e.run.as_mut()?;
                    run.after_stop = TaskState::Queued;
                    run.cancel.cancel();
                    run.handle.take()
                })
                .collect()
        };
        info!("stopping {} downloads", handles.len());
        let all = futures_util::future::join_all(handles);
        if tokio::time::timeout(Duration::from_secs(10), all).await.is_err() {
            warn!("some downloads did not stop within 10 s");
        }
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
