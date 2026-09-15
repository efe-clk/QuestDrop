//! Minimal in-memory fixed-window rate limiter (per IP).
//! Protects write endpoints from spam floods until auth + Redis arrive.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct RateLimiter {
    max_hits: u32,
    window: Duration,
    inner: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new(max_hits: u32, window: Duration) -> Self {
        Self {
            max_hits,
            window,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `Some(retry_after_secs)` when the caller is over the limit.
    pub fn check(&self, ip: IpAddr) -> Option<u64> {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();
        let hits = map.entry(ip).or_default();
        hits.retain(|t| now.duration_since(*t) < self.window);
        if hits.len() >= self.max_hits as usize {
            let oldest = hits.iter().min().copied().unwrap_or(now);
            let retry = self
                .window
                .checked_sub(now.duration_since(oldest))
                .unwrap_or(Duration::from_secs(1));
            Some(retry.as_secs().max(1))
        } else {
            hits.push(now);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn blocks_after_max_and_recovers() {
        let lim = RateLimiter::new(2, Duration::from_millis(50));
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(lim.check(ip).is_none());
        assert!(lim.check(ip).is_none());
        assert!(lim.check(ip).is_some());
        std::thread::sleep(Duration::from_millis(60));
        assert!(lim.check(ip).is_none());
    }
}
