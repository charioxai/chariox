//! Keeps alive the attachments a gateway opened on a kernel for its agent.
//!
//! A terminal-class attachment on the target lapses after about 30 s without a
//! heartbeat. The agent has no terminal connection to heartbeat with, so this
//! kernel does it for as long as the agent's grant is live and the kernel
//! answers. The attachments end when the grant ends, the kernel stops
//! answering, or the agent detaches.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::{KernelGateway, Origin};
use crate::local::{GatewayAttachmentRef, HeartbeatAttachmentsRequest, LocalDaemonRequest};

const HEARTBEAT_EVERY: Duration = Duration::from_secs(10);
const HEARTBEAT_DEADLINE: Duration = Duration::from_secs(10);
/// Three missed beats: the target has swept the attachments by then anyway.
const MISSES_BEFORE_GIVING_UP: u32 = 3;
static HEARTBEAT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Hash, PartialEq, Eq, Clone)]
struct Held {
    target: String,
    grant_id: String,
}

struct Attachments {
    harness: String,
    items: Vec<GatewayAttachmentRef>,
}

#[derive(Default)]
pub(super) struct Keeper {
    held: Mutex<HashMap<Held, Attachments>>,
}

impl Keeper {
    /// Adds `attachment` for `held`; true when the caller must start the beat.
    fn add(&self, held: &Held, harness: &str, attachment: GatewayAttachmentRef) -> bool {
        let mut all = self.held.lock().expect("keeper poisoned");
        let start = !all.contains_key(held);
        let entry = all.entry(held.clone()).or_insert_with(|| Attachments {
            harness: harness.to_string(),
            items: Vec::new(),
        });
        if !entry.items.contains(&attachment) {
            entry.items.push(attachment);
        }
        start
    }

    fn remove(&self, held: &Held, attachment_id: &str) {
        let mut all = self.held.lock().expect("keeper poisoned");
        if let Some(entry) = all.get_mut(held) {
            entry
                .items
                .retain(|item| item.attachment_id != attachment_id);
        }
    }

    /// What to beat for, or None (and the entry gone) when nothing is left.
    fn due(&self, held: &Held) -> Option<(String, Vec<GatewayAttachmentRef>)> {
        let mut all = self.held.lock().expect("keeper poisoned");
        match all.get(held) {
            Some(entry) if !entry.items.is_empty() => {
                Some((entry.harness.clone(), entry.items.clone()))
            }
            _ => {
                all.remove(held);
                None
            }
        }
    }

    fn give_up(&self, held: &Held) {
        self.held.lock().expect("keeper poisoned").remove(held);
    }
}

impl KernelGateway {
    /// Remembers what a forwarded request opened or closed on `kernel`.
    pub(crate) fn note_forwarded(
        self: &Arc<Self>,
        target: &str,
        origin: &Origin,
        request: &LocalDaemonRequest,
        response: &Value,
    ) {
        let held = Held {
            target: target.to_string(),
            grant_id: origin.grant_id.clone(),
        };
        match request {
            LocalDaemonRequest::DetachFromSession(detach) => {
                self.keeper.remove(&held, &detach.attachment_id);
            }
            LocalDaemonRequest::AttachToSession(_) | LocalDaemonRequest::AttachSessionView(_) => {
                let Some(attachment) = attached(response) else {
                    return;
                };
                if self.keeper.add(&held, &origin.harness, attachment) {
                    tokio::spawn(Arc::clone(self).beat(held));
                }
            }
            _ => {}
        }
    }

    async fn beat(self: Arc<Self>, held: Held) {
        let mut misses = 0;
        loop {
            tokio::time::sleep(HEARTBEAT_EVERY).await;
            if self
                .runtime_state
                .access_grant_summary(&held.grant_id)
                .is_none()
            {
                self.keeper.give_up(&held);
                return;
            }
            // `due` removes the entry when nothing is left, under the same lock a new
            // attachment is added with, so a later one starts a beat of its own.
            let Some((harness, attachments)) = self.keeper.due(&held) else {
                return;
            };
            let origin = Origin {
                grant_id: held.grant_id.clone(),
                harness,
            };
            let command_id = format!(
                "gw-heartbeat-{}",
                HEARTBEAT_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let request = LocalDaemonRequest::HeartbeatAttachments(HeartbeatAttachmentsRequest {
                attachments,
            });
            match self
                .forward(
                    &held.target,
                    &origin,
                    &command_id,
                    &request,
                    HEARTBEAT_DEADLINE,
                )
                .await
            {
                Ok(answer) => {
                    misses = 0;
                    for lapsed in lapsed(&answer) {
                        self.keeper.remove(&held, &lapsed);
                    }
                }
                Err(_) => {
                    misses += 1;
                    if misses >= MISSES_BEFORE_GIVING_UP {
                        self.keeper.give_up(&held);
                        return;
                    }
                }
            }
        }
    }
}

/// The attachment an `AttachToSession` answer opened.
fn attached(response: &Value) -> Option<GatewayAttachmentRef> {
    let attachment = response
        .get("SessionAttached")
        .or_else(|| response.get("SessionViewAttached"))?
        .get("attachment")?;
    Some(GatewayAttachmentRef {
        session_id: attachment.get("session_id")?.as_str()?.to_string(),
        attachment_id: attachment.get("id")?.as_str()?.to_string(),
    })
}

fn lapsed(response: &Value) -> Vec<String> {
    response
        .get("AttachmentsHeartbeated")
        .and_then(|answer| answer.get("lapsed"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held() -> Held {
        Held {
            target: "b3".into(),
            grant_id: "grant-1".into(),
        }
    }

    fn attachment(id: &str) -> GatewayAttachmentRef {
        GatewayAttachmentRef {
            session_id: "s1".into(),
            attachment_id: id.into(),
        }
    }

    // EA2-05: the first attachment starts the beat, later ones join it, and an
    // emptied set ends it.
    #[test]
    fn the_first_attachment_starts_the_beat_and_an_empty_set_ends_it() {
        let keeper = Keeper::default();
        assert!(keeper.add(&held(), "claude", attachment("a1")));
        assert!(!keeper.add(&held(), "claude", attachment("a2")));
        assert!(!keeper.add(&held(), "claude", attachment("a2")));
        assert_eq!(keeper.due(&held()).unwrap().1.len(), 2);
        keeper.remove(&held(), "a1");
        keeper.remove(&held(), "a2");
        assert!(keeper.due(&held()).is_none());
        assert!(keeper.add(&held(), "claude", attachment("a3")));
    }

    #[test]
    fn it_reads_the_attachment_of_an_attach_answer_and_the_lapsed_ids_of_a_beat() {
        let answer = serde_json::json!({"SessionAttached": {"attachment": {
            "id": "att-1", "session_id": "s1", "client_id": "c", "capability_level": "full"}}});
        assert_eq!(attached(&answer), Some(attachment("att-1")));
        assert_eq!(attached(&serde_json::json!({"SessionDeleted": {}})), None);
        let beat = serde_json::json!({"AttachmentsHeartbeated": {"lapsed": ["att-2"]}});
        assert_eq!(lapsed(&beat), vec!["att-2".to_string()]);
    }
}
