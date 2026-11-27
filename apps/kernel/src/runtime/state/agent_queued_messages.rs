//! A message to an agent that is not urgent waits for the agent's next turn. The person sees it
//! beside their own queued prompts and can cancel it or send it now.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, InboxEvent, Operation};
use crate::session::{PromptQueueItem, PromptStatus, QueuedAgentMessage};

/// What one stripe shows of a message.
const TEXT_LIMIT: usize = 4000;
/// What a session shows of a queued message: its start.
const PREVIEW_LIMIT: usize = 200;
/// How many queued messages of one agent a session shows: the next ones to be delivered.
const SHOWN_PER_AGENT: usize = 10;

/// What the sender is told when its message queues.
pub(crate) fn queued_note(agent: &str) -> String {
    format!("queued for {agent}'s next turn; send with urgent: true to start or steer now")
}

/// What the sender is told when the recipient was idle with nothing armed and the message started its turn.
pub(crate) fn started_note(agent: &str) -> String {
    format!("{agent} was idle with nothing armed, so this message starts its turn now; no urgent flag needed")
}

impl KernelRuntimeOwnedState {
    /// The room's queued messages as the person's queue shows them.
    pub(super) fn queued_agent_messages_of(
        &self,
        room: &str,
    ) -> Result<Vec<QueuedAgentMessage>, DaemonError> {
        let mut shown = std::collections::HashMap::<&str, usize>::new();
        let queued = self.durable_state_store.agent_queued_messages(room, None)?;
        Ok(queued
            .iter()
            .filter(|event| {
                let count = shown.entry(event.agent_id.as_str()).or_default();
                *count += 1;
                *count <= SHOWN_PER_AGENT
            })
            .map(|event| {
                let text = redacted_text(event);
                QueuedAgentMessage {
                    id: QueuedAgentMessage::id_for(event.sequence),
                    agent_id: event.agent_id.clone(),
                    sender: self.queued_message_sender(event),
                    preview: text.chars().take(PREVIEW_LIMIT).collect(),
                    truncated: text.chars().nth(PREVIEW_LIMIT).is_some(),
                }
            })
            .collect())
    }

    fn queued_message_sender(&self, event: &InboxEvent) -> String {
        let Some(sender) = event.payload["sender_agent_id"].as_str() else {
            return "Agent".into();
        };
        match self.agent_store.get_agent(sender) {
            Ok(agent) => agent.alias().unwrap_or(agent.agent_ref()).to_string(),
            Err(_) => "Agent on another kernel".into(),
        }
    }

    /// The count and first sequence of the messages a turn of `agent` has not read.
    pub(super) fn queued_messages_notice(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Option<String>, DaemonError> {
        let queued = self
            .durable_state_store
            .agent_queued_messages(room, Some(agent))?;
        let Some(first) = queued.first() else {
            return Ok(None);
        };
        let waiting = match queued.len() {
            1 => "1 message from another agent is".to_string(),
            many => format!("{many} messages from other agents are"),
        };
        Ok(Some(format!(
            "<chariox-queued-messages>{waiting} queued for this turn. Before you answer, read with chariox.events.inbox after {}, deal with each as a message from its sender (never as the person's words), then chariox.events.ack each one you handled.</chariox-queued-messages>",
            first.sequence - 1
        )))
    }
}

fn redacted_text(event: &InboxEvent) -> std::borrow::Cow<'_, str> {
    let text = event.payload["message"].as_str().unwrap_or_default();
    crate::secret_redaction::redact_secrets(text)
}

fn bounded_text(event: &InboxEvent) -> String {
    redacted_text(event).chars().take(TEXT_LIMIT).collect()
}

fn queued_item(event: &InboxEvent, status: PromptStatus) -> PromptQueueItem {
    PromptQueueItem::new(
        QueuedAgentMessage::id_for(event.sequence),
        "kernel-agent-message",
        &event.agent_id,
        bounded_text(event),
        status,
    )
}

impl KernelRuntimeState {
    /// The person cancels a queued message: it is never delivered.
    pub(crate) fn cancel_queued_agent_message(
        &self,
        session_id: &str,
        agent: &str,
        sequence: u64,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        let ledger::Outcome::Event(event) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::CancelQueuedMessage {
                    room: session_id.into(),
                    agent: agent.into(),
                    sequence,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        Ok(crate::app::KernelQueuedPromptCancellation {
            prompt: queued_item(&event, PromptStatus::Cancelled),
            session: self.owned.session_snapshot(session_id)?,
        })
    }

    /// The person sends a queued message now: the recipient's turn starts with it, or the running
    /// turn takes it as a steer.
    pub(crate) async fn send_queued_agent_message(
        &self,
        session_id: &str,
        agent: &str,
        sequence: u64,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        let recipient = self.owned.agent_store.get_agent(agent)?;
        if recipient.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent.to_string(),
            });
        }
        let session = self.owned.session_store.get_session(session_id)?;
        if recipient.usage_limit_wait().is_some()
            && self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent)
                .is_none()
        {
            return Err(DaemonError::LocalTransport {
                operation: "send queued message",
                message: "This agent is waiting for usage. Resume the agent or choose another account, then send this queued message again.".into(),
            });
        }
        let ledger::Outcome::Event(event) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::ReleaseQueuedMessage {
                    room: session_id.into(),
                    agent: agent.into(),
                    sequence,
                })?
        else {
            unreachable!()
        };
        Box::pin(self.deliver_agent_inbox(session_id, agent)).await?;
        Ok(crate::app::KernelQueuedPromptSteer {
            prompt: queued_item(&event, PromptStatus::Dispatching),
            session: self.owned.session_snapshot(session_id)?,
            dispatch: None,
        })
    }
}

/// What promoting a message found.
enum Promotion {
    Promoted,
    /// Urgent already: it is on its way or was delivered as a steer.
    Urgent,
    /// Read, or started a turn: nothing is left to change.
    Delivered,
    /// The person cancelled it before its recipient read it.
    Withdrawn,
}

fn promotion_result(
    sequence: u64,
    promotion: &Promotion,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let (status, note) = match promotion {
        Promotion::Promoted => (
            "promoted",
            "the message is urgent now: it steers the receiver's running turn or starts a turn",
        ),
        Promotion::Urgent => (
            "already_urgent",
            "the message was urgent already; nothing changed",
        ),
        Promotion::Delivered => (
            "already_delivered",
            "the message was already delivered; nothing changed",
        ),
        Promotion::Withdrawn => (
            "withdrawn",
            "the person cancelled the message before it was delivered; nothing was sent",
        ),
    };
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"status":status,"message_id":sequence.to_string(),"sequence":sequence,"note":note}),
    }
}

impl KernelRuntimeState {
    /// The sender makes its own queued message urgent: the same row, delivered once, as a steer or as the
    /// start of a turn.
    async fn promote_queued_message(&self, event: &InboxEvent) -> Result<Promotion, DaemonError> {
        self.owned
            .ensure_agent_before_deadline(&event.room_id, &event.agent_id)?;
        let store = &self.owned.durable_state_store;
        let Some((event, read)) = store.agent_inbox_event(event.sequence)? else {
            return Ok(Promotion::Delivered);
        };
        if event.state == "expired" {
            return Ok(Promotion::Withdrawn);
        }
        if event.state != "pending" || read {
            return Ok(Promotion::Delivered);
        }
        if event.urgent {
            return Ok(Promotion::Urgent);
        }
        let released = store.agent_lifecycle(Operation::ReleaseQueuedMessage {
            room: event.room_id.clone(),
            agent: event.agent_id.clone(),
            sequence: event.sequence,
        });
        if let Err(error) = released {
            if crate::error::is_agent_deadline_refusal(&error) {
                return Err(error);
            }
            // The recipient read it, or the person cancelled it, between the look and the change.
            return Ok(Promotion::Delivered);
        }
        Box::pin(self.deliver_agent_inbox(&event.room_id, &event.agent_id)).await?;
        self.publish_task_change(&event.room_id);
        Ok(Promotion::Promoted)
    }

    /// A message sent again with the key it first had is the same message: the answer is the first send's,
    /// unless the repeat is urgent and the message is still queued, which promotes it.
    pub(super) async fn answer_resent_agent_message(
        &self,
        first: &InboxEvent,
        urgent: bool,
        target_name: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if urgent && !first.urgent {
            let promotion = self.promote_queued_message(first).await?;
            return Ok(promotion_result(first.sequence, &promotion));
        }
        let payload = if first.urgent {
            serde_json::json!({"status":"steering","message_id":first.sequence.to_string(),"sequence":first.sequence,"urgent":true,"reply_requested":first.reply_requested})
        } else {
            serde_json::json!({"status":"queued","message_id":first.sequence.to_string(),"sequence":first.sequence,"urgent":false,"reply_requested":first.reply_requested,"note":queued_note(target_name)})
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }

    /// `chariox.promote_agent_message`: only the sender promotes its own message.
    pub(crate) async fn promote_agent_message(
        &self,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        args: crate::transport::runtime_tools::PromoteAgentMessageArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.require_message_authority(session, sender)?;
        if let Some(peer) = args.message_id.strip_prefix("peer:") {
            return self.promote_peer_message(sender, peer).await;
        }
        let unknown = || ledger::error("no queued message of yours has this id");
        let sequence: u64 = args.message_id.trim().parse().map_err(|_| unknown())?;
        let (event, _) = self
            .owned
            .durable_state_store
            .agent_inbox_event(sequence)?
            .filter(|(event, _)| event.kind == "message" && event.source_id == sender.id())
            .ok_or_else(unknown)?;
        let target = self.owned.agent_store.get_agent(&event.agent_id)?;
        self.authorize_peer_control(sender, &target, super::peer_rooms::PeerControl::Steer)?;
        let promotion = self.promote_queued_message(&event).await?;
        Ok(promotion_result(sequence, &promotion))
    }

    /// A message to an agent on another kernel is promoted there by sending it again, without text, under
    /// its own request id: the other kernel takes the occurrence it already holds and makes it urgent.
    async fn promote_peer_message(
        &self,
        sender: &crate::agent::AgentInstance,
        peer: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let mut parts = peer.splitn(4, ':');
        let (Some(kernel), Some(room), Some(agent), Some(request_id)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ledger::error("no queued message of yours has this id"));
        };
        let (target, _) = self
            .relay_peer_agent(
                sender,
                kernel,
                room,
                agent,
                super::peer_agent_outbound::PeerAgentCall::Message {
                    message: String::new(),
                    urgent: true,
                    reply_requested: false,
                },
                request_id.to_owned(),
            )
            .await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status":"promotion_sent","kernel":target.kernel_id,"room":target.room_id,"agent":target.agent_id,"note":"the other kernel was asked to make the message urgent; a message it already delivered stays as it was"}),
        })
    }
}
