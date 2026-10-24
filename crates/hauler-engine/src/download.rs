//! The download driver: probe, plan, parallel ranged fetches, resume.

use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, StatusCode, Url};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::checksum::Checksum;
use crate::error::{Error, Result};
use crate::limit::Limiter;
use crate::plan::{Plan, SegmentView};
use crate::probe::{self, Probe, sanitize_filename};
use crate::state::ResumeState;

#[derive(Clone, Debug)]
pub struct Options {
    /// Parallel connections per download.
    pub connections: usize,
    /// Never split a segment into pieces smaller than this.
    pub min_split: u64,
    /// Bytes buffered per connection before a positional write.
    pub buffer: usize,
    /// How often resume state is checkpointed.
    pub save_interval: Duration,
    /// Restart a connection slower than this fraction of the mean.
    pub slow_ratio: f64,
    /// Let a new connection run this long before judging its speed.
    pub slow_grace: Duration,
    /// Consecutive failures allowed per segment before giving up.
    pub max_retries: u32,
    /// Give up on a connection that sends nothing for this long.
    pub stall_timeout: Duration,
    /// Extra request headers (cookies, referer, auth).
    pub headers: HeaderMap,
    /// Save under this name instead of the server's.
    pub filename: Option<String>,
    pub checksum: Option<Checksum>,
    /// Ignore existing resume state and start over.
    pub restart: bool,
    /// Speed limits to obey, e.g. this download's own and a global one.
    pub limiter: Vec<Arc<Limiter>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            connections: 8,
            min_split: 1024 * 1024,
            buffer: 1024 * 1024,
            save_interval: Duration::from_secs(2),
            slow_ratio: 0.3,
            slow_grace: Duration::from_secs(5),
            max_retries: 8,
            stall_timeout: Duration::from_secs(30),
            headers: HeaderMap::new(),
            filename: None,
            checksum: None,
            restart: false,
            limiter: Vec::new(),
        }
    }
}

/// An HTTP client suited to segmented downloads. HTTP/2 is deliberately not
/// enabled: it would multiplex every segment onto one TCP connection and lose
/// the point of parallel connections.
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("hauler/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .build()?)
}

/// Live counters for a running download.
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes received, including bytes resumed from disk.
    pub received: AtomicU64,
    /// Connections currently streaming.
    pub connections: AtomicUsize,
    /// Segments split to keep idle connections busy.
    pub splits: AtomicU64,
    /// Slow connections restarted.
    pub restarts: AtomicU64,
    /// Failed requests retried.
    pub retries: AtomicU64,
    /// Segment map, refreshed about once a second. Empty in stream mode.
    segments: Mutex<Vec<SegmentView>>,
}

impl Progress {
    pub fn segments(&self) -> Vec<SegmentView> {
        self.segments.lock().unwrap().clone()
    }

    fn set_segments(&self, plan: &Plan) {
        *self.segments.lock().unwrap() = plan.view();
    }
}

#[derive(Debug)]
pub enum Outcome {
    Completed(PathBuf),
    /// Stopped by the caller; run the same download again to resume.
    Paused,
}

enum Mode {
    Segmented {
        plan: Plan,
        fresh: bool,
    },
    /// No range support or unknown size: one connection, not resumable.
    Stream,
}

pub struct Task {
    client: Client,
    opts: Options,
    url: String,
    probe: Probe,
    target: PathBuf,
    part: PathBuf,
    state_path: PathBuf,
    /// The `.part` file, exclusively locked for as long as the task lives.
    file: File,
    mode: Mode,
    progress: Arc<Progress>,
}

/// Probe `url` and decide where and how to download it, resuming earlier
/// progress when the remote file is unchanged.
pub async fn prepare(client: Client, url: &str, dir: &Path, opts: Options) -> Result<Task> {
    let probe = retry(&opts, || probe::probe(&client, url, &opts.headers)).await?;
    let mut opts = opts;
    opts.headers = headers_for(url, &probe.final_url, &opts.headers);
    let name = opts
        .filename
        .as_deref()
        .map(sanitize_filename)
        .filter(|n| !n.is_empty())
        .or_else(|| probe.filename.clone())
        .unwrap_or_else(|| "download".into());

    let mut target = dir.join(&name);
    let mut resume = !opts.restart && state_path(&target).exists();
    if !resume && (target.exists() || (!opts.restart && part_path(&target).exists())) {
        target = free_name(dir, &name);
    }
    // Never share a part file: if another download (another task, or a
    // `hauler get` next to the daemon) holds it, take the next free name.
    let file = loop {
        if let Some(f) = lock_part(&part_path(&target))? {
            break f;
        }
        info!("{} is in use by another download", part_path(&target).display());
        resume = false;
        target = free_name(dir, &name);
    };
    let part = part_path(&target);
    let state_path = state_path(&target);

    let saved = if resume {
        ResumeState::load(&state_path).unwrap_or_else(|e| {
            warn!("ignoring resume state: {e}");
            None
        })
    } else {
        None
    };
    // Progress on disk, but the server now answers from another URL with a
    // different file: more likely a "too many downloads" page than a new
    // version. Wait rather than throw the progress away.
    if let Some(saved) = &saved
        && !saved.segments.is_empty()
        && probe.final_url != saved.final_url
        && !saved.matches(probe.size.unwrap_or(0), probe.etag.as_deref(), probe.last_modified.as_deref())
    {
        return Err(Error::Refused(probe.final_url));
    }

    let mode = match probe.size {
        Some(size) if probe.ranges && size > 0 => {
            let mut plan = None;
            if let Some(saved) = saved {
                let part_ok = fs::metadata(&part).map(|m| m.len() == size).unwrap_or(false);
                if saved.matches(size, probe.etag.as_deref(), probe.last_modified.as_deref()) && part_ok {
                    plan = Some(Plan::from_state(&saved.segments, opts.min_split));
                } else {
                    info!("remote file changed or partial data missing; starting over");
                }
            }
            let fresh = plan.is_none();
            let plan = plan.unwrap_or_else(|| Plan::new(size, opts.connections, opts.min_split));
            Mode::Segmented { plan, fresh }
        }
        _ => Mode::Stream,
    };

    let progress = Arc::new(Progress::default());
    if let Mode::Segmented { plan, .. } = &mode {
        progress.received.store(plan.written(), Ordering::Relaxed);
    }
    Ok(Task { client, opts, url: url.to_owned(), probe, target, part, state_path, file, mode, progress })
}

impl Task {
    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn size(&self) -> Option<u64> {
        self.probe.size
    }

    pub fn resumable(&self) -> bool {
        matches!(self.mode, Mode::Segmented { .. })
    }

    /// Bytes already on disk from an earlier run.
    pub fn resumed_bytes(&self) -> u64 {
        match &self.mode {
            Mode::Segmented { plan, fresh: false } => plan.written(),
            _ => 0,
        }
    }

    pub fn progress(&self) -> Arc<Progress> {
        self.progress.clone()
    }

    /// Download until complete, failed, or `cancel` fires (a pause).
    pub async fn run(self, cancel: CancellationToken) -> Result<Outcome> {
        let Task { client, opts, url, probe, target, part, state_path, file, mode, progress } = self;
        let paths = Paths { target, part, state: state_path };
        match mode {
            Mode::Segmented { plan, fresh } => {
                run_segmented(client, opts, url, probe, paths, file, plan, fresh, progress, cancel).await
            }
            Mode::Stream => run_stream(client, opts, url, probe, paths, file, progress, cancel).await,
        }
    }
}

struct Paths {
    target: PathBuf,
    part: PathBuf,
    state: PathBuf,
}

fn part_path(target: &Path) -> PathBuf {
    with_suffix(target, ".part")
}

fn state_path(target: &Path) -> PathBuf {
    with_suffix(target, ".hauler")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Headers to send to `final_url`. Credentials go only to the origin they
/// were given for: after a redirect to another origin they are dropped, as
/// browsers do, so a CDN or a hostile redirect never sees the user's cookies.
fn headers_for(original: &str, final_url: &str, headers: &HeaderMap) -> HeaderMap {
    let same_origin = match (Url::parse(original), Url::parse(final_url)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    };
    let mut h = headers.clone();
    if !same_origin {
        for name in [header::COOKIE, header::AUTHORIZATION, header::PROXY_AUTHORIZATION] {
            h.remove(name);
        }
    }
    h
}

/// Open the part file and take an exclusive advisory lock on it. `None`
/// when another process or task already holds it.
fn lock_part(path: &Path) -> Result<Option<File>> {
    let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    match rustix::fs::flock(&f, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(f)),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
        // Some network filesystems cannot lock; carry on without it.
        Err(e) => {
            warn!("cannot lock {}: {e}", path.display());
            Ok(Some(f))
        }
    }
}

/// `name.ext` -> `name (1).ext`, the first one with no file, part or state.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists() && !part_path(p).exists() && !state_path(p).exists())
        .unwrap()
}

fn preallocate(file: &File, size: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    if rustix::fs::fallocate(file, rustix::fs::FallocateFlags::empty(), 0, size).is_ok() {
        return Ok(());
    }
    // Filesystems without fallocate (FAT, some network mounts) get a sparse file.
    file.set_len(size)
}

/// The longest a `Retry-After` from the server can make us wait.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);

fn backoff(attempt: u32, hint: Option<Duration>) -> Duration {
    hint.map(|h| h.min(MAX_RETRY_AFTER)).unwrap_or_else(|| {
        let base = Duration::from_millis(500) * 2u32.saturating_pow(attempt.saturating_sub(1).min(6));
        base.min(Duration::from_secs(30)).mul_f64(0.5 + fastrand::f64())
    })
}

async fn retry<T, F, Fut>(opts: &Options, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if e.is_retryable() && attempt < opts.max_retries => {
                attempt += 1;
                debug!("attempt {attempt} failed: {e}");
                tokio::time::sleep(backoff(attempt, e.retry_after())).await;
            }
            Err(e) if e.is_retryable() => {
                return Err(Error::RetriesExhausted { attempts: attempt + 1, last: Box::new(e) });
            }
            Err(e) => return Err(e),
        }
    }
}

async fn throttle(limiters: &[Arc<Limiter>], n: u64) {
    for l in limiters {
        l.acquire(n).await;
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(std::io::Error::other)?.map_err(Error::from)
}

async fn finish(opts: &Options, paths: &Paths) -> Result<Outcome> {
    if let Some(sum) = opts.checksum.clone() {
        let part = paths.part.clone();
        tokio::task::spawn_blocking(move || sum.verify_file(&part)).await.map_err(std::io::Error::other)??;
    }
    fs::rename(&paths.part, &paths.target)?;
    match fs::remove_file(&paths.state) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    Ok(Outcome::Completed(paths.target.clone()))
}

// ---------------------------------------------------------------------------
// Segmented mode

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
async fn run_segmented(
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

/// Stream one ranged request for segment `seg`, starting where it left off.
async fn fetch(id: usize, seg: usize, shared: &Shared) -> Result<FetchEnd> {
    let stat = &shared.workers[id];
    let (pos, end) = {
        let plan = shared.plan.lock().unwrap();
        let s = plan.segment(seg);
        (s.received, s.end)
    };
    if pos >= end {
        return Ok(FetchEnd::Done);
    }

    let mut req = shared
        .client
        .get(&shared.url)
        .headers(shared.opts.headers.clone())
        .header(header::RANGE, format!("bytes={}-{}", pos, end - 1));
    if let Some(v) = &shared.validator {
        req = req.header(header::IF_RANGE, v);
    }
    let stall = shared.opts.stall_timeout;
    let resp = tokio::select! {
        _ = shared.stop.cancelled() => return Ok(FetchEnd::Stopped),
        r = tokio::time::timeout(stall, req.send()) => r.map_err(|_| Error::Stalled(stall))??,
    };
    // Segment requests go straight to the probed URL, so any redirect means
    // the server sent us away (often to an HTML "too many requests" page).
    if resp.url().as_str() != shared.url {
        return Err(Error::Refused(resp.url().to_string()));
    }
    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {}
        // A full body or an unsatisfiable range means the file is not the one
        // we started with (If-Range failed) or the server stopped honouring
        // ranges. Either way the bytes on disk can no longer be trusted.
        StatusCode::OK | StatusCode::RANGE_NOT_SATISFIABLE => return Err(Error::Changed),
        _ => return Err(probe::status_error(&resp)),
    }
    let content_range = resp.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).unwrap_or("");
    check_content_range(content_range, pos, shared.size)?;

    stat.restart.store(false, Ordering::Relaxed);
    stat.conn.fetch_add(1, Ordering::Relaxed);
    *stat.since.lock().unwrap() = Instant::now();
    stat.streaming.store(true, Ordering::Relaxed);
    shared.progress.connections.fetch_add(1, Ordering::Relaxed);
    let mut writer = SegmentWriter { seg, offset: pos, buf: Vec::with_capacity(shared.opts.buffer), shared };
    let result = stream_segment(resp, &mut writer, stat, shared).await;
    stat.streaming.store(false, Ordering::Relaxed);
    shared.progress.connections.fetch_sub(1, Ordering::Relaxed);
    // Whatever happened, keep what was received.
    let flushed = writer.flush().await;
    let end = result?;
    flushed?;
    Ok(end)
}

fn check_content_range(value: &str, pos: u64, size: u64) -> Result<()> {
    let parsed = value.strip_prefix("bytes ").and_then(|r| {
        let (range, total) = r.split_once('/')?;
        let start: u64 = range.split_once('-')?.0.parse().ok()?;
        Some((start, total.parse::<u64>().ok()))
    });
    match parsed {
        Some((start, total)) if start == pos => match total {
            Some(t) if t != size => Err(Error::Changed),
            _ => Ok(()),
        },
        _ => Err(Error::BadRange(value.to_owned())),
    }
}

async fn stream_segment(
    resp: reqwest::Response,
    writer: &mut SegmentWriter<'_>,
    stat: &WorkerStat,
    shared: &Shared,
) -> Result<FetchEnd> {
    let poll = Duration::from_millis(500);
    let mut idle = Duration::ZERO;
    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            _ = shared.stop.cancelled() => return Ok(FetchEnd::Stopped),
            r = tokio::time::timeout(poll, stream.next()) => r,
        };
        let chunk = match next {
            Err(_) => {
                idle += poll;
                if stat.restart.load(Ordering::Relaxed) {
                    return Ok(FetchEnd::Restart);
                }
                if idle >= shared.opts.stall_timeout {
                    return Err(Error::Stalled(idle));
                }
                continue;
            }
            Ok(None) => {
                let s = shared.plan.lock().unwrap();
                let s = s.segment(writer.seg);
                return if s.received >= s.end { Ok(FetchEnd::Done) } else { Err(Error::Incomplete) };
            }
            Ok(Some(chunk)) => chunk?,
        };
        idle = Duration::ZERO;

        // Take only what still belongs to this segment; a split may have
        // moved its end since the request went out.
        let (take, done) = {
            let mut plan = shared.plan.lock().unwrap();
            let s = plan.segment_mut(writer.seg);
            let take = (chunk.len() as u64).min(s.end.saturating_sub(s.received));
            s.received += take;
            (take as usize, s.received >= s.end)
        };
        writer.buf.extend_from_slice(&chunk[..take]);
        stat.bytes.fetch_add(take as u64, Ordering::Relaxed);
        shared.progress.received.fetch_add(take as u64, Ordering::Relaxed);
        tokio::select! {
            _ = shared.stop.cancelled() => return Ok(FetchEnd::Stopped),
            _ = throttle(&shared.opts.limiter, take as u64) => {}
        }

        if done {
            return Ok(FetchEnd::Done);
        }
        if writer.buf.len() >= shared.opts.buffer {
            writer.flush().await?;
        }
        if stat.restart.load(Ordering::Relaxed) {
            return Ok(FetchEnd::Restart);
        }
    }
}

struct SegmentWriter<'a> {
    seg: usize,
    /// File offset of `buf[0]`.
    offset: u64,
    buf: Vec<u8>,
    shared: &'a Shared,
}

impl SegmentWriter<'_> {
    async fn flush(&mut self) -> Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let file = self.shared.file.clone();
        let data = std::mem::take(&mut self.buf);
        let offset = self.offset;
        let mut data = blocking(move || file.write_all_at(&data, offset).map(|()| data)).await?;
        self.offset += data.len() as u64;
        self.shared.plan.lock().unwrap().segment_mut(self.seg).written = self.offset;
        data.clear();
        self.buf = data;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stream mode

#[allow(clippy::too_many_arguments)]
async fn run_stream(
    client: Client,
    opts: Options,
    url: String,
    probe: Probe,
    paths: Paths,
    file: File,
    progress: Arc<Progress>,
    cancel: CancellationToken,
) -> Result<Outcome> {
    // Nothing to resume from, but the state file keeps the name: the next run
    // restarts into this same part instead of abandoning it for `name (1)`.
    ResumeState::new(url, probe.final_url.clone(), probe.size.unwrap_or(0), None, None).save(&paths.state)?;
    let file = Arc::new(file);
    let mut attempt = 0;
    loop {
        match stream_once(&client, &opts, &probe, &file, &progress, &cancel).await {
            Ok(true) => return finish(&opts, &paths).await,
            Ok(false) => return Ok(Outcome::Paused),
            Err(e) if e.is_retryable() && attempt < opts.max_retries => {
                attempt += 1;
                progress.retries.fetch_add(1, Ordering::Relaxed);
                progress.received.store(0, Ordering::Relaxed);
                tokio::time::sleep(backoff(attempt, e.retry_after())).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// One full GET into the part file. Returns false when cancelled.
async fn stream_once(
    client: &Client,
    opts: &Options,
    probe: &Probe,
    file: &Arc<File>,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<bool> {
    file.set_len(0)?;
    let resp = client.get(&probe.final_url).headers(opts.headers.clone()).send().await?;
    if resp.status() != StatusCode::OK {
        return Err(probe::status_error(&resp));
    }
    progress.connections.store(1, Ordering::Relaxed);
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(opts.buffer);
    let mut offset = 0u64;
    let result = loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => break Ok(false),
            r = tokio::time::timeout(opts.stall_timeout, stream.next()) => r,
        };
        match next {
            Err(_) => break Err(Error::Stalled(opts.stall_timeout)),
            Ok(None) => break Ok(true),
            Ok(Some(Err(e))) => break Err(e.into()),
            Ok(Some(Ok(chunk))) => {
                buf.extend_from_slice(&chunk);
                progress.received.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                tokio::select! {
                    _ = cancel.cancelled() => break Ok(false),
                    _ = throttle(&opts.limiter, chunk.len() as u64) => {}
                }
                if buf.len() >= opts.buffer {
                    let (f, data) = (file.clone(), std::mem::take(&mut buf));
                    let data = blocking(move || f.write_all_at(&data, offset).map(|()| data)).await?;
                    offset += data.len() as u64;
                    buf = data;
                    buf.clear();
                }
            }
        }
    };
    progress.connections.store(0, Ordering::Relaxed);
    if !buf.is_empty() {
        let f = file.clone();
        blocking(move || f.write_all_at(&buf, offset)).await?;
    }
    let complete = result?;
    if complete {
        if let Some(size) = probe.size
            && progress.received.load(Ordering::Relaxed) != size
        {
            return Err(Error::Incomplete);
        }
        let f = file.clone();
        blocking(move || f.sync_all()).await?;
    }
    Ok(complete)
}
