//! Retrying with backoff, and speed limits.

use std::sync::Arc;
use std::time::Duration;

use tracing::debug;

use super::Options;
use crate::error::{Error, Result};
use crate::limit::Limiter;

/// The longest a `Retry-After` from the server can make us wait.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);

pub(super) fn backoff(attempt: u32, hint: Option<Duration>) -> Duration {
    hint.map(|h| h.min(MAX_RETRY_AFTER)).unwrap_or_else(|| {
        let base = Duration::from_millis(500) * 2u32.saturating_pow(attempt.saturating_sub(1).min(6));
        base.min(Duration::from_secs(30)).mul_f64(0.5 + fastrand::f64())
    })
}

pub(super) async fn retry<T, F, Fut>(opts: &Options, mut f: F) -> Result<T>
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

pub(super) async fn throttle(limiters: &[Arc<Limiter>], n: u64) {
    for l in limiters {
        l.acquire(n).await;
    }
}
