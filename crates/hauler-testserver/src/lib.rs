//! A small HTTP/1.1 server for testing and benchmarking hauler.
//!
//! It serves one synthetic file whose bytes are a pure function of their
//! offset, so clients can verify any range without the server keeping the
//! file in memory. It supports `Range`, `ETag` and `If-Range`, and can
//! throttle each connection to mimic servers that cap per-connection speed,
//! or cap how many responses stream at once, redirecting the rest to an HTML
//! "too many requests" page the way some file hosts do.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Empty, StreamBody, combinators::BoxBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, header};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

const CHUNK: usize = 16 * 1024;

/// Byte at `offset` of the synthetic file.
pub fn content_byte(offset: u64) -> u8 {
    let x = offset.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (offset >> 13);
    (x >> 56) as u8
}

/// Fill `buf` with the synthetic file's bytes starting at `offset`.
pub fn fill(offset: u64, buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = content_byte(offset + i as u64);
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Size of the served file in bytes.
    pub size: u64,
    /// Honour `Range` requests. When false every request gets a full 200.
    pub ranges: bool,
    /// Per-connection speed cap in bytes per second.
    pub bytes_per_sec: Option<u64>,
    /// Make every Nth request this many times slower than `bytes_per_sec`.
    pub slow_every: Option<(u64, u64)>,
    /// Send `Content-Disposition: attachment; filename=...`.
    pub filename: Option<String>,
    /// Answer every request with a 302 to this URL instead.
    pub redirect_to: Option<String>,
    /// At most this many responses stream at once; further requests for the
    /// file get a 302 to `/busy.html`, a small HTML page.
    pub max_streams: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            size: 8 * 1024 * 1024,
            ranges: true,
            bytes_per_sec: None,
            slow_every: None,
            filename: None,
            redirect_to: None,
            max_streams: None,
        }
    }
}

/// Live state shared with the test that started the server.
#[derive(Debug, Default)]
pub struct Stats {
    /// Requests served so far.
    pub requests: AtomicU64,
    /// Bump to change the ETag, simulating the file changing on the server.
    pub version: AtomicU64,
    /// Requests that carried a `Cookie` header.
    pub cookie_requests: AtomicU64,
    /// Responses streaming right now.
    pub streaming: AtomicU64,
    /// Requests sent to `/busy.html` because of `max_streams`.
    pub refused: AtomicU64,
}

/// Counts a response as streaming until its body is dropped.
struct Streaming(Arc<Stats>);

impl Drop for Streaming {
    fn drop(&mut self) {
        self.0.streaming.fetch_sub(1, Ordering::SeqCst);
    }
}

const BUSY_PAGE: &str = "<html><body>Too many requests. Finish your current download first.</body></html>";

pub struct Server {
    pub addr: SocketAddr,
    pub stats: Arc<Stats>,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://{}/file.bin", self.addr)
    }
}

/// Start the server on an ephemeral localhost port.
pub async fn spawn(config: Config) -> std::io::Result<Server> {
    spawn_on("127.0.0.1:0".parse().unwrap(), config).await
}

pub async fn spawn_on(addr: SocketAddr, config: Config) -> std::io::Result<Server> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let stats = Arc::new(Stats::default());
    let config = Arc::new(config);
    let server_stats = stats.clone();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let config = config.clone();
            let stats = server_stats.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req| handle(req, config.clone(), stats.clone()));
                let _ = http1::Builder::new().serve_connection(TokioIo::new(tcp), service).await;
            });
        }
    });
    Ok(Server { addr, stats })
}

type Body = BoxBody<Bytes, Infallible>;

async fn handle(
    req: Request<hyper::body::Incoming>,
    config: Arc<Config>,
    stats: Arc<Stats>,
) -> Result<Response<Body>, Infallible> {
    let n = stats.requests.fetch_add(1, Ordering::SeqCst) + 1;
    if req.headers().contains_key(header::COOKIE) {
        stats.cookie_requests.fetch_add(1, Ordering::SeqCst);
    }
    if let Some(to) = &config.redirect_to {
        let resp = Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, to)
            .body(Empty::new().boxed())
            .unwrap();
        return Ok(resp);
    }
    if req.uri().path() == "/busy.html" {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .header(header::CONTENT_LENGTH, BUSY_PAGE.len())
            .body(BodyExt::boxed(http_body_util::Full::new(Bytes::from_static(BUSY_PAGE.as_bytes()))))
            .unwrap();
        return Ok(resp);
    }
    let streaming = match config.max_streams {
        Some(max) => {
            if stats.streaming.fetch_add(1, Ordering::SeqCst) >= max {
                stats.streaming.fetch_sub(1, Ordering::SeqCst);
                stats.refused.fetch_add(1, Ordering::SeqCst);
                let resp = Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "/busy.html")
                    .body(Empty::new().boxed())
                    .unwrap();
                return Ok(resp);
            }
            Some(Streaming(stats.clone()))
        }
        None => None,
    };
    let etag = format!("\"v{}-{}\"", stats.version.load(Ordering::SeqCst), config.size);
    let size = config.size;

    let mut range = None;
    if config.ranges {
        let if_range_ok = match req.headers().get(header::IF_RANGE) {
            Some(v) => v.to_str().ok() == Some(etag.as_str()),
            None => true,
        };
        if if_range_ok {
            range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| parse_range(v, size));
            if req.headers().contains_key(header::RANGE) && range.is_none() {
                let resp = Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                    .body(Empty::new().boxed())
                    .unwrap();
                return Ok(resp);
            }
        }
    }

    let (status, start, end) = match range {
        Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e),
        None => (StatusCode::OK, 0, size),
    };

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, end - start)
        .header(header::ETAG, &etag)
        .header(header::LAST_MODIFIED, "Wed, 21 Oct 2026 07:28:00 GMT");
    if config.ranges {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end - 1, size));
    }
    if let Some(name) = &config.filename {
        builder = builder.header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""));
    }
    if req.method() == hyper::Method::HEAD {
        return Ok(builder.body(Empty::new().boxed()).unwrap());
    }

    let mut rate = config.bytes_per_sec;
    if let (Some(r), Some((every, factor))) = (rate, config.slow_every)
        && every > 0
        && n.is_multiple_of(every)
    {
        rate = Some((r / factor.max(1)).max(1));
    }

    let body = stream::unfold((start, streaming), move |(pos, streaming)| async move {
        if pos >= end {
            return None;
        }
        let len = CHUNK.min((end - pos) as usize);
        if let Some(r) = rate {
            tokio::time::sleep(Duration::from_secs_f64(len as f64 / r as f64)).await;
        }
        let mut buf = vec![0u8; len];
        fill(pos, &mut buf);
        Some((Ok::<_, Infallible>(Frame::data(Bytes::from(buf))), (pos + len as u64, streaming)))
    });
    Ok(builder.body(BodyExt::boxed(StreamBody::new(body))).unwrap())
}

/// Parse a single `bytes=a-b`, `bytes=a-` or `bytes=-n` range into a
/// half-open `[start, end)` interval.
fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (size.saturating_sub(n), size)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { size } else { b.parse::<u64>().ok()?.saturating_add(1).min(size) };
        (start, end)
    };
    (start < end && start < size).then_some((start, end))
}
