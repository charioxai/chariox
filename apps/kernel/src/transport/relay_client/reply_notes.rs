//! What the connection writer needs to know about a reply beside its envelope: which request
//! kind and which client it answers. The notes name the requester in the write diagnostics,
//! feed the traffic counters of the last hour, and carry the per-client bound on queued bulk replies.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chariox_relay::protocol::RelayEnvelope;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Bulk replies (bigger than the priority lane carries) one client may have queued or being
/// written at once; its later replies wait, so one client cannot fill the shared event lane
/// ahead of everyone else's events.
const BULK_REPLIES_PER_CLIENT: usize = 4;
const USAGE_WINDOW: Duration = Duration::from_secs(60 * 60);
const USAGE_LOGGED_REQUESTERS: usize = 3;

/// Who a reply answers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ReplyOrigin {
    pub(crate) request_kind: &'static str,
    /// The client's relay public key; the bound on bulk replies is per key.
    client_key: String,
    /// Eight characters of the key's thumbprint, for logs.
    pub(crate) client: String,
}

impl ReplyOrigin {
    pub(crate) fn new(request_kind: &'static str, client_public_key: &str) -> Self {
        let thumbprint =
            crate::runtime::terminal_pairings::public_key_thumbprint(client_public_key);
        Self {
            request_kind,
            client_key: client_public_key.to_string(),
            client: thumbprint.chars().take(8).collect(),
        }
    }
}

/// A reply waiting in a lane. The bulk slot it holds is released when the writer is done with it.
pub(crate) struct QueuedReply {
    pub(crate) origin: ReplyOrigin,
    _bulk_slot: Option<OwnedSemaphorePermit>,
}

#[derive(Default)]
pub(crate) struct ReplyNotes {
    queued: Mutex<HashMap<String, QueuedReply>>,
    bulk_slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    usage: Mutex<ReplyUsage>,
}

impl std::fmt::Debug for ReplyNotes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplyNotes")
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The note of a reply is found by the relay's id of the request it answers.
fn reply_key(envelope: &RelayEnvelope) -> Option<String> {
    match envelope {
        RelayEnvelope::DaemonResponse {
            relay_request_id, ..
        } => Some(format!("client:{relay_request_id}")),
        RelayEnvelope::DaemonIncomingPeerResponse {
            relay_request_id, ..
        } => Some(format!("peer:{relay_request_id}")),
        _ => None,
    }
}

impl ReplyNotes {
    /// Notes `envelope` as the answer to `origin`. A bulk reply waits here while the client
    /// already has `BULK_REPLIES_PER_CLIENT` queued.
    pub(crate) async fn note(
        &self,
        envelope: &RelayEnvelope,
        origin: ReplyOrigin,
        bulk: bool,
    ) -> Option<String> {
        let key = reply_key(envelope)?;
        let bulk_slot = if bulk {
            let slots = {
                let mut slots = locked(&self.bulk_slots);
                // A client with nothing queued holds no entry.
                slots.retain(|_, slots| {
                    Arc::strong_count(slots) > 1
                        || slots.available_permits() < BULK_REPLIES_PER_CLIENT
                });
                Arc::clone(
                    slots
                        .entry(origin.client_key.clone())
                        .or_insert_with(|| Arc::new(Semaphore::new(BULK_REPLIES_PER_CLIENT))),
                )
            };
            slots.acquire_owned().await.ok()
        } else {
            None
        };
        locked(&self.queued).insert(
            key.clone(),
            QueuedReply {
                origin,
                _bulk_slot: bulk_slot,
            },
        );
        Some(key)
    }

    /// The note of `envelope` when it is a reply; the caller holds it until the write is done.
    pub(crate) fn take(&self, envelope: &RelayEnvelope) -> Option<QueuedReply> {
        locked(&self.queued).remove(&reply_key(envelope)?)
    }

    /// A reply never sent (its lane closed) leaves no note behind.
    pub(crate) fn forget(&self, key: &str) {
        drop(locked(&self.queued).remove(key));
    }

    /// One frame went out on `lane`.
    pub(crate) fn record_written(
        &self,
        lane: &'static str,
        reply: Option<&QueuedReply>,
        bytes: usize,
    ) {
        locked(&self.usage).record(
            lane,
            reply.map(|reply| &reply.origin),
            bytes,
            Instant::now(),
        );
    }
}

#[derive(Default, Clone, Copy)]
struct Totals {
    bytes_out: u64,
    replies: u64,
}

struct ReplyUsage {
    window_started: Instant,
    by_requester: HashMap<(&'static str, String), Totals>,
    by_lane: HashMap<&'static str, Totals>,
}

impl Default for ReplyUsage {
    fn default() -> Self {
        Self {
            window_started: Instant::now(),
            by_requester: HashMap::new(),
            by_lane: HashMap::new(),
        }
    }
}

impl ReplyUsage {
    fn record(
        &mut self,
        lane: &'static str,
        origin: Option<&ReplyOrigin>,
        bytes: usize,
        now: Instant,
    ) {
        if now.duration_since(self.window_started) >= USAGE_WINDOW {
            self.log_top_requesters();
            *self = Self {
                window_started: now,
                ..Self::default()
            };
        }
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let lane_totals = self.by_lane.entry(lane).or_default();
        lane_totals.bytes_out += bytes;
        lane_totals.replies += u64::from(origin.is_some());
        if let Some(origin) = origin {
            let totals = self
                .by_requester
                .entry((origin.request_kind, origin.client.clone()))
                .or_default();
            totals.bytes_out += bytes;
            totals.replies += 1;
        }
    }

    fn log_top_requesters(&self) {
        let mut requesters: Vec<_> = self.by_requester.iter().collect();
        requesters.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.bytes_out));
        let top: Vec<_> = requesters
            .into_iter()
            .take(USAGE_LOGGED_REQUESTERS)
            .map(|((request_kind, client), totals)| {
                serde_json::json!({
                    "request_kind": request_kind,
                    "client": client,
                    "bytes_out": totals.bytes_out,
                    "replies": totals.replies,
                })
            })
            .collect();
        let lanes: serde_json::Map<_, _> = self
            .by_lane
            .iter()
            .map(|(lane, totals)| {
                (
                    (*lane).to_string(),
                    serde_json::json!({"bytes_out": totals.bytes_out, "replies": totals.replies}),
                )
            })
            .collect();
        crate::logging::info_with_fields(
            "daemon.relay_client",
            "relay traffic of the last hour",
            serde_json::json!({ "top_requesters": top, "lanes": lanes }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::relay_client::RelayOutgoingSender;

    fn bulk_reply(id: &str) -> RelayEnvelope {
        RelayEnvelope::DaemonResponse {
            relay_request_id: id.into(),
            encrypted_response: Some(chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "daemon".into(),
                nonce: "nonce".into(),
                ciphertext: "x".repeat(64 * 1024 + 1),
            }),
            error: None,
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood, 2026-11-09): one client reading a 46 MB file
    // kept the whole event lane busy with its chunks. At most four of its bulk replies are
    // queued; the rest wait, and another client's reply is not behind them.
    #[tokio::test]
    async fn one_client_holds_at_most_four_bulk_replies() {
        let (sender, _priority, mut events) = RelayOutgoingSender::channel(64);
        let reader = |kind| ReplyOrigin::new(kind, "key-of-the-file-reader");
        let mut senders = Vec::new();
        for index in 0..10 {
            let sender = sender.clone();
            senders.push(tokio::spawn(async move {
                sender
                    .send_reply(
                        bulk_reply(&format!("chunk-{index}")),
                        reader("machine_file.read"),
                    )
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut queued = Vec::new();
        while let Ok(envelope) = events.try_recv() {
            queued.push(envelope);
        }
        assert_eq!(
            queued.len(),
            4,
            "the reader holds four bulk replies, not ten"
        );

        let other = ReplyOrigin::new("session.list", "key-of-another-client");
        assert!(sender.send_reply(bulk_reply("other"), other).await);
        assert!(
            matches!(events.try_recv(), Ok(RelayEnvelope::DaemonResponse { relay_request_id, .. }) if relay_request_id == "other"),
            "another client's reply is not queued behind the reader's"
        );

        // The writer finishes the reader's replies one by one; each frees a slot for the next.
        let mut written = queued.len();
        while written < 10 {
            let done = queued.remove(0);
            drop(sender.notes.take(&done));
            tokio::time::sleep(Duration::from_millis(20)).await;
            while let Ok(envelope) = events.try_recv() {
                queued.push(envelope);
                written += 1;
            }
            assert!(queued.len() <= 4, "never more than four queued");
        }
        for sending in senders {
            assert!(sending.await.unwrap());
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayproof r4, F5): four queued bulk replies hold all four
    // slots. When the writer goes away their envelopes are dropped with its queue, so a fifth
    // reply waiting for a slot would wait for ever, and its request task with it.
    #[tokio::test]
    async fn a_reply_waiting_for_a_slot_settles_when_the_writer_ends() {
        let (sender, _priority, events) = RelayOutgoingSender::channel(64);
        let reader = || ReplyOrigin::new("machine_file.read", "key-of-the-file-reader");
        for index in 0..4 {
            assert!(
                sender
                    .send_reply(bulk_reply(&format!("chunk-{index}")), reader())
                    .await
            );
        }
        let fifth = tokio::spawn({
            let sender = sender.clone();
            async move { sender.send_reply(bulk_reply("chunk-4"), reader()).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!fifth.is_finished(), "the fifth reply waits for a slot");

        drop(events);
        let settled = tokio::time::timeout(Duration::from_secs(2), fifth)
            .await
            .expect("the fifth reply is still waiting for a slot after the writer ended")
            .unwrap();
        assert!(!settled, "nothing was sent: the writer is gone");
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood): the lanes kernel logs who sent how much,
    // top three, once an hour.
    #[test]
    fn the_three_heaviest_requesters_are_logged_once_an_hour() {
        let logs = crate::logging::capture::start();
        let started = Instant::now();
        let mut usage = ReplyUsage {
            window_started: started,
            ..ReplyUsage::default()
        };
        for (kind, key, bytes) in [
            ("machine_file.read", "reader", 350_000),
            ("machine_file.read", "reader", 350_000),
            ("session.list", "tab", 2_000),
            ("history.outline", "tab", 9_000),
            ("agent.state", "cli", 500),
            ("room.list", "other", 100),
        ] {
            usage.record("event", Some(&ReplyOrigin::new(kind, key)), bytes, started);
        }
        let quiet = logs
            .records()
            .iter()
            .filter(|record| record.contains("relay traffic of the last hour"))
            .count();
        usage.record("priority", None, 10, started + USAGE_WINDOW);
        let hourly: Vec<_> = logs
            .records()
            .into_iter()
            .filter(|record| record.contains("relay traffic of the last hour"))
            .collect();
        assert_eq!(hourly.len(), quiet + 1, "one line at the end of the hour");
        let line = hourly.last().unwrap();
        assert!(line.contains("\"bytes_out\":700000") && line.contains("machine_file.read"));
        assert!(!line.contains("room.list"), "only the top three: {line}");
        assert!(usage.by_requester.is_empty(), "a new hour starts empty");
    }
}
