//! Per-domain rate limiter using token bucket algorithm.
//!
//! Prevents hammering servers by accident. Each domain gets its own bucket
//! with configurable requests-per-second and burst capacity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Handle stored in deno_core OpState for access from ops.
pub type RateLimiterHandle = Arc<Mutex<RateLimiter>>;

/// Per-domain rate limiter with token bucket algorithm.
pub struct RateLimiter {
    limits: HashMap<String, DomainLimit>,
    default_rps: f64,
    default_burst: u32,
}

struct DomainLimit {
    rps: f64,
    burst: u32,
    tokens: f64,
    last_check: Instant,
}

impl DomainLimit {
    fn new(rps: f64, burst: u32) -> Self {
        Self {
            rps,
            burst,
            tokens: burst as f64, // Start full
            last_check: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time, capped at burst.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_check).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rps).min(self.burst as f64);
        self.last_check = now;
    }

    /// Try to consume one token. Returns None if allowed, Some(wait) if not.
    fn try_consume(&mut self) -> Option<Duration> {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            // How long until we have 1 token?
            let deficit = 1.0 - self.tokens;
            let wait_secs = deficit / self.rps;
            Some(Duration::from_secs_f64(wait_secs))
        }
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            limits: HashMap::new(),
            default_rps: 1.0,
            default_burst: 5,
        }
    }

    /// Set custom rate limit for a domain.
    pub fn set_limit(&mut self, domain: &str, rps: f64, burst: u32) {
        let entry = self.limits.entry(domain.to_string()).or_insert_with(|| DomainLimit::new(rps, burst));
        entry.rps = rps;
        entry.burst = burst;
        eprintln!("[NEORENDER:RATE] Set {domain}: {rps} req/s, burst {burst}");
    }

    /// Check if a request to this domain is allowed.
    /// Returns None if allowed, Some(Duration) if the caller should wait.
    pub fn check(&mut self, domain: &str) -> Option<Duration> {
        let default_rps = self.default_rps;
        let default_burst = self.default_burst;
        let entry = self.limits
            .entry(domain.to_string())
            .or_insert_with(|| DomainLimit::new(default_rps, default_burst));
        entry.try_consume()
    }

    /// Block until the request is allowed. Logs when rate-limited.
    pub fn wait_if_needed(&mut self, domain: &str) {
        if let Some(wait) = self.check(domain) {
            eprintln!("[NEORENDER:RATE] Rate limited: {domain} — waiting {:.0}ms", wait.as_millis());
            std::thread::sleep(wait);
            // Consume the token after waiting
            let entry = self.limits.get_mut(domain).unwrap();
            entry.refill();
            entry.tokens -= 1.0;
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
