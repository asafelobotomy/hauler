//! The RPC operations: adding, pausing, resuming and removing downloads,
//! limits, priorities and settings.

use std::path::Path;

use hauler_engine::header::{HeaderName, HeaderValue};
use hauler_engine::{self as engine, Checksum, Limiter};
use hauler_ipc::{
    AddParams, Event, RpcError, Segment, SegmentMap, Settings, SettingsPatch, Status, TaskId, TaskInfo, TaskState,
};
use tracing::info;

use super::{Entry, Manager, internal, is_plain_name, now, stop};
use crate::store::TaskSpec;

impl Manager {
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
}
