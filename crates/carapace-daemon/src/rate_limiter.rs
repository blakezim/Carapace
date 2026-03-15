//! Sliding-window rate limiter.
//!
//! Tracks request timestamps per key (method name for now; channel name in
//! Phase 5). Uses `std::sync::Mutex` because the critical section is tiny
//! and never awaits.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::{RateLimitConfig, RateLimitEntry};

/// Result of a rate-limit check.
#[derive(Debug, PartialEq, Eq)]
pub enum RateLimitResult {
    Allowed,
    Exceeded { limit: u32, window_secs: u64 },
}

/// Sliding-window rate limiter with per-key tracking.
pub struct RateLimiter {
    config: RateLimitConfig,
    windows: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Check the rate limit for `key` and record the attempt.
    ///
    /// The attempt is recorded **before** checking, so blocked requests
    /// also count toward the limit (prevents probing).
    pub fn check_and_record(&self, key: &str) -> RateLimitResult {
        let entry = match self.resolve_limit(key) {
            Some(e) => e,
            None => return RateLimitResult::Allowed, // No limit configured
        };

        let window = Duration::from_secs(entry.per_seconds);
        let now = Instant::now();
        let cutoff = now - window;

        let mut windows = self.windows.lock().unwrap();
        let timestamps = windows.entry(key.to_string()).or_default();

        // Record this attempt first.
        timestamps.push(now);

        // Prune timestamps outside the window.
        timestamps.retain(|t| *t > cutoff);

        if timestamps.len() as u32 > entry.requests {
            RateLimitResult::Exceeded {
                limit: entry.requests,
                window_secs: entry.per_seconds,
            }
        } else {
            RateLimitResult::Allowed
        }
    }

    /// Look up the rate limit for a key, falling back to "default".
    fn resolve_limit(&self, key: &str) -> Option<RateLimitEntry> {
        self.config
            .entries
            .get(key)
            .or_else(|| self.config.entries.get("default"))
            .cloned()
    }

    /// Prune all timestamps older than their respective windows.
    ///
    /// Called periodically from a background task to prevent unbounded
    /// memory growth.
    pub fn cleanup(&self) {
        let mut windows = self.windows.lock().unwrap();
        let now = Instant::now();

        windows.retain(|key, timestamps| {
            if let Some(entry) = self.resolve_limit(key) {
                let cutoff = now - Duration::from_secs(entry.per_seconds);
                timestamps.retain(|t| *t > cutoff);
                !timestamps.is_empty()
            } else {
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_default(requests: u32, per_seconds: u64) -> RateLimitConfig {
        let mut entries = HashMap::new();
        entries.insert(
            "default".into(),
            RateLimitEntry {
                requests,
                per_seconds,
            },
        );
        RateLimitConfig { entries }
    }

    #[test]
    fn allows_within_limit() {
        let limiter = RateLimiter::new(config_with_default(3, 60));
        assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
        assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
        assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
    }

    #[test]
    fn blocks_over_limit() {
        let limiter = RateLimiter::new(config_with_default(2, 60));
        assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
        assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
        assert_eq!(
            limiter.check_and_record("test"),
            RateLimitResult::Exceeded {
                limit: 2,
                window_secs: 60
            }
        );
    }

    #[test]
    fn separate_keys_independent() {
        let limiter = RateLimiter::new(config_with_default(1, 60));
        assert_eq!(limiter.check_and_record("a"), RateLimitResult::Allowed);
        assert_eq!(limiter.check_and_record("b"), RateLimitResult::Allowed);
        // "a" is now over limit
        assert_eq!(
            limiter.check_and_record("a"),
            RateLimitResult::Exceeded {
                limit: 1,
                window_secs: 60
            }
        );
        // "b" is also over limit independently
        assert_eq!(
            limiter.check_and_record("b"),
            RateLimitResult::Exceeded {
                limit: 1,
                window_secs: 60
            }
        );
    }

    #[test]
    fn no_limit_always_allows() {
        let limiter = RateLimiter::new(RateLimitConfig {
            entries: HashMap::new(),
        });
        for _ in 0..100 {
            assert_eq!(limiter.check_and_record("test"), RateLimitResult::Allowed);
        }
    }

    #[test]
    fn channel_specific_overrides_default() {
        let mut entries = HashMap::new();
        entries.insert(
            "default".into(),
            RateLimitEntry {
                requests: 10,
                per_seconds: 60,
            },
        );
        entries.insert(
            "imsg".into(),
            RateLimitEntry {
                requests: 1,
                per_seconds: 60,
            },
        );
        let limiter = RateLimiter::new(RateLimitConfig { entries });

        // "imsg" uses its own limit (1)
        assert_eq!(limiter.check_and_record("imsg"), RateLimitResult::Allowed);
        assert_eq!(
            limiter.check_and_record("imsg"),
            RateLimitResult::Exceeded {
                limit: 1,
                window_secs: 60
            }
        );

        // "other" falls back to default (10)
        for _ in 0..10 {
            assert_eq!(limiter.check_and_record("other"), RateLimitResult::Allowed);
        }
        assert_eq!(
            limiter.check_and_record("other"),
            RateLimitResult::Exceeded {
                limit: 10,
                window_secs: 60
            }
        );
    }
}
