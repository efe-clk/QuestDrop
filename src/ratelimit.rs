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

/// Upper bound on tracked IPs. Without eviction a flood of spoofed source
/// IPs grows the map without limit; the sweep below keeps it bounded.
const MAX_TRACKED_IPS: usize = 10_000;

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
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        if map.len() >= MAX_TRACKED_IPS {
            map.retain(|_, hits| hits.iter().any(|t| now.duration_since(*t) < self.window));
            if map.len() >= MAX_TRACKED_IPS {
                map.clear();
            }
        }
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

    #[test]
    fn sweep_bounds_memory_under_ip_flood() {
        let lim = RateLimiter::new(1_000_000, Duration::from_secs(60));
        for i in 0..(MAX_TRACKED_IPS as u32 + 100) {
            let ip: IpAddr = format!("10.{}.{}.{}", (i >> 16) & 255, (i >> 8) & 255, i & 255)
                .parse()
                .unwrap();
            let _ = lim.check(ip);
        }
        let fresh: IpAddr = "192.168.0.1".parse().unwrap();
        assert!(lim.check(fresh).is_none(), "limiter must survive IP flood");
        assert!(lim.inner.lock().unwrap().len() <= MAX_TRACKED_IPS + 1);
    }
}
