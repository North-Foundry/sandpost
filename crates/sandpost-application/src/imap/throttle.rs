//! Bounded in-memory limiting of failed IMAP logins per client address and per username.
//!
//! Failures are counted in a sliding window. A client address or username over its limit is
//! refused without verifying the password, which also bounds Argon2 work an attacker can cause.
//! A successful login clears the counters of its address and username.
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Window in which failures are counted.
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failures from one client address within the window before it is refused.
const MAXIMUM_ADDRESS_FAILURES: usize = 20;
/// Failures for one username within the window before it is refused, from any address.
const MAXIMUM_USERNAME_FAILURES: usize = 50;
/// Most tracked keys; the least recently failing ones are dropped beyond it.
const MAXIMUM_TRACKED_KEYS: usize = 4096;

/// What a counter is keyed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ThrottleKey {
    Address(IpAddr),
    Username(String),
}

/// Recent failure times per key.
#[derive(Default)]
pub(crate) struct LoginThrottle {
    failures: Mutex<HashMap<ThrottleKey, Vec<Instant>>>,
}

impl LoginThrottle {
    /// The keys an attempt is counted under; usernames compare ASCII-case-insensitively.
    fn keys(address: Option<IpAddr>, username: &str) -> Vec<(ThrottleKey, usize)> {
        let mut keys = vec![(
            ThrottleKey::Username(username.to_ascii_lowercase()),
            MAXIMUM_USERNAME_FAILURES,
        )];
        if let Some(address) = address {
            keys.push((ThrottleKey::Address(address), MAXIMUM_ADDRESS_FAILURES));
        }
        keys
    }

    /// Report whether an attempt must be refused without checking the password.
    pub(crate) fn is_blocked(&self, address: Option<IpAddr>, username: &str) -> bool {
        self.is_blocked_at(address, username, Instant::now())
    }

    /// Report whether an attempt at `now` must be refused.
    fn is_blocked_at(&self, address: Option<IpAddr>, username: &str, now: Instant) -> bool {
        let failures = self
            .failures
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self::keys(address, username)
            .into_iter()
            .any(|(key, limit)| {
                failures.get(&key).is_some_and(|times| {
                    times
                        .iter()
                        .filter(|time| now.duration_since(**time) < FAILURE_WINDOW)
                        .count()
                        >= limit
                })
            })
    }

    /// Count a failed attempt.
    pub(crate) fn record_failure(&self, address: Option<IpAddr>, username: &str) {
        self.record_failure_at(address, username, Instant::now());
    }

    /// Count a failed attempt at `now`, pruning expired failures and bounding memory.
    fn record_failure_at(&self, address: Option<IpAddr>, username: &str, now: Instant) {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for (key, limit) in Self::keys(address, username) {
            let times = failures.entry(key).or_default();
            times.retain(|time| now.duration_since(*time) < FAILURE_WINDOW);
            times.push(now);
            if times.len() > limit {
                times.remove(0);
            }
        }
        if failures.len() > MAXIMUM_TRACKED_KEYS {
            failures.retain(|_, times| {
                times.retain(|time| now.duration_since(*time) < FAILURE_WINDOW);
                !times.is_empty()
            });
            while failures.len() > MAXIMUM_TRACKED_KEYS {
                let oldest = failures
                    .iter()
                    .min_by_key(|(_, times)| times.last().copied())
                    .map(|(key, _)| key.clone());
                match oldest {
                    Some(key) => {
                        failures.remove(&key);
                    }
                    None => break,
                }
            }
        }
    }

    /// Clear the counters of a successful login.
    pub(crate) fn record_success(&self, address: Option<IpAddr>, username: &str) {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for (key, _) in Self::keys(address, username) {
            failures.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address is refused after its limit and released when the window passes.
    #[test]
    fn addresses_are_limited_within_the_window() {
        let throttle = LoginThrottle::default();
        let address = Some("192.0.2.1".parse().unwrap());
        let start = Instant::now();
        for attempt in 0..MAXIMUM_ADDRESS_FAILURES {
            assert!(!throttle.is_blocked_at(address, &format!("user{attempt}"), start));
            throttle.record_failure_at(address, &format!("user{attempt}"), start);
        }
        assert!(throttle.is_blocked_at(address, "fresh", start));
        assert!(!throttle.is_blocked_at(Some("192.0.2.2".parse().unwrap()), "fresh", start));
        assert!(!throttle.is_blocked_at(address, "fresh", start + FAILURE_WINDOW));
    }

    /// A username is refused from any address after its own limit; success clears it.
    #[test]
    fn usernames_are_limited_and_success_clears() {
        let throttle = LoginThrottle::default();
        let now = Instant::now();
        for attempt in 0..MAXIMUM_USERNAME_FAILURES {
            let address = Some(IpAddr::from([10, 0, (attempt / 256) as u8, attempt as u8]));
            throttle.record_failure_at(address, "Target", now);
        }
        assert!(throttle.is_blocked_at(Some("198.51.100.1".parse().unwrap()), "target", now));
        throttle.record_success(None, "TARGET");
        assert!(!throttle.is_blocked_at(Some("198.51.100.1".parse().unwrap()), "target", now));
    }

    /// Tracked keys stay bounded.
    #[test]
    fn memory_is_bounded() {
        let throttle = LoginThrottle::default();
        let now = Instant::now();
        for index in 0..(MAXIMUM_TRACKED_KEYS as u32) {
            throttle.record_failure_at(None, &format!("user{index}"), now);
        }
        throttle.record_failure_at(None, "one more", now);
        assert!(throttle.failures.lock().unwrap().len() <= MAXIMUM_TRACKED_KEYS);
    }
}
