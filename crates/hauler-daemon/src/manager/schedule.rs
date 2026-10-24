//! Scheduling: starting queued downloads, driving them through the engine,
//! and reporting their progress.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::FutureExt;
use hauler_engine::header::{HeaderMap, HeaderName, HeaderValue};
use hauler_engine::{self as engine, CancellationToken, Outcome, Task};
use hauler_ipc::{Event, ProgressUpdate, TaskId, TaskState};
use tracing::{error, info, warn};

use super::{Inner, Manager, Running, SAVE_EVERY_TICKS, now, stop};

impl Manager {
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
