//! Token-bucket speed limits that can be shared and changed at runtime.
//!
//! A download holds a list of limiters, for example its own and a global one,
//! and waits on each after receiving a chunk. Waiting stops reads from the
//! socket, so TCP flow control slows the server down too.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug)]
pub struct Limiter {
    bucket: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    /// Bytes per second; 0 means unlimited.
    rate: u64,
    /// May go negative: a chunk is admitted at once and its debt is waited off.
    tokens: f64,
    last: Instant,
}

/// Longest single sleep, so a raised limit takes effect quickly.
const MAX_WAIT: Duration = Duration::from_millis(250);

impl Limiter {
    pub fn new(bytes_per_sec: u64) -> Arc<Self> {
        Arc::new(Self { bucket: Mutex::new(Bucket { rate: bytes_per_sec, tokens: 0.0, last: Instant::now() }) })
    }

    pub fn unlimited() -> Arc<Self> {
        Self::new(0)
    }

    pub fn rate(&self) -> u64 {
        self.bucket.lock().unwrap().rate
    }

    /// Change the limit; 0 removes it.
    pub fn set_rate(&self, bytes_per_sec: u64) {
        let mut b = self.bucket.lock().unwrap();
        b.refill();
        b.rate = bytes_per_sec;
        b.tokens = b.tokens.min(b.burst());
    }

    /// Account for `n` bytes, sleeping as long as the limit requires.
    pub async fn acquire(&self, n: u64) {
        {
            let mut b = self.bucket.lock().unwrap();
            if b.rate == 0 {
                return;
            }
            b.refill();
            b.tokens -= n as f64;
        }
        loop {
            let wait = {
                let mut b = self.bucket.lock().unwrap();
                if b.rate == 0 {
                    b.tokens = 0.0;
                    return;
                }
                b.refill();
                if b.tokens >= 0.0 {
                    return;
                }
                Duration::from_secs_f64(-b.tokens / b.rate as f64)
            };
            tokio::time::sleep(wait.min(MAX_WAIT)).await;
        }
    }
}

impl Bucket {
    /// Allow a quarter second of burst, at least 64 KiB.
    fn burst(&self) -> f64 {
        (self.rate as f64 / 4.0).max(64.0 * 1024.0)
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        if self.rate > 0 {
            self.tokens = (self.tokens + elapsed * self.rate as f64).min(self.burst());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn holds_the_average_rate() {
        let limiter = Limiter::new(1024 * 1024);
        let start = tokio::time::Instant::now();
        for _ in 0..64 {
            limiter.acquire(64 * 1024).await;
        }
        // 4 MiB at 1 MiB/s, minus the initial burst allowance.
        let secs = start.elapsed().as_secs_f64();
        assert!((3.5..=4.1).contains(&secs), "{secs}");
    }

    #[tokio::test(start_paused = true)]
    async fn unlimited_never_waits_and_rate_can_change() {
        let limiter = Limiter::unlimited();
        let start = tokio::time::Instant::now();
        limiter.acquire(1 << 30).await;
        assert_eq!(start.elapsed(), Duration::ZERO);

        limiter.set_rate(1024);
        assert_eq!(limiter.rate(), 1024);
        limiter.set_rate(0);
        limiter.acquire(1 << 30).await;
    }
}
