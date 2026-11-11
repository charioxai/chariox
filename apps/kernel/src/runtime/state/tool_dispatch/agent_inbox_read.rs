//! The inbox tools: what an agent reads after a short wake, and how it acks.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, InboxEvent, Operation};
use crate::runtime::state::agent_inbox_sender::SenderRelation;
use std::ops::ControlFlow;

/// Events one read returns unless the agent asks for another count.
const READ_LIMIT_DEFAULT: usize = 16;
const READ_LIMIT_MAX: usize = 64;
/// A payload longer than this is cut and flagged `truncated`.
pub(super) const PAYLOAD_LIMIT: usize = 16 * 1024;
const SUMMARY_LIMIT: usize = 160;
const READING_NOTICE: &str = "Each event says who sent it (source.relation) and what that means for you (guidance). Follow the guidance of each event.";

/// The payload of one event within the read bound, and whether it was cut.
pub(in crate::runtime::state) fn bounded_payload(
    payload: &serde_json::Value,
) -> (serde_json::Value, bool) {
    let text = payload.to_string();
    if text.len() <= PAYLOAD_LIMIT {
        return (payload.clone(), false);
    }
    let mut end = PAYLOAD_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (serde_json::Value::String(text[..end].to_owned()), true)
}

impl KernelRuntimeState {
    pub(crate) fn read_agent_inbox(
        &self,
        room: &str,
        actor: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let after = args["after"].as_u64().unwrap_or(0);
        let limit = args["limit"].as_u64().map_or(READ_LIMIT_DEFAULT, |n| {
            (n as usize).clamp(1, READ_LIMIT_MAX)
        });
        let work = self.owned.sudo_work_task(room, actor);
        let store = &self.owned.durable_state_store;
        // `after` only pages: a row at or below it that was never shown is returned again.
        let mut open = store.agent_unread_inbox(room, actor, 0, after, work.as_deref())?;
        let unread_full = open.len() >= ledger::INBOX_PAGE;
        store.scan_agent_inbox(room, actor, after, work.as_deref(), |event| {
            if ledger::is_open(&event) {
                open.push(event);
            }
            if open.len() > limit {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        let more = open.len() > limit || unread_full;
        let shown: Vec<&InboxEvent> = open.iter().take(limit).collect();
        if !shown.is_empty() {
            store.agent_lifecycle(Operation::MarkShown {
                room: room.into(),
                agent: actor.into(),
                sequences: shown.iter().map(|event| event.sequence).collect(),
            })?;
        }
        let reader = self.owned.agent_store.get_agent(actor)?;
        let agents = self.owned.agent_store.get_session_agents(room);
        let views: Vec<(SenderRelation, serde_json::Value)> = shown
            .iter()
            .map(|event| self.inbox_event_view(room, &reader, &agents, event))
            .collect();
        let untrusted = views.iter().any(|(relation, _)| !relation.is_instruction());
        let events: Vec<serde_json::Value> = views.into_iter().map(|(_, view)| view).collect();
        let next_after = shown.last().map_or(after, |event| event.sequence);
        Ok(serde_json::json!({
            "untrusted": untrusted,
            "notice": READING_NOTICE,
            "next_after": next_after,
            "more": more,
            "events": events,
        }))
    }

    fn inbox_event_view(
        &self,
        room: &str,
        reader: &crate::agent::AgentInstance,
        agents: &[crate::agent::AgentInstance],
        event: &InboxEvent,
    ) -> (SenderRelation, serde_json::Value) {
        let agent = self.owned.agent_store.get_agent(&event.source_id).ok();
        let session = self.owned.session_store.get_session(room).ok();
        let is_run = session.is_some_and(|session| {
            session
                .workflow_runs()
                .iter()
                .any(|run| run.id() == event.source_id)
        });
        // A followed peer on another kernel is named by where it runs there.
        let remote = agent
            .is_none()
            .then(|| match event.kind.as_str() {
                "peer_event" => super::super::peer_source_stamp(&event.source_id),
                "message" => super::super::peer_message_stamp(&event.source_id, &event.payload),
                _ => None,
            })
            .flatten();
        let source_kind = if agent.is_some() || remote.is_some() {
            "agent"
        } else if is_run {
            "workflow_run"
        } else {
            "source"
        };
        let alias = agent
            .as_ref()
            .map(|agent| agent.alias().unwrap_or(agent.agent_ref()).to_owned());
        let remote_name = remote
            .as_ref()
            .map(|stamp| format!("{}/{}", stamp.kernel_id, stamp.agent_id));
        let name = alias
            .as_deref()
            .or(remote_name.as_deref())
            .unwrap_or(&event.source_id);
        let relation = if remote.is_some() {
            SenderRelation::Peer
        } else {
            SenderRelation::of(reader, agent.as_ref(), agents)
        };
        let what = super::super::agent_inbox_wake_text::kind_label(&event.kind);
        let mut summary = format!("{what} from {name}");
        summary.truncate(summary.floor_char_boundary(SUMMARY_LIMIT));
        let mut payload = event.payload.clone();
        let attachments = payload
            .as_object_mut()
            .and_then(|fields| fields.remove("attachments"))
            .unwrap_or_else(|| serde_json::json!([]));
        let (payload, truncated) = bounded_payload(&payload);
        // The kernel stamps an agent sender with where it runs; a peer may be in another room.
        let (kernel_id, sender_room) = match &agent {
            Some(agent) => (
                Some(self.owned.config_projection.snapshot().daemon_id),
                Some(agent.session_id().to_owned()),
            ),
            None => match &remote {
                Some(stamp) => (Some(stamp.kernel_id.clone()), Some(stamp.room_id.clone())),
                None => (None, None),
            },
        };
        let view = serde_json::json!({
            "sequence": event.sequence,
            "kind": event.kind,
            "urgent": event.urgent,
            "needs_answer": ledger::needs_answer(event),
            "reply_requested": event.reply_requested,
            "source": {
                "kind": source_kind,
                "id": event.source_id,
                "alias": alias,
                "kernel_id": kernel_id,
                "room": sender_room,
                "relation": relation.name(),
            },
            "guidance": relation.guidance(name),
            "summary": summary,
            "payload": payload,
            "truncated": truncated,
            "attachments": attachments,
        });
        (relation, view)
    }

    /// A handled `child_finished` receipt also ends the sub-agent it reports, so the
    /// parent has no separate delete step to forget.
    pub(crate) async fn ack_agent_inbox(
        &self,
        room: &str,
        actor: &str,
        args: &serde_json::Value,
        now: u64,
    ) -> Result<serde_json::Value, DaemonError> {
        let handled = args["handled"].as_bool().unwrap_or(false);
        let work = self.owned.sudo_work_task(room, actor);
        let mut finished_children = Vec::new();
        let mut ack = |event: &InboxEvent| {
            let subtree = if handled && event.kind == "child_finished" {
                self.finished_child_subtree(room, actor, &event.payload)?
            } else {
                None
            };
            // The deletion is recorded in the write that handles the receipt.
            let operation = match subtree.clone() {
                Some(subtree) => Operation::AckHandledChild {
                    room: room.into(),
                    agent: actor.into(),
                    sequence: event.sequence,
                    subtree,
                    now,
                },
                None => Operation::Ack {
                    room: room.into(),
                    agent: actor.into(),
                    sequence: event.sequence,
                    handled,
                    now,
                },
            };
            self.owned.durable_state_store.agent_lifecycle(operation)?;
            finished_children.extend(subtree);
            Ok::<_, DaemonError>(())
        };
        if let Some(through) = args["through"].as_u64() {
            let (mut acknowledged, mut skipped) = (Vec::new(), Vec::new());
            let mut cursor = 0;
            loop {
                let page = self.owned.durable_state_store.agent_work_inbox(
                    room,
                    actor,
                    cursor,
                    work.as_deref(),
                )?;
                let Some(last) = page.last().map(|event| event.sequence) else {
                    break;
                };
                // A row the agent was never shown stays as it is, whatever sequence the ack names.
                let unread: std::collections::HashSet<u64> = self
                    .owned
                    .durable_state_store
                    .agent_unread_inbox(room, actor, cursor, last, work.as_deref())?
                    .into_iter()
                    .map(|event| event.sequence)
                    .collect();
                for event in page
                    .iter()
                    .filter(|event| event.sequence <= through && ledger::is_open(event))
                {
                    if unread.contains(&event.sequence) {
                        skipped.push(
                            serde_json::json!({"sequence": event.sequence, "reason": "not_read"}),
                        );
                    } else if ledger::needs_answer(event) {
                        skipped.push(serde_json::json!({"sequence": event.sequence, "reason": "needs_answer"}));
                    } else if matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked")
                    {
                        skipped.push(serde_json::json!({"sequence": event.sequence, "reason": "delivery_unsettled"}));
                    } else {
                        ack(event)?;
                        acknowledged.push(event.sequence);
                    }
                }
                if last >= through {
                    break;
                }
                cursor = last;
            }
            self.end_handled_children(&finished_children).await;
            return Ok(serde_json::json!({"acknowledged": acknowledged, "skipped": skipped}));
        }
        let seq = args["sequence"]
            .as_u64()
            .ok_or_else(|| ledger::error("sequence or through required"))?;
        let event = self
            .owned
            .durable_state_store
            .agent_work_inbox(room, actor, seq.saturating_sub(1), work.as_deref())?
            .into_iter()
            .find(|event| event.sequence == seq)
            .ok_or_else(|| {
                ledger::error(if work.is_some() {
                    "event is deferred outside this sudo work"
                } else {
                    "event unavailable in this inbox"
                })
            })?;
        if ledger::needs_answer(&event) {
            return Err(ledger::error(
                "this event needs an answer: answer or pass the decision first, then it can be acknowledged",
            ));
        }
        ack(&event)?;
        self.end_handled_children(&finished_children).await;
        Ok(serde_json::json!({"acknowledged": seq}))
    }
}
