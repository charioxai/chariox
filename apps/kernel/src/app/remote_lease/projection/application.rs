//! MP-08 / MP-10 / MP-11: crash-safe home page effects and post-receipt queue admission.
use super::*;
use crate::app::provider_output_fanout::ProviderOutputFanout;
use crate::history::{HistoryEvent, HistoryEventTurnContext, SessionHistoryEntry};

pub(super) fn item_identity(
    ack: &RemoteProjectionAcknowledgement,
    kind: &str,
    index: usize,
) -> String {
    format!(
        "{}:{}:{kind}:{index}",
        ack.receipt_key, ack.delivery.sequence
    )
}

impl RemoteLeaseRuntime<'_> {
    pub(super) fn home_workflow_prompt_is_terminal(
        &self,
        session: &str,
        prompt: &PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let (Some(run), Some(node)) = (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(false);
        };
        let snapshot = self.app.sessions.get_session(session)?;
        if let Some(run) = snapshot.workflow_run(run) {
            return Ok(run
                .node_runs()
                .iter()
                .find(|n| n.id() == node)
                .is_some_and(|n| {
                    matches!(
                        n.status(),
                        crate::session::WorkflowNodeRunStatus::Completed
                            | crate::session::WorkflowNodeRunStatus::Failed
                            | crate::session::WorkflowNodeRunStatus::Stopped
                    )
                }));
        }
        crate::app::workflow_runtime::workflow_prompt_run_is_archived_terminal(
            self.app, session, prompt,
        )
    }

    pub(super) fn fail_home_workflow_prompt(
        &self,
        session: &str,
        prompt: &PromptQueueItem,
        failure: crate::session::WorkflowFailureEvent,
    ) -> Result<(), DaemonError> {
        if self.home_workflow_prompt_is_terminal(session, prompt)? {
            return Ok(());
        }
        let (Some(run), Some(node)) = (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        self.app
            .sessions_mut()
            .record_workflow_failure_event(session, run, failure)?;
        self.app
            .sessions_mut()
            .fail_workflow_node_run(session, run, node)?;
        Ok(())
    }

    fn persist_home_workflow_results(
        &self,
        completions: &[PromptCompletion],
    ) -> Result<(), DaemonError> {
        for completion in completions {
            let prompt = &completion.completed;
            if prompt.workflow_run_id().is_none() {
                continue;
            }
            let agent = self.app.agents.get_agent(prompt.target_agent_id())?;
            let durable = self.app.durable_state_store();
            durable.with_workflow_runtime_transition_lock(|| {
                let session = crate::app::KernelSessionReadService::new(self.app)
                    .session_snapshot(agent.session_id())?;
                durable
                    .persist_workflow_runtime_transition(&session, "home_projection_application")?;
                Ok(())
            })?;
        }
        Ok(())
    }

    pub(super) fn restore_home_projection_settlements(
        &self,
        session: &str,
        agent: &str,
        result: &crate::history::leased_projection::HomeProjectionResult,
    ) -> Result<(), DaemonError> {
        // Resume the locally accepted outcome, not a second interpretation of
        // packet completions that may include unrelated or unscoped receipts.
        for completion in &result.completions {
            let prompt = &completion.completed;
            let saved = result
                .settlement
                .as_ref()
                .filter(|saved| saved.prompt_id == prompt.id())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "restore home projection outcome",
                    message: "journal has no exact admitted prompt outcome".into(),
                })?;
            self.record_projected_prompt_settlement(
                session,
                agent,
                prompt,
                &saved.provider_run_id,
                saved.settled_at_ms,
                saved.termination.clone(),
                saved.status,
            )?;
        }
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent_snapshot(session, agent)?;
        if result.provider_failed
            && active.as_ref().is_none_or(|active| {
                result
                    .completions
                    .iter()
                    .any(|completion| completion.completed.id() == active.id())
            })
        {
            self.app
                .agents()
                .mark_unexpected_provider_exit_error(agent, true)?;
        }
        Ok(())
    }

    pub(super) fn finish_home_projection(
        &mut self,
        outcome: &mut RemoteRuntimeProjectionOutcome,
    ) -> Result<(), DaemonError> {
        let ack = outcome.acknowledgement.clone();
        if !outcome.application_result_ready {
            if outcome.advance_queue {
                if let Some(completion) = outcome.completions.first() {
                    let agent = self
                        .app
                        .agents
                        .get_agent(completion.completed.target_agent_id())?;
                    outcome.queue_head = self
                        .app
                        .prompt_owner_peek_next_queued_prompt(agent.session_id(), agent.id())?;
                }
            }
            if let Some(ack) = ack.as_ref() {
                // A ready journal skips workflow mutation on replay. Its effects
                // must already survive restart; terminal-node guards also make
                // a crash between this transition and result save repeatable.
                self.persist_home_workflow_results(&outcome.completions)?;
                let run = crate::provider::projected_leased_provider_run_id(
                    &ack.binding.leased_agent_id,
                    &ack.provider_run_id,
                );
                outcome.retain_native_binding = self
                    .app
                    .provider_run_projection
                    .get(&run)
                    .is_some_and(|run| {
                        !run.client_interface().is_chariox()
                            && run.state() == crate::provider::ProviderRunState::Running
                    });
                self.app
                    .operational_history_store()
                    .record_home_projection_result(
                        &ack.receipt_key,
                        ack.delivery.sequence,
                        &crate::history::leased_projection::HomeProjectionResult {
                            completions: outcome.completions.clone(),
                            provider_failed: outcome.provider_failed,
                            advance_queue: outcome.advance_queue,
                            queue_head: outcome.queue_head.clone(),
                            retain_native_binding: outcome.retain_native_binding,
                            settlement: outcome.settlement.clone(),
                        },
                    )?;
            }
        }
        for completion in &outcome.completions {
            let completed = &completion.completed;
            let agent_id = completed.target_agent_id();
            // The journal can outlive prompt removal and successor admission.
            // It may finish the exact original, never the new active prompt.
            let session_id = self
                .app
                .agents
                .get_agent(agent_id)?
                .session_id()
                .to_string();
            let active = self
                .app
                .prompt_owner_active_prompt_for_agent(&session_id, agent_id)?;
            if active
                .as_ref()
                .is_some_and(|prompt| prompt.id() == completed.id())
            {
                let binding = self
                    .app
                    .agents
                    .get_agent(agent_id)?
                    .remote_execution()
                    .cloned();
                if let Some(binding) = binding {
                    let run = binding.active_worker_provider_run_id.as_deref();
                    if let Some(run) = run {
                        let projected = crate::provider::projected_leased_provider_run_id(
                            &binding.leased_agent_id,
                            run,
                        );
                        crate::transport::flow_control::clear_prompt_activity(self.app, &projected);
                        let retain = outcome.retain_native_binding
                            || self
                                .app
                                .provider_run_projection
                                .get(&projected)
                                .is_some_and(|run| {
                                    !run.client_interface().is_chariox()
                                        && run.state() == crate::provider::ProviderRunState::Running
                                });
                        if !retain {
                            self.app
                                .agents
                                .set_remote_execution_active_worker_provider_run_id(
                                    agent_id, None,
                                )?;
                        }
                    }
                }
                // The worker receipt already owns the definitive settlement.
                // Remove only its prompt through the shared owner; synthesizing
                // a local cancellation here would replace the scoped receipt.
                let session = self.app.sessions.get_session(&session_id)?;
                self.app
                    .prompt_state_owner()
                    .complete_active_prompt_if_matches(&session, agent_id, Some(completed.id()))
                    .ok_or_else(|| DaemonError::NoActivePrompt {
                        session_id: session_id.clone(),
                    })?;
                self.app
                    .mirror_prompt_owner_agent_state(&session_id, agent_id)?;
            } else {
                // Retry a failed durable mirror as well as a process restart.
                self.app
                    .mirror_prompt_owner_agent_state(&session_id, agent_id)?;
            }
        }
        if let Some(ack) = ack.as_ref() {
            // Output, workflow effects and exact prompt removal are durable before
            // this write. Queue admission is always after it. A failed write leaves
            // the result journal ready to finish without replaying those effects.
            self.app
                .operational_history_store()
                .record_leased_projection_delivery_receipt(
                    &ack.receipt_key,
                    ack.delivery.sequence,
                )?;
        }
        if outcome.advance_queue {
            for completion in &mut outcome.completions {
                let agent = self
                    .app
                    .agents
                    .get_agent(completion.completed.target_agent_id())?;
                completion.started_next = self.advance_home_projection_queue(
                    agent.session_id(),
                    agent.id(),
                    &mut outcome.remote_dispatches,
                    outcome.queue_head.as_ref(),
                )?;
            }
        }
        if let Some(ack) = ack.as_ref() {
            self.app
                .operational_history_store()
                .finish_home_projection_application(&ack.receipt_key, ack.delivery.sequence)?;
        }
        Ok(())
    }

    fn advance_home_projection_queue(
        &mut self,
        session_id: &str,
        agent_id: &str,
        dispatches: &mut Vec<crate::app::KernelRemotePromptDispatchIntent>,
        expected_head: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if expected_head.is_none() {
            self.app.sync_focused_provider_run_if_idle(session_id)?;
            return Ok(None);
        }
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_some()
        {
            if let Some((active, intent)) = crate::app::KernelAgentService::new(self.app)
                .recover_admitted_remote_prompt_intent(
                    session_id,
                    agent_id,
                    expected_head.expect("head checked above"),
                )?
            {
                dispatches.push(intent);
                return Ok(Some(active));
            }
            return Ok(None);
        }
        let Some(remote) = self
            .app
            .agents
            .get_agent(agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        let queued = self
            .app
            .prompt_owner_peek_next_queued_prompt(session_id, agent_id)?;
        if queued.as_ref().map(PromptQueueItem::id) != expected_head.map(PromptQueueItem::id) {
            return Ok(None);
        }
        let workflow = |prompt: &PromptQueueItem| {
            crate::app::workflow_runtime::is_workflow_prompt_source(prompt.source_attachment_id())
        };
        let mut next = None;
        if !queued.as_ref().is_some_and(workflow) {
            if let Some((prompt, intent)) = crate::app::KernelAgentService::new(self.app)
                .admit_next_queued_remote_prompt(session_id, agent_id, expected_head)?
            {
                dispatches.push(intent);
                next = Some(prompt);
            }
        }
        if next.is_none() {
            let selected = self
                .app
                .prompt_owner_peek_next_queued_prompt(session_id, agent_id)?;
            if selected.as_ref().is_some_and(workflow)
                && selected.as_ref().map(PromptQueueItem::id)
                    == expected_head.map(PromptQueueItem::id)
            {
                next = self
                    .app
                    .advance_next_queued_prompt_remote_with_workflow_dispatch(
                        session_id,
                        agent_id,
                        &remote.worker_kernel_id,
                        &remote.leased_agent_id,
                        remote.relay_url.as_deref(),
                        remote.relay_token.as_deref(),
                        selected.as_ref(),
                    )?;
            }
        }
        if next.is_none() {
            self.app.sync_focused_provider_run_if_idle(session_id)?;
        }
        Ok(next)
    }

    fn persist_home_projection_entry(
        &self,
        identity: &str,
        entry: SessionHistoryEntry,
    ) -> Result<bool, DaemonError> {
        entry.validate_for_history_append("append home projection")?;
        let context = crate::app::HistoryEventContextResolver::new(
            self.app.providers.clone(),
            self.app.sessions.clone(),
            self.app.prompt_state_owner(),
            self.app.active_turn_store(),
        )
        .resolve(&entry);
        let store = self.app.operational_history_store();
        let event = HistoryEvent::transcript(store.reserve_sequence(), &entry, context);
        let (event, inserted) = store.append_home_projection_history(identity, event)?;
        if inserted
            && crate::provider_output_policy::tool_history::is_unread_output_history_entry(&entry)
        {
            if let Some(agent_id) = entry.agent_id.as_deref() {
                self.app.sessions.note_agent_output_sequence(
                    &entry.session_id,
                    agent_id,
                    event.sequence,
                )?;
            }
        }
        if self.app.history_archive_enabled() {
            store.enqueue_archive_events(std::slice::from_ref(&event))?;
        }
        Ok(inserted)
    }

    pub(super) fn project_home_output(
        &mut self,
        session: &str,
        agent: &str,
        run: &str,
        recipients: &[String],
        chunk: RelayProjectedOutputChunk,
        identity: Option<String>,
    ) -> Result<(), DaemonError> {
        let Some(identity) = identity else {
            self.app.fan_out_output_for_agent(
                session,
                run,
                Some(agent),
                chunk.kind,
                chunk.merge_key,
                recipients.to_vec(),
                &chunk.bytes,
            );
            return Ok(());
        };
        // ProviderTerminal/PromptEcho are ephemeral and have no canonical transcript.
        if !matches!(
            chunk.kind,
            TerminalOutputKind::ProviderTerminal | TerminalOutputKind::PromptEcho
        ) {
            let bytes = crate::provider_output_policy::output_bounds::bounded_terminal_output_bytes(
                &chunk.kind,
                &chunk.bytes,
            );
            let active = self
                .app
                .prompt_owner_active_prompt_for_agent(session, agent)?;
            let entry = SessionHistoryEntry::provider_output(
                session,
                run,
                Some(agent),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )
            .with_prompt_origin(active.as_ref().map(PromptQueueItem::prompt_origin))
            .with_source_attachment_id(
                active
                    .as_ref()
                    .map(|prompt| prompt.source_attachment_id().to_string()),
            );
            if !self.persist_home_projection_entry(&identity, entry)? {
                return Ok(());
            }
        }
        ProviderOutputFanout::without_history(self.app).fan_out_for_agent(
            session,
            run,
            Some(agent),
            chunk.kind,
            chunk.merge_key,
            recipients.to_vec(),
            &chunk.bytes,
        );
        Ok(())
    }

    pub(super) fn project_home_notice(
        &mut self,
        session: &str,
        agent: &str,
        run: &str,
        recipients: &[String],
        notice: String,
        identity: Option<String>,
    ) -> Result<(), DaemonError> {
        if let Some(identity) = identity {
            if !self.persist_home_projection_entry(
                &identity,
                SessionHistoryEntry::notice(session, Some(run), Some(agent), notice.clone()),
            )? {
                return Ok(());
            }
            ProviderOutputFanout::without_history(self.app).record_notice_for_agent(
                session,
                Some(run),
                Some(agent),
                recipients.to_vec(),
                notice,
            );
        } else {
            self.app.record_notice_for_agent(
                session,
                Some(run),
                Some(agent),
                recipients.to_vec(),
                notice,
            );
        }
        Ok(())
    }

    pub(super) fn persist_home_projection_prompt(
        &self,
        session: &str,
        prompt: &PromptQueueItem,
        identity: &str,
    ) -> Result<(), DaemonError> {
        let mut entry = SessionHistoryEntry::user_prompt_with_attachments(
            session,
            prompt.source_attachment_id(),
            prompt.target_agent_id(),
            crate::prompt_transcript::render_prompt_transcript(
                prompt.prompt(),
                prompt.attachments(),
            ),
            prompt.attachments(),
        )
        .with_prompt_origin(prompt.prompt_origin());
        entry.timestamp_ms = prompt.created_at_ms();
        entry.merge_key = Some(format!("prompt:{}", prompt.id()));
        self.persist_home_projection_entry(identity, entry)?;
        Ok(())
    }

    pub(super) fn record_home_projection_settlement(
        &self,
        session: &str,
        agent: &str,
        prompt: &PromptQueueItem,
        run: &str,
        at: u64,
        status: crate::git_observer::CompletedTurnSettlementStatus,
    ) -> Result<(), DaemonError> {
        let store = self.app.operational_history_store();
        let event = HistoryEvent::operational(
            store.reserve_sequence(),
            crate::history::HistoryEventKind::ProviderStatus,
            Some(crate::history::HistoryEventRole::System),
            None,
            std::collections::BTreeMap::from([
                (
                    crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY.into(),
                    serde_json::json!(at),
                ),
                (
                    crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY.into(),
                    serde_json::json!(status.as_str()),
                ),
            ]),
            HistoryEventTurnContext {
                session_id: Some(session.into()),
                agent_id: Some(agent.into()),
                turn_id: Some(prompt.id().into()),
                prompt_id: Some(prompt.id().into()),
                provider_run_id: Some(run.into()),
                ..Default::default()
            },
        );
        let (event, _) = store.append_home_projection_history(
            &format!("settlement:{session}:{agent}:{}", prompt.id()),
            event,
        )?;
        if self.app.history_archive_enabled() {
            store.enqueue_archive_events(&[event])?;
        }
        Ok(())
    }
}
