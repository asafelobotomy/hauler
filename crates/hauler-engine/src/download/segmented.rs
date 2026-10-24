//! Segmented mode: workers fetching ranges in parallel, a monitor that
//! restarts slow connections, and a saver that checkpoints resume state.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::files::{Paths, blocking, finish, preallocate};
use super::retry::backoff;
use super::{Options, Outcome, Progress};
use crate::error::{Error, Result};
use crate::plan::Plan;
use crate::probe::Probe;
use crate::state::ResumeState;

mod fetch;

use fetch::fetch;

#[derive(Debug)]
struct WorkerStat {
    /// Bytes received over this worker's lifetime.
    bytes: AtomicU64,
    /// Bumped on every new connection, so the monitor can reset its average.
    conn: AtomicU64,
    streaming: AtomicBool,
    restart: AtomicBool,
    since: Mutex<Instant>,
    segment: AtomicUsize,
}

impl Default for WorkerStat {
    fn default() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            conn: AtomicU64::new(0),
            streaming: AtomicBool::new(false),
            restart: AtomicBool::new(false),
            since: Mutex::new(Instant::now()),
            segment: AtomicUsize::new(usize::MAX),
        }
    }
}

struct Shared {
    client: Client,
    opts: Options,
    url: String,
    size: u64,
    validator: Option<String>,
    plan: Mutex<Plan>,
    file: Arc<File>,
    progress: Arc<Progress>,
    workers: Vec<WorkerStat>,
    stop: CancellationToken,
}

enum FetchEnd {
    /// The segment is fully received.
    Done,
    /// The monitor asked for a fresh connection.
    Restart,
    Stopped,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_segmented(
    client: Client,
    opts: Options,
    url: String,
    probe: Probe,
    paths: Paths,
    file: File,
    plan: Plan,
    fresh: bool,
    progress: Arc<Progress>,
    cancel: CancellationToken,
) -> Result<Outcome> {
    let size = probe.size.expect("segmented mode needs a size");
    if fresh {
        file.set_len(0)?;
        preallocate(&file, size)?;
    }
    let mut state =
        ResumeState::new(url, probe.final_url.clone(), size, probe.etag.clone(), probe.last_modified.clone());
    state.segments = plan.snapshot();
    state.save(&paths.state)?;

    let n = opts.connections.max(1);
    let shared = Arc::new(Shared {
        client,
        url: probe.final_url.clone(),
        size,
        validator: probe.validator().map(str::to_owned),
        plan: Mutex::new(plan),
        file: Arc::new(file),
        progress,
        workers: (0..n).map(|_| WorkerStat::default()).collect(),
        stop: cancel.child_token(),
        opts,
    });

    let mut workers = JoinSet::new();
    for id in 0..n {
        workers.spawn(worker(id, shared.clone()));
    }
    let background = CancellationToken::new();
    let monitor = tokio::spawn(monitor(shared.clone(), background.clone()));
    let saver = tokio::spawn(saver(shared.clone(), state.clone(), paths.state.clone(), background.clone()));

    let mut first_err = None;
    while let Some(res) = workers.join_next().await {
        let res = res.unwrap_or_else(|e| Err(std::io::Error::other(e).into()));
        if let Err(e) = res
            && first_err.is_none()
        {
            shared.stop.cancel();
            first_err = Some(e);
        }
    }
    background.cancel();
    let _ = monitor.await;
    let _ = saver.await;

    shared.progress.set_segments(&shared.plan.lock().unwrap());
    let complete = shared.plan.lock().unwrap().is_complete();
    if !complete || first_err.is_some() {
        checkpoint(&shared, &mut state, &paths.state).await?;
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    if !complete {
        return if cancel.is_cancelled() { Ok(Outcome::Paused) } else { Err(Error::Incomplete) };
    }
    let file = shared.file.clone();
    blocking(move || file.sync_all()).await?;
    finish(&shared.opts, &paths).await
}

/// Make written data durable, then record it. In that order, a crash can lose
/// progress but never mark bytes as done that are not on disk.
async fn checkpoint(shared: &Shared, state: &mut ResumeState, path: &Path) -> Result<()> {
    state.segments = shared.plan.lock().unwrap().snapshot();
    let file = shared.file.clone();
    blocking(move || file.sync_data()).await?;
    let (state_copy, path) = (state.clone(), path.to_owned());
    tokio::task::spawn_blocking(move || state_copy.save(&path)).await.map_err(std::io::Error::other)?
}

async fn saver(shared: Arc<Shared>, mut state: ResumeState, path: PathBuf, stop: CancellationToken) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(shared.opts.save_interval) => {}
        }
        if let Err(e) = checkpoint(&shared, &mut state, &path).await {
            warn!("could not save resume state: {e}");
        }
    }
}

/// Restart connections that run far below the mean. A new connection often
/// lands on a faster path through the CDN or load balancer.
async fn monitor(shared: Arc<Shared>, stop: CancellationToken) {
    let n = shared.workers.len();
    let mut last_bytes = vec![0u64; n];
    let mut last_conn = vec![u64::MAX; n];
    let mut speed = vec![0f64; n];
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tick.tick() => {}
        }
        shared.progress.set_segments(&shared.plan.lock().unwrap());
        let mut judged = Vec::new();
        for (i, w) in shared.workers.iter().enumerate() {
            let bytes = w.bytes.load(Ordering::Relaxed);
            let delta = bytes.saturating_sub(last_bytes[i]) as f64;
            last_bytes[i] = bytes;
            let conn = w.conn.load(Ordering::Relaxed);
            if conn != last_conn[i] {
                last_conn[i] = conn;
                speed[i] = delta;
            } else {
                speed[i] = 0.7 * speed[i] + 0.3 * delta;
            }
            let old_enough = w.since.lock().unwrap().elapsed() >= shared.opts.slow_grace;
            if w.streaming.load(Ordering::Relaxed) && old_enough {
                judged.push(i);
            }
        }
        if judged.len() < 2 {
            continue;
        }
        let mean = judged.iter().map(|&i| speed[i]).sum::<f64>() / judged.len() as f64;
        if mean <= 0.0 {
            continue;
        }
        let plan = shared.plan.lock().unwrap();
        for &i in &judged {
            let seg = shared.workers[i].segment.load(Ordering::Relaxed);
            // Not worth a new handshake if the mean speed would finish it in 2 s.
            let worth_it = seg != usize::MAX && plan.segment(seg).remaining() as f64 > 2.0 * mean;
            if speed[i] < shared.opts.slow_ratio * mean && worth_it {
                debug!("worker {i} at {:.0} B/s vs mean {:.0} B/s; restarting", speed[i], mean);
                shared.workers[i].restart.store(true, Ordering::Relaxed);
                shared.progress.restarts.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

async fn worker(id: usize, shared: Arc<Shared>) -> Result<()> {
    let stat = &shared.workers[id];
    loop {
        if shared.stop.is_cancelled() {
            return Ok(());
        }
        let seg = {
            let mut plan = shared.plan.lock().unwrap();
            let seg = plan.claim(id);
            shared.progress.splits.store(plan.splits(), Ordering::Relaxed);
            seg
        };
        let Some(seg) = seg else { return Ok(()) };
        stat.segment.store(seg, Ordering::Relaxed);

        let mut failures = 0;
        loop {
            let before = shared.plan.lock().unwrap().segment(seg).received;
            match fetch(id, seg, &shared).await {
                Ok(FetchEnd::Done) => break,
                Ok(FetchEnd::Restart) => continue,
                Ok(FetchEnd::Stopped) => {
                    shared.plan.lock().unwrap().release(seg);
                    return Ok(());
                }
                Err(e) => {
                    let after = shared.plan.lock().unwrap().segment(seg).received;
                    if after > before {
                        failures = 0;
                    }
                    // A server that caps connections per client turns away
                    // the extra ones. While others still stream, leave the
                    // segment to them instead of failing the download.
                    if e.is_turned_away() && shared.progress.connections.load(Ordering::Relaxed) > 0 {
                        info!("worker {id}: {e}; carrying on with fewer connections");
                        shared.plan.lock().unwrap().release(seg);
                        stat.segment.store(usize::MAX, Ordering::Relaxed);
                        return Ok(());
                    }
                    if e.is_retryable() && failures < shared.opts.max_retries {
                        failures += 1;
                        shared.progress.retries.fetch_add(1, Ordering::Relaxed);
                        debug!("worker {id} segment {seg}: {e}; retry {failures}");
                        tokio::select! {
                            _ = shared.stop.cancelled() => {}
                            _ = tokio::time::sleep(backoff(failures, e.retry_after())) => {}
                        }
                        continue;
                    }
                    shared.plan.lock().unwrap().release(seg);
                    return Err(if e.is_retryable() {
                        Error::RetriesExhausted { attempts: failures + 1, last: Box::new(e) }
                    } else {
                        e
                    });
                }
            }
        }
        let mut plan = shared.plan.lock().unwrap();
        plan.segment_mut(seg).owner = None;
        stat.segment.store(usize::MAX, Ordering::Relaxed);
    }
}
