//! Names who keeps presenting a refused relay token without trusting the token,
//! and keeps the log to one line per client kind and subject per window.
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::auth::{RelayAction, RelayAuthError};

/// One log line per client kind and subject in this window; the rest are counted.
const REFUSAL_LOG_WINDOW: Duration = Duration::from_secs(60);
/// Bound on distinct (kind, subject) pairs remembered between windows.
const REFUSAL_LOG_CAPACITY: usize = 1024;

/// Claims read from a refused token. Nothing here is verified: the token was
/// refused, so it only helps an operator find the client that sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnverifiedSubject {
    pub subject_kind: Option<String>,
    /// First 8 hex digits of SHA-256 over the subject: stable, not reversible.
    pub subject_hash: Option<String>,
    pub expired_for_s: Option<u64>,
}

impl UnverifiedSubject {
    pub(crate) fn read(token: &str, now_ms: u64) -> Self {
        let claims = token
            .trim()
            .split('.')
            .nth(1)
            .and_then(|segment| URL_SAFE_NO_PAD.decode(segment).ok())
            .and_then(|payload| serde_json::from_slice::<Value>(&payload).ok());
        let text = |keys: &[&str]| {
            claims.as_ref().and_then(|claims| {
                keys.iter()
                    .find_map(|key| claims.get(*key).and_then(Value::as_str))
                    .map(str::to_string)
            })
        };
        let expires_at_ms = claims.as_ref().and_then(|claims| {
            claims
                .get("expires_at_ms")
                .and_then(Value::as_u64)
                .or_else(|| claims.get("exp").and_then(Value::as_u64).map(|s| s * 1000))
        });
        Self {
            subject_kind: text(&["subject_kind"]).map(|kind| printable(&kind, 16)),
            subject_hash: text(&["sub", "subject"]).map(|subject| {
                Sha256::digest(subject.as_bytes())
                    .iter()
                    .take(4)
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }),
            expired_for_s: expires_at_ms.map(|expiry| now_ms.saturating_sub(expiry) / 1000),
        }
    }
}

/// The error a refused expired token raises. It prints exactly what
/// `RelayAuthError::TokenExpired` printed, which clients match on.
#[derive(Debug)]
pub(crate) struct TokenRefusal {
    pub action: RelayAction,
    pub subject: UnverifiedSubject,
}

impl TokenRefusal {
    pub(crate) fn expired(action: RelayAction, token: &str, now_ms: u64) -> Self {
        Self {
            action,
            subject: UnverifiedSubject::read(token, now_ms),
        }
    }

    pub(crate) fn client_kind(&self) -> &'static str {
        match self.action {
            RelayAction::DaemonRegister | RelayAction::DaemonHeartbeat => "kernel",
            RelayAction::ClientMetadataRead
            | RelayAction::ClientConnect
            | RelayAction::PacketRoute => "client",
            RelayAction::PeerRequest | RelayAction::PeerEvent => "peer",
        }
    }

    fn key(&self) -> (&'static str, String) {
        (
            self.client_kind(),
            self.subject.subject_hash.clone().unwrap_or_default(),
        )
    }

    pub(crate) fn log_fields(&self, hello: &ConnectionHello, counted: u64) -> Value {
        json!({
            "client_kind": self.client_kind(),
            "action": format!("{:?}", self.action),
            "token_subject_kind": self.subject.subject_kind,
            "token_subject_hash": self.subject.subject_hash,
            "token_expired_for_s": self.subject.expired_for_s,
            "user_agent": hello.user_agent,
            "transport": hello.transport,
            "refusals_in_previous_window": counted,
            "window_s": REFUSAL_LOG_WINDOW.as_secs(),
        })
    }
}

impl fmt::Display for TokenRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&RelayAuthError::TokenExpired, formatter)
    }
}

impl std::error::Error for TokenRefusal {}

/// What the websocket handshake tells about the client.
#[derive(Debug, Default, Clone)]
pub(crate) struct ConnectionHello {
    pub user_agent: Option<String>,
    pub transport: String,
}

pub(crate) fn printable(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(limit)
        .collect()
}

/// Refusals counted since the last logged line.
#[derive(Clone, Copy)]
struct RefusalWindow {
    since: Instant,
    count: u64,
}

impl RefusalWindow {
    fn admit(window: &mut Option<Self>, now: Instant) -> Option<u64> {
        if let Some(open) = window.as_mut() {
            if now.duration_since(open.since) < REFUSAL_LOG_WINDOW {
                open.count += 1;
                return None;
            }
        }
        let counted = window.map_or(0, |open| open.count);
        *window = Some(Self {
            since: now,
            count: 1,
        });
        Some(counted)
    }
}

#[derive(Default)]
pub(crate) struct RefusalLogLimiter {
    seen: HashMap<(&'static str, String), RefusalWindow>,
    /// Subjects the full table has no room for share this one line per window.
    overflow: Option<RefusalWindow>,
    /// Expired entries are swept at most once per window, so a burst of
    /// distinct subjects costs a bounded amount of work per refusal.
    next_sweep: Option<Instant>,
    #[cfg(test)]
    sweeps: u32,
}

impl RefusalLogLimiter {
    /// `Some(n)` when a line should be logged, with `n` refusals counted since
    /// the previous line of this kind and subject; `None` when it is only counted.
    /// A subject that does not fit in the table is counted in the shared overflow
    /// window, whose line carries the details of whichever refusal opened it.
    pub(crate) fn admit(&mut self, refusal: &TokenRefusal, now: Instant) -> Option<u64> {
        let key = refusal.key();
        if !self.seen.contains_key(&key) && self.seen.len() >= REFUSAL_LOG_CAPACITY {
            self.sweep(now);
            if self.seen.len() >= REFUSAL_LOG_CAPACITY {
                return RefusalWindow::admit(&mut self.overflow, now);
            }
        }
        let mut window = self.seen.get(&key).copied();
        let verdict = RefusalWindow::admit(&mut window, now);
        if let Some(window) = window {
            self.seen.insert(key, window);
        }
        verdict
    }

    fn sweep(&mut self, now: Instant) {
        if self.next_sweep.is_some_and(|due| now < due) {
            return;
        }
        self.next_sweep = Some(now + REFUSAL_LOG_WINDOW);
        #[cfg(test)]
        {
            self.sweeps += 1;
        }
        self.seen
            .retain(|_, window| now.duration_since(window.since) < REFUSAL_LOG_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(claims: &Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
        )
    }

    #[test]
    fn subject_is_read_without_the_token_and_hashed() {
        let raw = token(&json!({
            "sub": "client-1234", "subject_kind": "client", "exp": 1_000,
        }));
        let subject = UnverifiedSubject::read(&raw, 1_600_000);
        assert_eq!(subject.subject_kind.as_deref(), Some("client"));
        assert_eq!(subject.expired_for_s, Some(600));
        let hash = subject.subject_hash.unwrap();
        assert_eq!(hash.len(), 8);
        assert!(!raw.contains(&hash) && !hash.contains("client"));
    }

    #[test]
    fn unreadable_token_yields_no_subject() {
        let subject = UnverifiedSubject::read("not-a-token", 5);
        assert_eq!(
            subject,
            UnverifiedSubject {
                subject_kind: None,
                subject_hash: None,
                expired_for_s: None,
            }
        );
    }

    #[test]
    fn refusal_prints_the_text_clients_match_on() {
        let refusal = TokenRefusal::expired(RelayAction::ClientConnect, "x", 0);
        assert_eq!(refusal.to_string(), "relay token is expired");
        assert_eq!(refusal.client_kind(), "client");
    }

    #[test]
    fn one_line_per_kind_and_subject_per_window_then_a_count() {
        let sub = |name: &str| {
            TokenRefusal::expired(
                RelayAction::ClientConnect,
                &token(&json!({"sub": name, "exp": 1})),
                10_000,
            )
        };
        let mut limiter = RefusalLogLimiter::default();
        let start = Instant::now();
        assert_eq!(limiter.admit(&sub("a"), start), Some(0));
        for second in 1..50 {
            assert_eq!(
                limiter.admit(&sub("a"), start + Duration::from_secs(second)),
                None
            );
        }
        assert_eq!(limiter.admit(&sub("b"), start), Some(0));
        assert_eq!(
            limiter.admit(&sub("a"), start + REFUSAL_LOG_WINDOW),
            Some(50)
        );
    }

    #[test]
    fn a_burst_of_distinct_subjects_keeps_the_table_and_the_work_bounded() {
        let sub = |name: &str| {
            TokenRefusal::expired(
                RelayAction::ClientConnect,
                &token(&json!({"sub": name, "exp": 1})),
                10_000,
            )
        };
        let mut limiter = RefusalLogLimiter::default();
        let start = Instant::now();
        let mut logged = 0;
        for index in 0..REFUSAL_LOG_CAPACITY * 3 {
            let at = start + Duration::from_millis(index as u64);
            logged += usize::from(limiter.admit(&sub(&format!("s{index}")), at).is_some());
        }
        assert!(limiter.seen.len() <= REFUSAL_LOG_CAPACITY);
        assert_eq!(
            logged,
            REFUSAL_LOG_CAPACITY + 1,
            "table subjects plus one overflow line"
        );
        assert!(limiter.sweeps <= 1, "swept {} times", limiter.sweeps);
        // A known subject stays rate limited; an unknown one shares the overflow line.
        assert_eq!(
            limiter.admit(&sub("s0"), start + Duration::from_secs(5)),
            None
        );
        assert_eq!(
            limiter.admit(&sub("late"), start + Duration::from_secs(5)),
            None
        );
        // After the window the table is swept and new subjects fit again.
        let later = start + REFUSAL_LOG_WINDOW * 2;
        assert_eq!(limiter.admit(&sub("late"), later), Some(0));
        assert!(limiter.seen.len() < REFUSAL_LOG_CAPACITY);
        assert_eq!(limiter.admit(&sub("s0"), later), Some(0));
    }
}
