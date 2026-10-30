//! Byte counters per realm and stream scope. Sizes only: the relay never
//! looks inside a payload, and a scope is just the label a client chose when it
//! subscribed. Memory is bounded by a fixed number of scopes per realm and
//! a fixed number of keys overall; whatever does not fit is counted under
//! `other`, never dropped.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Requests, replies, presence and everything not tied to a subscription.
pub const CONTROL_SCOPE: &str = "control";
/// The scope of a subscription that named none.
pub const SESSION_SCOPE: &str = "session";
/// Bytes of the display tunnel (`/display/...`).
pub const DISPLAY_TUNNEL_SCOPE: &str = "display_tunnel";
/// Scopes that do not fit the label rules or the per-realm limit.
pub const OTHER_SCOPE: &str = "other";
const OVERFLOW_REALM: &str = "_overflow";
const MAX_SCOPE_LEN: usize = 48;
const MAX_SCOPES_PER_REALM: usize = 16;
const MAX_KEYS: usize = 8192;

/// Envelope kinds that carry display tunnel payload (not its registration).
pub fn is_display_tunnel_data_kind(kind: &str) -> bool {
    matches!(
        kind,
        "daemon_display_tunnel_open"
            | "daemon_display_tunnel_response_start"
            | "daemon_display_tunnel_chunk"
            | "daemon_display_tunnel_client_chunk"
            | "daemon_display_tunnel_client_close"
            | "daemon_display_tunnel_close"
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCounters {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub frames_in: u64,
    pub frames_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Received by the relay from a peer.
    In,
    /// Handed by the relay to a peer.
    Out,
}

impl UsageCounters {
    fn add(&mut self, direction: Direction, bytes: u64) {
        let (total, frames) = match direction {
            Direction::In => (&mut self.bytes_in, &mut self.frames_in),
            Direction::Out => (&mut self.bytes_out, &mut self.frames_out),
        };
        *total = total.saturating_add(bytes);
        *frames = frames.saturating_add(1);
    }

    fn merge(&mut self, other: &Self) {
        self.bytes_in = self.bytes_in.saturating_add(other.bytes_in);
        self.bytes_out = self.bytes_out.saturating_add(other.bytes_out);
        self.frames_in = self.frames_in.saturating_add(other.frames_in);
        self.frames_out = self.frames_out.saturating_add(other.frames_out);
    }
}

pub type UsageSnapshot = BTreeMap<(String, String), UsageCounters>;

#[derive(Debug, Clone, Default)]
pub struct UsageMeter {
    counters: Arc<Mutex<UsageSnapshot>>,
}

impl UsageMeter {
    pub fn record(&self, realm: &str, scope: &str, direction: Direction, bytes: usize) {
        let mut one = UsageCounters::default();
        one.add(direction, bytes as u64);
        self.add(realm, scope, &one);
    }

    fn add(&self, realm: &str, scope: &str, counters: &UsageCounters) {
        let mut all = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let scope = bounded_scope(&all, realm, scope);
        let key = (realm.to_string(), scope.to_string());
        let key = if all.contains_key(&key) || all.len() < MAX_KEYS {
            key
        } else {
            (OVERFLOW_REALM.to_string(), OTHER_SCOPE.to_string())
        };
        all.entry(key).or_default().merge(counters);
    }

    /// The current counters, left in place.
    pub fn snapshot(&self) -> UsageSnapshot {
        self.counters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Take everything counted so far and start from zero.
    pub fn drain(&self) -> UsageSnapshot {
        std::mem::take(&mut *self.counters.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Add a drained batch back (a report that could not be kept).
    pub fn restore(&self, drained: UsageSnapshot) {
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        for (key, value) in drained {
            counters.entry(key).or_default().merge(&value);
        }
    }
}

/// A label made of `[a-z0-9_.-]`, at most 48 bytes, and not the 17th distinct
/// scope of its realm; anything else is `other`.
fn bounded_scope<'a>(counters: &UsageSnapshot, realm: &str, scope: &'a str) -> &'a str {
    let valid = !scope.is_empty()
        && scope.len() <= MAX_SCOPE_LEN
        && scope.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        });
    if !valid {
        return OTHER_SCOPE;
    }
    if matches!(scope, CONTROL_SCOPE | OTHER_SCOPE)
        || counters.contains_key(&(realm.to_string(), scope.to_string()))
    {
        return scope;
    }
    let known = counters
        .range((realm.to_string(), String::new())..)
        .take_while(|((r, _), _)| r == realm)
        .count();
    if known < MAX_SCOPES_PER_REALM - 2 {
        scope
    } else {
        OTHER_SCOPE
    }
}

#[derive(Debug, Default)]
struct Binding {
    realm: Option<Arc<str>>,
    before_auth: UsageCounters,
}

/// One connection's view of the meter. A connection belongs to no realm until
/// it authenticates; what it exchanged before that (its login frame) is
/// counted as control traffic of the realm it then proves to be in.
#[derive(Debug, Clone, Default)]
pub struct ConnectionMeter {
    meter: Option<UsageMeter>,
    binding: Arc<Mutex<Binding>>,
}

impl ConnectionMeter {
    pub fn new(meter: UsageMeter) -> Self {
        Self {
            meter: Some(meter),
            binding: Arc::default(),
        }
    }

    /// A meter that counts nothing, for connections built outside a server.
    pub fn detached() -> Self {
        Self::default()
    }

    pub fn bind_realm(&self, realm: &str) {
        let Some(meter) = &self.meter else { return };
        let mut binding = self.binding.lock().unwrap_or_else(|e| e.into_inner());
        if binding.realm.is_some() {
            return;
        }
        let pending = std::mem::take(&mut binding.before_auth);
        binding.realm = Some(Arc::from(realm));
        if pending != UsageCounters::default() {
            meter.add(realm, CONTROL_SCOPE, &pending);
        }
    }

    pub fn record(&self, direction: Direction, scope: &str, bytes: usize) {
        let Some(meter) = &self.meter else { return };
        let mut binding = self.binding.lock().unwrap_or_else(|e| e.into_inner());
        match &binding.realm {
            Some(realm) => meter.record(realm, scope, direction, bytes),
            None => binding.before_auth.add(direction, bytes as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(meter: &UsageMeter) -> u64 {
        meter
            .snapshot()
            .values()
            .map(|c| c.bytes_in + c.bytes_out)
            .sum()
    }

    #[test]
    fn scope_labels_are_bounded_and_nothing_is_dropped() {
        let meter = UsageMeter::default();
        for index in 0..40 {
            meter.record("realm", &format!("scope-{index}"), Direction::In, 10);
        }
        meter.record("realm", "Not A Label!", Direction::In, 10);
        meter.record("realm", &"x".repeat(MAX_SCOPE_LEN + 1), Direction::Out, 10);
        meter.record("realm", CONTROL_SCOPE, Direction::In, 10);
        let counted = meter.snapshot();
        assert!(
            counted.len() <= MAX_SCOPES_PER_REALM,
            "{} scopes",
            counted.len()
        );
        assert!(counted.contains_key(&("realm".to_string(), CONTROL_SCOPE.to_string())));
        assert_eq!(total(&meter), 43 * 10, "every byte stays counted");
    }

    #[test]
    fn total_keys_are_bounded_and_overflow_is_still_counted() {
        let meter = UsageMeter::default();
        for index in 0..MAX_KEYS + 50 {
            meter.record(&format!("realm-{index}"), CONTROL_SCOPE, Direction::Out, 1);
        }
        assert!(meter.snapshot().len() <= MAX_KEYS + 1);
        assert_eq!(total(&meter), (MAX_KEYS + 50) as u64);
    }

    #[test]
    fn a_connection_counts_its_login_frame_for_the_realm_it_proves_to_be_in() {
        let meter = UsageMeter::default();
        let connection = ConnectionMeter::new(meter.clone());
        connection.record(Direction::In, "session", 100);
        assert!(
            meter.snapshot().is_empty(),
            "no realm is known before login"
        );
        connection.bind_realm("realm");
        connection.record(Direction::Out, "session", 5);
        let counted = meter.snapshot();
        let control = counted[&("realm".to_string(), CONTROL_SCOPE.to_string())];
        assert_eq!((control.bytes_in, control.frames_in), (100, 1));
        assert_eq!(
            counted[&("realm".to_string(), "session".to_string())].bytes_out,
            5
        );
        connection.bind_realm("another");
        connection.record(Direction::In, "session", 1);
        assert!(counted.keys().all(|(realm, _)| realm == "realm"));
        assert!(!meter.snapshot().keys().any(|(realm, _)| realm == "another"));
    }

    #[test]
    fn drain_hands_everything_over_once_and_restore_gives_it_back() {
        let meter = UsageMeter::default();
        meter.record("realm", "session", Direction::In, 9);
        let drained = meter.drain();
        assert!(meter.drain().is_empty());
        meter.record("realm", "session", Direction::In, 1);
        meter.restore(drained);
        assert_eq!(total(&meter), 10);
    }
}
