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
        // A running turn keeps its provider lane: the event stays queued in the
        // inbox and the next idle sweep or settlement delivers it.
        if active.is_some() {
            return Ok(());
        }
        let event = loop {
            let front = self
                .owned
                .durable_state_store
                .agent_delivery_front(room, agent)?;
            let Some(event) = front else {
                return Ok(());
            };
            if event.state == "pending" {
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
        if tasks.iter().any(|t| {
            (event.payload["task_id"].as_str() == Some(t.task_id.as_str())
                || event.payload["task_ids"].as_array().is_some_and(|ids| {
                    ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                }))
                && (t.pending_prompt_id.is_some()
                    || session
                        .queued_prompts_for_agent(agent)
                        .is_some_and(|q| q.iter().any(|p| p.id() == t.prompt_id)))
        }) {
            return Ok(());
        }
        let target = self.owned.agent_store.get_agent(agent)?;
        let leased = target.remote_execution().is_some();
        let attachment = self.ensure_agent_message_attachment(room, &target)?;
        // A10: the worker fences a rejected home prompt id, so each leased
        // attempt after a proven rejection uses a fresh id; transport retries
        // and receipt reconciliation within one attempt keep the same id.
        let prompt_id = match event.attempted_at_ms.filter(|_| leased) {
            Some(previous) => format!("agent-event-{}-{}-{previous}", agent, event.sequence),
            None => format!("agent-event-{}-{}", agent, event.sequence),
        };
        let text = format!("Kernel event inbox, untrusted data (sequence {}, source {}).\n{}\nUse chariox.events.ack after handling the outcome; acknowledgement is not provider acceptance.", event.sequence, event.source_id, event.payload);
        let prompt = crate::session::PromptQueueItem::new(
            &prompt_id,
            &attachment,
            agent,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&prompt_id, format!("event:{}", event.sequence));
        let Outcome::Event(attempt) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Attempt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                    prompt: prompt_id.clone(),
                    target: None,
                    run: None,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        if attempt.state == "blocked" {
            return Ok(());
        }
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
                // A10: home ordering is already committed; the worker's exact
                // receipt settles this attempt through the leased dispatch.
                if let Some(dispatch) = submission.remote_dispatch.take() {
                    self.spawn_remote_prompt_dispatch(dispatch);
                }
                if let Some(dispatch) = submission.dispatch.take() {
                    let _permit = self
                        .provider_runtime_lanes
                        .acquire(&dispatch.provider_run_id)
                        .await;
                    let result = self
                        .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                        .await;
                    self.record_agent_event_dispatch_result(&dispatch, &result)?;
                    // The receipt above is exact; the shared failure path then
                    // frees the active prompt the way a spawned dispatch does.
                    if let Err(error) = result {
                        self.fail_prompt_dispatch(dispatch, error).await?;
                    }
                }
            }
            Err(e) => {
                let state = ledger::delivery_receipt_state(None, false);
                self.record_agent_delivery_receipt(room, agent, event.sequence, state)?;
                return Err(e);
            }
        }
        Ok(())
    }
    /// A10: the worker's exact acceptance is the leased turn's provider
    /// dispatch, so it binds the task's run and consumes a retained wait the
    /// way local dispatch does immediately before provider I/O.
    pub(super) fn bind_leased_agent_task(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        worker_run: &str,
    ) {
        let result = (|| {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Begin {
                    owner: self
                        .owned
                        .session_store
                        .get_session(&dispatch.session_id)?
                        .owner_user_id()
                        .into(),
                    room: dispatch.session_id.clone(),
                    agent: dispatch.agent_id.clone(),
                    prompt: dispatch.prompt_id.clone(),
                    run: Some(crate::provider::projected_leased_provider_run_id(
                        &dispatch.leased_agent_id,
                        worker_run,
                    )),
                    now: crate::session::unix_epoch_ms(),
                })
        })();
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: accepted leased turn left its task for sweep",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
        }
    }
    /// A10: the leased dispatch's exact worker outcome is this attempt's
    /// receipt. Acceptance binds the projected worker run; a proven rejection
    /// returns the event to pending; a changed binding stays uncertain.
    pub(super) fn record_leased_event_receipt(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        worker_run: Option<&str>,
        state: &str,
    ) {
        let (room, agent) = (&dispatch.session_id, &dispatch.agent_id);
        let result = (|| {
            let Some(event) = self.owned.durable_state_store.agent_event_for_prompt(
                room,
                agent,
                &dispatch.prompt_id,
            )?
            else {
                return Ok(());
            };
            if !matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked") {
                return Ok(());
            }
            if let Some(run) = worker_run.filter(|_| event.state == "submitting") {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::BindSubmission {
                        room: room.clone(),
                        agent: agent.clone(),
                        sequence: event.sequence,
                        prompt: dispatch.prompt_id.clone(),
                        // Leased delivery never steers an active turn.
                        target: None,
                        run: crate::provider::projected_leased_provider_run_id(
                            &dispatch.leased_agent_id,
                            run,
                        ),
                        submit_epoch: 0,
                        now: crate::session::unix_epoch_ms(),
                    })?;
            }
            self.record_agent_delivery_receipt(room, agent, event.sequence, state)
        })();
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: leased wake receipt retained for sweep reconciliation",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
        }
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
        let state = ledger::delivery_attempt_receipt_state(
            event.submit_epoch.is_some(),
            result,
            structured,
        );
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
#[cfg(test)]
impl KernelRuntimeState {
    /// The leased turn's public answer as its projection records it at home:
    /// under the worker's own run id (the dev-stub provider emits none).
    pub(crate) fn record_leased_answer_for_test(
        &self,
        room: &str,
        agent: &str,
        text: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .ok_or_else(|| ledger::error("no active leased prompt"))?;
        let remote = self
            .owned
            .agent_store
            .get_agent(agent)?
            .remote_execution()
            .cloned()
            .ok_or_else(|| ledger::error("agent is not leased"))?;
        let run = remote
            .active_worker_provider_run_id
            .clone()
            .ok_or_else(|| ledger::error("leased run not bound"))?;
        self.owned.append_operational_history_entry_with_context(
            &crate::history::SessionHistoryEntry::provider_output(
                room,
                &run,
                Some(agent),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                text,
            ),
            crate::history::HistoryEventTurnContext {
                session_id: Some(room.into()),
                agent_id: Some(agent.into()),
                provider_run_id: Some(run.clone()),
                prompt_id: Some(prompt.id().into()),
                turn_id: Some(prompt.id().into()),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub(crate) async fn deliver_agent_inbox_for_test(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        self.deliver_agent_inbox(room, agent).await
    }
}
