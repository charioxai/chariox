//! The inbox tools: what an agent reads after a short wake, and how it acks.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, InboxEvent, Operation};
use crate::runtime::state::agent_inbox_sender::SenderRelation;
use crate::runtime::state::decision_holder::DecisionHolder;
use std::ops::ControlFlow;

/// Events one read returns unless the agent asks for another count.
const READ_LIMIT_DEFAULT: usize = 16;
const READ_LIMIT_MAX: usize = 64;
/// One read returns at most this many bytes of a payload; the rest is read with `offset`.
pub(super) const PAYLOAD_LIMIT: usize = 16 * 1024;
const SUMMARY_LIMIT: usize = 160;
const READING_NOTICE: &str = "Each event says who sent it (source.relation) and what that means for you (guidance). Follow the guidance of each event.";

/// One window of an event's payload: the whole payload when it fits, otherwise the next
/// `PAYLOAD_LIMIT` bytes of its JSON text from `offset`.
pub(in crate::runtime::state) struct PayloadWindow {
    pub(in crate::runtime::state) value: serde_json::Value,
    /// Bytes of the whole payload's JSON text.
    pub(in crate::runtime::state) total: usize,
    /// Where the next window starts; `None` when this one reaches the end.
    pub(in crate::runtime::state) next_offset: Option<usize>,
}

pub(in crate::runtime::state) fn payload_window(
    payload: &serde_json::Value,
    offset: usize,
) -> Result<PayloadWindow, DaemonError> {
    let text = payload.to_string();
    if offset == 0 && text.len() <= PAYLOAD_LIMIT {
        return Ok(PayloadWindow {
            value: payload.clone(),
            total: text.len(),
            next_offset: None,
        });
    }
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(ledger::error(
            "offset is not a position in this payload: use the next_offset of the previous read",
        ));
    }
    let mut end = (offset + PAYLOAD_LIMIT).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(PayloadWindow {
        value: serde_json::Value::String(text[offset..end].to_owned()),
        total: text.len(),
        next_offset: (end < text.len()).then_some(end),
    })
}

/// The sender of an inbox event: what the tree says it is and the name the reader sees.
pub(in crate::runtime::state) struct EventSender {
    agent: Option<crate::agent::AgentInstance>,
    remote: Option<crate::transport::relay_peer::PeerAgentStamp>,
    kind: &'static str,
    alias: Option<String>,
    pub(in crate::runtime::state) name: String,
    pub(in crate::runtime::state) relation: SenderRelation,
}

impl EventSender {
    /// What `chariox.send_agent_message` needs to answer this sender, when its name is not enough:
    /// the sender is in another room or on another kernel.
    pub(in crate::runtime::state) fn reply_route(
        &self,
        room: &str,
        event: &InboxEvent,
    ) -> Option<String> {
        match (&self.agent, &self.remote) {
            (Some(agent), _) if agent.session_id() != room => Some(format!(
                "agent {:?}, room {:?}",
                event.source_id,
                agent.session_id()
            )),
            (None, Some(stamp)) => Some(format!(
                "agent {:?}, room {:?}, kernel {:?}",
                stamp.agent_id, stamp.room_id, stamp.kernel_id
            )),
            _ => None,
        }
    }
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
        if let Some(sequence) = args["sequence"].as_u64() {
            return self.read_inbox_event(room, actor, work.as_deref(), sequence, args);
        }
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
            .map(|event| self.inbox_event_view(room, &reader, &agents, event, 0))
            .collect::<Result<_, _>>()?;
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

    /// One event read by its sequence, whatever its state: the way to read a payload that did
    /// not fit one read, window by window with `offset`.
    fn read_inbox_event(
        &self,
        room: &str,
        actor: &str,
        work: Option<&str>,
        sequence: u64,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let store = &self.owned.durable_state_store;
        let event = store
            .agent_work_inbox(room, actor, sequence.saturating_sub(1), work)?
            .into_iter()
            .find(|event| event.sequence == sequence)
            .ok_or_else(|| ledger::error("event unavailable in this inbox"))?;
        if ledger::is_open(&event) {
            store.agent_lifecycle(Operation::MarkShown {
                room: room.into(),
                agent: actor.into(),
                sequences: vec![sequence],
            })?;
        }
        let reader = self.owned.agent_store.get_agent(actor)?;
        let agents = self.owned.agent_store.get_session_agents(room);
        let (relation, view) = self.inbox_event_view(room, &reader, &agents, &event, offset)?;
        Ok(serde_json::json!({
            "untrusted": !relation.is_instruction(),
            "notice": READING_NOTICE,
            "next_after": args["after"].as_u64().unwrap_or(0),
            "more": false,
            "events": [view],
        }))
    }

    /// A decision event needs an answer only while its card is open and its agent holds it: a card
    /// another holder (or the user) has now, or one that ended without an answer (its asker
    /// stopped, or the kernel that held it stopped), leaves this agent nothing to answer, so its
    /// event can be acknowledged.
    fn decision_open(&self, room: &str, event: &InboxEvent) -> bool {
        if !ledger::needs_answer(event) {
            return false;
        }
        let Some(card) = event.payload["interaction_id"].as_str() else {
            return true;
        };
        if let Some(pending) = self.owned.pending_interactions.write().get(card) {
            return pending.holder == DecisionHolder::Agent(event.agent_id.clone());
        }
        self.owned
            .session_store
            .get_session(room)
            .is_ok_and(|session| {
                session
                    .active_interactions()
                    .iter()
                    .any(|open| open.id() == card)
            })
    }

    /// Who sent `event`, as far as the agent tree and the peer stamps tell.
    pub(in crate::runtime::state) fn event_sender(
        &self,
        room: &str,
        reader: &crate::agent::AgentInstance,
        agents: &[crate::agent::AgentInstance],
        event: &InboxEvent,
    ) -> EventSender {
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
        let kind = if agent.is_some() || remote.is_some() {
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
            .clone()
            .or(remote_name)
            .unwrap_or_else(|| event.source_id.clone());
        let relation = if remote.is_some() {
            SenderRelation::Peer
        } else {
            SenderRelation::of(reader, agent.as_ref(), agents)
        };
        EventSender {
            agent,
            remote,
            kind,
            alias,
            name,
            relation,
        }
    }

    fn inbox_event_view(
        &self,
        room: &str,
        reader: &crate::agent::AgentInstance,
        agents: &[crate::agent::AgentInstance],
        event: &InboxEvent,
        offset: usize,
    ) -> Result<(SenderRelation, serde_json::Value), DaemonError> {
        let EventSender {
            agent,
            remote,
            kind: source_kind,
            alias,
            name,
            relation,
        } = self.event_sender(room, reader, agents, event);
        let name = name.as_str();
        let what = super::super::agent_inbox_wake_text::kind_label(&event.kind);
        let mut summary = format!("{what} from {name}");
        summary.truncate(summary.floor_char_boundary(SUMMARY_LIMIT));
        let mut payload = event.payload.clone();
        let attachments = payload
            .as_object_mut()
            .and_then(|fields| fields.remove("attachments"))
            .unwrap_or_else(|| serde_json::json!([]));
        let window = payload_window(&payload, offset)?;
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
            "needs_answer": self.decision_open(room, event),
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
            "payload": window.value,
            "payload_bytes": window.total,
            "payload_offset": offset,
            "next_offset": window.next_offset,
            "truncated": window.next_offset.is_some(),
            "attachments": attachments,
        });
        Ok((relation, view))
    }

    pub(crate) async fn ack_agent_inbox(
        &self,
        room: &str,
        actor: &str,
        args: &serde_json::Value,
        now: u64,
    ) -> Result<serde_json::Value, DaemonError> {
        let handled = args["handled"].as_bool().unwrap_or(false);
        let work = self.owned.sudo_work_task(room, actor);
        let ack = |event: &InboxEvent| {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Ack {
                    room: room.into(),
                    agent: actor.into(),
                    sequence: event.sequence,
                    handled,
                    now,
                })
                .map(|_| ())
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
                    } else if self.decision_open(room, event) {
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
        if self.decision_open(room, &event) {
            return Err(ledger::error(
                "this event needs an answer: answer or pass the decision first, then it can be acknowledged",
            ));
        }
        ack(&event)?;
        Ok(serde_json::json!({"acknowledged": seq}))
    }
}
