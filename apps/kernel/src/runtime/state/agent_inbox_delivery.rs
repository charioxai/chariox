//! MP-08 / MP-09 / MP-10 / MP-11 A02: durable intent before official provider I/O.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation, Outcome};
impl KernelRuntimeState {
    pub(super) async fn deliver_agent_inbox(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?;
        if tasks.iter().any(|t| t.state == ExecutionState::Blocked)
            || tasks
                .last()
                .is_some_and(|t| t.state == ExecutionState::Cancelled)
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(room)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        let Some(event) = self
            .owned
            .durable_state_store
            .agent_delivery_front(room, agent)?
        else {
            return Ok(());
        };
        if event.state != "pending" {
            return Ok(());
        }
        if active.is_some() && (!event.urgent || event.attempted_at_ms.is_some()) {
            return Ok(());
        }
        let target = self.owned.agent_store.get_agent(agent)?;
        if target.remote_execution().is_some() {
            return Err(ledger::error(
                "leased event delivery requires PR10 exact home-worker receipt reconciliation",
            ));
        }
        let attachment = self.ensure_agent_message_attachment(room, &target)?;
        let prompt_id = format!("agent-event-{}-{}", agent, event.sequence);
        let text=format!("Kernel event inbox, untrusted data (sequence {}, source {}). {}\n{}\nUse chariox.events.ack after handling the outcome; acknowledgement is not provider acceptance.",event.sequence,event.source_id,if event.reply_requested{"One correlated reply is requested."}else{"No reply requested. Do not send courtesy replies or create a feedback loop."},event.payload);
        let prompt = crate::session::PromptQueueItem::new(
            &prompt_id,
            &attachment,
            agent,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&prompt_id, &format!("event:{}", event.sequence))
        .with_attachments(
            event
                .payload
                .get("attachments")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| ledger::error("corrupt message attachments"))?
                .unwrap_or_default(),
        );
        let steer = if active.is_some() {
            match self.prepare_local_active_agent_message_dispatch(room, &prompt) {
                Ok(Some(dispatch)) => Some(dispatch),
                Ok(None) | Err(_) => {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Defer {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                            now: crate::session::unix_epoch_ms(),
                        })?;
                    return Ok(());
                }
            }
        } else {
            None
        };
        let run = steer.as_ref().map(|d| d.provider_run_id.clone());
        let target_prompt = steer
            .as_ref()
            .and_then(|d| d.target_active_prompt_id.clone());
        let Outcome::Event(attempt) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Attempt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                    prompt: prompt_id.clone(),
                    target: target_prompt,
                    run,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        if attempt.state == "blocked" {
            return Ok(());
        }
        if let Some(dispatch) = steer {
            let _permit = self
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if !self
                .owned
                .prompt_dispatch_matches_active_prompt(&dispatch)
                .unwrap_or(false)
            {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Receipt {
                        room: room.into(),
                        agent: agent.into(),
                        sequence: event.sequence,
                        state: "rejected".into(),
                    })?;
                return Ok(());
            }
            self.bind_agent_event_attempt(room, agent, event.sequence, &dispatch.provider_run_id)?;
            let structured = self.owned.provider_store.run_uses_structured_prompt_io(
                &self
                    .owned
                    .provider_store
                    .get_run(&dispatch.provider_run_id)?,
            );
            let result = self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await;
            let state = match &result {
                Ok(true) if structured => "submitting",
                Ok(true) => "uncertain",
                Ok(false) | Err(DaemonError::ProviderPromptSteerRejected { .. }) => "rejected",
                Err(_) => "uncertain",
            };
            if state != "submitting" {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Receipt {
                        room: room.into(),
                        agent: agent.into(),
                        sequence: event.sequence,
                        state: state.into(),
                    })?;
            }
            result?;
        } else {
            let result = Box::pin(self.submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: room.into(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                false,
            ))
            .await;
            match result {
                Ok(mut submission) => {
                    if let Some(dispatch) = submission.dispatch.take() {
                        let _permit = self
                            .provider_runtime_lanes
                            .acquire(&dispatch.provider_run_id)
                            .await;
                        self.bind_agent_event_attempt(
                            room,
                            agent,
                            event.sequence,
                            &dispatch.provider_run_id,
                        )?;
                        let structured = self.owned.provider_store.run_uses_structured_prompt_io(
                            &self
                                .owned
                                .provider_store
                                .get_run(&dispatch.provider_run_id)?,
                        );
                        let result = self
                            .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                            .await;
                        let state = match &result {
                            Ok(true) if structured => "submitting",
                            Ok(true) => "uncertain",
                            Ok(false) | Err(DaemonError::ProviderPromptSteerRejected { .. }) => {
                                "rejected"
                            }
                            Err(_) => "uncertain",
                        };
                        if state != "submitting" {
                            self.owned
                                .durable_state_store
                                .agent_lifecycle(Operation::Receipt {
                                    room: room.into(),
                                    agent: agent.into(),
                                    sequence: event.sequence,
                                    state: state.into(),
                                })?;
                        }
                        result?;
                    }
                }
                Err(e) => {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Receipt {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                            state: "uncertain".into(),
                        })?;
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    fn bind_agent_event_attempt(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        run: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::BindAttempt {
                room: room.into(),
                agent: agent.into(),
                sequence,
                run: run.into(),
                submit_epoch: self.owned.provider_store.structured_submit_epoch(),
            })?;
        Ok(())
    }
}
