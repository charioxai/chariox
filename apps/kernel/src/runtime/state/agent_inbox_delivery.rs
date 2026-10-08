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
        let session = self.owned.session_store.get_session(room)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        let event = loop {
            let front = if active.is_some() {
                self.owned
                    .durable_state_store
                    .agent_urgent_delivery_front(room, agent)?
            } else {
                self.owned
                    .durable_state_store
                    .agent_delivery_front(room, agent)?
            };
            let Some(event) = front else {
                return Ok(());
            };
            if event.state == "pending" && event.kind != "message" {
                let mut ids = Vec::new();
                if let Some(id) = event.payload["task_id"].as_str() {
                    ids.push(id);
                }
                if let Some(more) = event.payload["task_ids"].as_array() {
                    ids.extend(more.iter().filter_map(|v| v.as_str()));
                }
                if !ids.is_empty()
                    && ids.iter().all(|id| {
                        tasks.iter().find(|t| t.task_id == *id).is_none_or(|t| {
                            matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                        })
                    })
                {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Expire {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                        })?;
                    continue;
                }
                if ids.iter().any(|id| {
                    tasks
                        .iter()
                        .any(|t| t.task_id == *id && t.state == ExecutionState::Blocked)
                }) && !ids.iter().any(|id| {
                    tasks
                        .iter()
                        .any(|t| t.task_id == *id && t.state == ExecutionState::Waiting)
                }) {
                    // The source occurrence and blocked task retain their outcome.
                    // Retire only this automatic wake so unrelated later wakes run.
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Expire {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                        })?;
                    continue;
                }
            }
            break event;
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
                self.record_agent_delivery_receipt(room, agent, event.sequence, "rejected")?;
                return Ok(());
            }

            let result = self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await;
            self.record_agent_event_dispatch_result(&dispatch, &result)?;
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
                        let result = self
                            .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                            .await;
                        self.record_agent_event_dispatch_result(&dispatch, &result)?;
                        result?;
                    }
                }
                Err(e) => {
                    let state = ledger::delivery_receipt_state(None, false);
                    self.record_agent_delivery_receipt(room, agent, event.sequence, state)?;
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    fn record_agent_delivery_receipt(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        state: &str,
    ) -> Result<(), DaemonError> {
        let Outcome::Event(event) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Receipt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence,
                    state: state.into(),
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        // A refused pre-I/O attempt remains retryable; only acceptance or an
        // uncertain receipt needs a client notice. Sweep surfaces bounded failures.
        if event.state == "pending" {
            return Ok(());
        }
        self.owned.record_notice_for_agent(
            room,
            None,
            Some(agent),
            self.owned
                .attachment_store
                .list_session_attachment_ids(room),
            ledger::receipt_notice(&event),
        );
        Ok(())
    }
    // Called by the shared dispatch path, including authorized substitute reruns.
    pub(super) fn record_agent_event_dispatch_result(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        result: &Result<bool, DaemonError>,
    ) -> Result<(), DaemonError> {
        let Some(event) = self.owned.durable_state_store.agent_event_for_prompt(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
        )?
        else {
            return Ok(());
        };
        if event.state != "submitting" {
            return Ok(());
        }
        // The send boundary may have replaced dispatch's initial run. The
        // bound run, rather than that initial profile, defines receipt semantics.
        let structured = event
            .provider_run_id
            .as_deref()
            .map(|id| {
                self.owned.provider_store.get_run(id).map(|run| {
                    self.owned
                        .provider_store
                        .run_uses_structured_prompt_io(&run)
                })
            })
            .transpose()?
            .unwrap_or(false);
        let state = ledger::delivery_receipt_state(Some(result), structured);
        if state != "submitting" {
            self.record_agent_delivery_receipt(
                &dispatch.session_id,
                &dispatch.agent_id,
                event.sequence,
                state,
            )?;
        }
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    /// Bind only after dispatch admission and immediately before provider I/O.
    pub(super) fn bind_agent_event_submission(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        let Some(event) = self.durable_state_store.agent_event_for_prompt(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
        )?
        else {
            return Ok(());
        };
        self.durable_state_store
            .agent_lifecycle(Operation::BindSubmission {
                room: dispatch.session_id.clone(),
                agent: dispatch.agent_id.clone(),
                sequence: event.sequence,
                prompt: dispatch.prompt_id.clone(),
                target: dispatch.target_active_prompt_id.clone(),
                run: dispatch.provider_run_id.clone(),
                submit_epoch: self.provider_store.structured_submit_epoch(),
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }
}
