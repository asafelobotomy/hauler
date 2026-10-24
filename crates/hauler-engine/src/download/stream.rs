//! Stream mode: one connection from the start, not resumable.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use tokio_util::sync::CancellationToken;

use super::files::{Paths, blocking, finish};
use super::retry::{backoff, throttle};
use super::{Options, Outcome, Progress};
use crate::error::{Error, Result};
use crate::probe::{self, Probe};
use crate::state::ResumeState;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_stream(
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
