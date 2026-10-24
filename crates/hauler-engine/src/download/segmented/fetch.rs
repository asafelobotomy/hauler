//! One ranged request: send it, check the answer, stream it to disk.

use std::os::unix::fs::FileExt;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header;

use super::super::files::blocking;
use super::super::retry::throttle;
use super::{FetchEnd, Shared, WorkerStat};
use crate::error::{Error, Result};
use crate::probe;

/// Stream one ranged request for segment `seg`, starting where it left off.
pub(super) async fn fetch(id: usize, seg: usize, shared: &Shared) -> Result<FetchEnd> {
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
