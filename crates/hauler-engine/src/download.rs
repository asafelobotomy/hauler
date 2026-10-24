//! The download driver: probe, plan, parallel ranged fetches, resume.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Url};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::checksum::Checksum;
use crate::error::{Error, Result};
use crate::limit::Limiter;
use crate::plan::{Plan, SegmentView};
use crate::probe::{self, Probe, sanitize_filename};
use crate::state::ResumeState;

mod files;
mod retry;
mod segmented;
mod stream;

use files::{Paths, free_name, lock_part, part_path, state_path};
use retry::retry;
use segmented::run_segmented;
use stream::run_stream;

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
