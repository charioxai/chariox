//! The wake prompt carries the text of the events it wakes the agent for, so an
//! agent that can act on them does not open its inbox first. What does not fit
//! stays in the inbox behind the counts-and-kinds pointer.
use super::agent_inbox_wake_text::{kind_label, wake_text};
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, InboxEvent};

/// One event's quoted text never exceeds this many characters.
const BODY_LIMIT: usize = 2_000;
/// All quoted text of one wake never exceeds this many characters.
const TEXT_LIMIT: usize = 6_000;
const REPLY_RULE: &str =
    "If this request started your turn, your final answer goes back to the sender.";

pub(super) struct InlineWake {
    pub text: String,
    /// The events whose text is in `text`: the agent was shown them.
    pub shown: Vec<u64>,
}

/// What the agent reads of an event: a message is its text, any other event its payload.
fn body(event: &InboxEvent) -> String {
    if event.kind == "message" {
        if let Some(message) = event.payload["message"].as_str() {
            return message.to_owned();
        }
    }
    let mut payload = event.payload.clone();
    if let Some(fields) = payload.as_object_mut() {
        fields.remove("attachments");
    }
    payload.to_string()
}

fn quoted(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl KernelRuntimeState {
    /// `after` is the sequence the agent's inbox read starts behind.
    pub(super) fn inline_wake(
        &self,
        room: &str,
        agent: &str,
        lead: &InboxEvent,
        batch: &[InboxEvent],
        after: u64,
    ) -> Result<InlineWake, DaemonError> {
        let reader = self.owned.agent_store.get_agent(agent)?;
        let agents = self.owned.agent_store.get_session_agents(room);
        let events: Vec<&InboxEvent> = std::iter::once(lead).chain(batch).collect();
        let mut blocks = Vec::new();
        let mut used = 0;
        for event in &events {
            let text = body(event);
            let sender = self.event_sender(room, &reader, &agents, event);
            let mut block = format!(
                "Event {} ({}). {}",
                event.sequence,
                kind_label(&event.kind),
                sender.relation.guidance(&sender.name)
            );
            if event.kind == "message" && !event.reply_requested {
                if let Some(route) = sender.reply_route(room, event) {
                    block.push_str(&format!(
                        " To answer it, call chariox.send_agent_message with {route}."
                    ));
                }
            }
            if ledger::needs_answer(event) {
                block.push_str(" It stays open until you answer or pass the decision.");
            } else if event.is_loss() {
                block.push_str(&format!(
                    " It stays open until you settle it: chariox.events.ack with sequence {}, handled true once you dealt with it.",
                    event.sequence
                ));
            }
            block.push('\n');
            block.push_str(&quoted(&text));
            if text.chars().count() > BODY_LIMIT || used + block.chars().count() > TEXT_LIMIT {
                break;
            }
            used += block.chars().count();
            blocks.push(block);
        }
        let inlined = blocks.len();
        if inlined == 0 {
            return Ok(InlineWake {
                text: wake_text(lead, batch, after),
                shown: Vec::new(),
            });
        }
        let noun = if inlined == 1 { "event" } else { "events" };
        let mut text = format!(
            "Inbox: {inlined} new {noun}; lines starting with > are quoted from {}.\n\n{}",
            if inlined == 1 { "it" } else { "them" },
            blocks.join("\n\n")
        );
        if lead.reply_requested {
            text.push_str("\n\n");
            text.push_str(REPLY_RULE);
        }
        if let Some((rest_lead, rest)) = events[inlined..].split_first() {
            let rest: Vec<InboxEvent> = rest.iter().map(|event| (*event).clone()).collect();
            text.push_str("\n\n");
            text.push_str(&wake_text(rest_lead, &rest, after));
        }
        Ok(InlineWake {
            text,
            shown: events[..inlined].iter().map(|e| e.sequence).collect(),
        })
    }
}
