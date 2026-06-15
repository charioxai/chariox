use super::*;

impl KernelRuntimeState {
    pub(crate) async fn inject_metaagent_agent_lifecycle_event_for_agent(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        kind: &str,
    ) -> Result<(), DaemonError> {
        let title = match kind {
            "agent.spawned" => format!("Agent `{}` was spawned", agent.agent_ref()),
            "agent.deleted" => format!("Agent `{}` was deleted", agent.agent_ref()),
            _ => format!("Agent `{}` changed", agent.agent_ref()),
        };
        let summary = match kind {
            "agent.spawned" => format!(
                "A regular agent `{}` was spawned by the user in this session.",
                agent.agent_ref()
            ),
            "agent.deleted" => format!(
                "A regular agent `{}` was deleted by the user in this session.",
                agent.agent_ref()
            ),
            _ => format!(
                "A regular agent `{}` had lifecycle event `{kind}` in this session.",
                agent.agent_ref()
            ),
        };
        let source_attachment_id = crate::scheduler::runtime::workflow_prompt_source_attachment_id(
            &format!("metaagent-{kind}-{}", agent.id()),
        );
        let dispatches = self
            .owned
            .metaagent_owned_agent_event_prompt_dispatches_for_agent(
                session_id,
                kind,
                agent,
                &source_attachment_id,
                title,
                summary,
                serde_json::json!({
                    "agent": agent,
                    "kind": kind,
                }),
            );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(())
    }

    pub(crate) fn inject_metaagent_turn_completion_event(
        &self,
        session_id: &str,
        completed_agent_id: &str,
        completion: &crate::session::PromptCompletion,
    ) -> Result<(), DaemonError> {
        let completed_agent = self.owned.agent_store.get_agent(completed_agent_id)?;
        if completed_agent.is_metaagent() {
            return Ok(());
        }
        let prompt_preview = completion
            .completed
            .prompt()
            .chars()
            .take(240)
            .collect::<String>();
        let title = format!(
            "{} completed a turn",
            completed_agent
                .alias()
                .unwrap_or_else(|| completed_agent.agent_ref())
        );
        let summary = format!(
            "Agent {} completed prompt {}. User prompt preview: {}",
            completed_agent.agent_ref(),
            completion.completed.id(),
            if prompt_preview.trim().is_empty() {
                "<empty>"
            } else {
                prompt_preview.trim()
            }
        );
        let dispatches = self.owned.metaagent_owned_agent_event_prompt_dispatches(
            session_id,
            "agent.turn.completed",
            completed_agent_id,
            completion.completed.source_attachment_id(),
            title,
            summary,
            serde_json::json!({
                "completed_prompt_id": completion.completed.id(),
                "source_attachment_id": completion.completed.source_attachment_id(),
                "completed_agent_id": completed_agent.id(),
                "completed_agent_ref": completed_agent.agent_ref(),
                "completed_agent_alias": completed_agent.alias(),
                "started_next_prompt_id": completion.started_next.as_ref().map(|prompt| prompt.id()),
            }),
        );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(())
    }

    pub(crate) fn inject_metaagent_turn_failure_event(
        &self,
        session_id: &str,
        failed_agent_id: &str,
        failed_prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<(), DaemonError> {
        let failed_agent = self.owned.agent_store.get_agent(failed_agent_id)?;
        if failed_agent.is_metaagent() {
            return Ok(());
        }
        let prompt_preview = failed_prompt.prompt().chars().take(240).collect::<String>();
        let title = format!(
            "{} failed a turn",
            failed_agent
                .alias()
                .unwrap_or_else(|| failed_agent.agent_ref())
        );
        let summary = format!(
            "Agent {} failed prompt {}. Error: {}. User prompt preview: {}",
            failed_agent.agent_ref(),
            failed_prompt.id(),
            message.trim(),
            if prompt_preview.trim().is_empty() {
                "<empty>"
            } else {
                prompt_preview.trim()
            }
        );
        let dispatches = self.owned.metaagent_owned_agent_event_prompt_dispatches(
            session_id,
            "agent.turn.failed",
            failed_agent_id,
            failed_prompt.source_attachment_id(),
            title,
            summary,
            serde_json::json!({
                "failed_prompt_id": failed_prompt.id(),
                "source_attachment_id": failed_prompt.source_attachment_id(),
                "failed_agent_id": failed_agent.id(),
                "failed_agent_ref": failed_agent.agent_ref(),
                "failed_agent_alias": failed_agent.alias(),
                "provider_run_id": provider_run_id,
                "message": message,
            }),
        );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(())
    }

    pub(crate) async fn submit_metaagent_command_prompt(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        source_attachment_id: &str,
        target_agent_id: &str,
        prompt_text: String,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let prompt_id = self.owned.session_store.reserve_prompt_id();
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id.clone(),
            source_attachment_id,
            target_agent_id,
            prompt_text,
            crate::session::PromptStatus::Queued,
        );
        if let Some(dispatches) = self
            .owned
            .steer_active_metaagent_prompt(session_id, &prompt)?
        {
            self.spawn_workflow_prompt_dispatches(dispatches);
            self.persist_metaagent_prompt_submission(
                session_id,
                metaagent,
                target_agent_id,
                &prompt_id,
                "steered",
                None,
            );
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "status": "steered",
                    "prompt_id": prompt_id,
                    "target_agent_id": target_agent_id,
                }),
            });
        }
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            })
            .await?;
        if let (crate::session::PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &dispatch.session_id,
                &dispatch.agent_id,
                prompt.id(),
                &dispatch.provider_run_id,
                "metaagent-command",
            );
        }
        let audit_status = match &submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => "submitted",
            crate::session::PromptSubmissionOutcome::Queued { .. } => "queued",
        };
        let audit_provider_run_id = submission
            .dispatch
            .as_ref()
            .map(|dispatch| dispatch.provider_run_id.clone());
        self.persist_metaagent_prompt_submission(
            session_id,
            metaagent,
            target_agent_id,
            &prompt_id,
            audit_status,
            audit_provider_run_id.as_deref(),
        );
        let agent_activity = self.agent_activity_for_session(&submission.session);
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "status": audit_status,
                "outcome": summarize_metaagent_command_prompt_outcome(&submission.outcome),
                "target_agent_id": target_agent_id,
                "provider_run_id": audit_provider_run_id,
                "agent_activity": summarize_metaagent_command_agent_activity(&agent_activity),
            }),
        })
    }

    fn persist_metaagent_prompt_submission(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        target_agent_id: &str,
        prompt_id: &str,
        status: &str,
        provider_run_id: Option<&str>,
    ) {
        let timestamp_ms = crate::session::unix_epoch_ms();
        let correlation_id = format!("metaagent:{}:prompt:{prompt_id}", metaagent.id());
        if let Err(error) = self.owned.durable_state_store.append_event(
            "metaagent.prompt.submitted",
            Some(prompt_id.to_string()),
            serde_json::json!({
                "session_id": session_id,
                "user_id": metaagent.owner_user_id(),
                "metaagent_id": metaagent.id(),
                "target_agent_id": target_agent_id,
                "prompt_id": prompt_id,
                "provider_run_id": provider_run_id,
                "status": status,
                "causation_id": prompt_id,
                "correlation_id": correlation_id,
                "timestamp_ms": timestamp_ms,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.audit",
                "failed to persist metaagent prompt submission audit",
                serde_json::json!({
                    "session_id": session_id,
                    "metaagent_id": metaagent.id(),
                    "target_agent_id": target_agent_id,
                    "prompt_id": prompt_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

fn summarize_metaagent_command_prompt_outcome(
    outcome: &crate::session::PromptSubmissionOutcome,
) -> serde_json::Value {
    match outcome {
        crate::session::PromptSubmissionOutcome::Started { prompt } => serde_json::json!({
            "kind": "started",
            "prompt": summarize_metaagent_command_prompt(prompt),
        }),
        crate::session::PromptSubmissionOutcome::Queued { prompt } => serde_json::json!({
            "kind": "queued",
            "prompt": summarize_metaagent_command_prompt(prompt),
        }),
    }
}

fn summarize_metaagent_command_prompt(
    prompt: &crate::session::PromptQueueItem,
) -> serde_json::Value {
    serde_json::json!({
        "id": prompt.id(),
        "target_agent_id": prompt.target_agent_id(),
        "status": prompt.status(),
        "workflow_run_id": prompt.workflow_run_id(),
        "workflow_node_run_id": prompt.workflow_node_run_id(),
    })
}

fn summarize_metaagent_command_agent_activity(
    activity: &std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
) -> serde_json::Value {
    serde_json::json!({
        "agents": activity
            .iter()
            .map(|(agent_id, state)| {
                serde_json::json!({
                    "agent_id": agent_id,
                    "status": state.status,
                    "prompt_status": state.prompt_status,
                    "busy": state.busy,
                })
            })
            .collect::<Vec<_>>(),
    })
}

impl KernelRuntimeOwnedState {
    pub(super) fn metaagent_owned_agent_event_prompt_dispatches(
        &self,
        session_id: &str,
        kind: &str,
        source_agent_id: &str,
        source_attachment_id: &str,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: serde_json::Value,
    ) -> WorkflowPromptDispatches {
        let Ok(source_agent) = self.agent_store.get_agent(source_agent_id) else {
            return WorkflowPromptDispatches::default();
        };
        self.metaagent_owned_agent_event_prompt_dispatches_for_agent(
            session_id,
            kind,
            &source_agent,
            source_attachment_id,
            title,
            summary,
            detail,
        )
    }

    pub(super) fn metaagent_owned_agent_event_prompt_dispatches_for_agent(
        &self,
        session_id: &str,
        kind: &str,
        source_agent: &crate::agent::AgentInstance,
        source_attachment_id: &str,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: serde_json::Value,
    ) -> WorkflowPromptDispatches {
        if source_agent.is_metaagent() {
            return WorkflowPromptDispatches::default();
        }
        let Some(metaagent) = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .find(|agent| {
                agent.is_metaagent() && agent.owner_user_id() == source_agent.owner_user_id()
            })
        else {
            return WorkflowPromptDispatches::default();
        };
        self.metaagent_event_prompt_for_metaagent(
            session_id,
            &metaagent,
            kind,
            Some(source_agent.id()),
            source_attachment_id,
            title,
            summary,
            detail,
            source_agent.agent_ref().to_string(),
        )
    }

    pub(super) fn metaagent_workflow_event_prompt_dispatches(
        &self,
        session_id: &str,
        kind: &str,
        source_agent_id: Option<&str>,
        source_attachment_id: &str,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: serde_json::Value,
    ) -> WorkflowPromptDispatches {
        let title = title.into();
        let summary = summary.into();
        let source = source_agent_id
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok())
            .map(|agent| agent.agent_ref().to_string())
            .unwrap_or_else(|| "workflow".to_string());
        let mut dispatches = WorkflowPromptDispatches::default();
        for metaagent in self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| agent.is_metaagent())
            .filter(|agent| {
                self.metaagent_events
                    .has_optional_subscription(agent.id(), kind)
            })
        {
            dispatches.extend(self.metaagent_event_prompt_for_metaagent(
                session_id,
                &metaagent,
                kind,
                source_agent_id,
                source_attachment_id,
                title.clone(),
                summary.clone(),
                detail.clone(),
                source.clone(),
            ));
        }
        dispatches
    }

    fn metaagent_event_prompt_for_metaagent(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        kind: &str,
        source_agent_id: Option<&str>,
        source_attachment_id: &str,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: serde_json::Value,
        source: String,
    ) -> WorkflowPromptDispatches {
        let title = title.into();
        let summary = summary.into();
        let prompt_id = self.session_store.reserve_prompt_id();
        let record =
            self.metaagent_events
                .record(crate::runtime::metaagent_event::NewMetaagentEvent {
                    session_id: session_id.to_string(),
                    metaagent_id: metaagent.id().to_string(),
                    owner_user_id: metaagent.owner_user_id().to_string(),
                    kind: kind.to_string(),
                    source_agent_id: source_agent_id.map(str::to_string),
                    title: title.clone(),
                    summary: summary.clone(),
                    detail,
                    injected_prompt_id: Some(prompt_id.clone()),
                });
        self.persist_metaagent_event_record("metaagent.event.recorded", &record);
        let assembly = crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
            crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                event_id: record.event_id.clone(),
                event_kind: record.kind.clone(),
                source,
                title,
                body: summary,
            },
        );
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id,
            source_attachment_id,
            metaagent.id(),
            assembly.visible_user_prompt,
            crate::session::PromptStatus::Queued,
        );
        self.submit_metaagent_event_prompt(session_id, &record.event_id, prompt)
            .unwrap_or_default()
    }

    fn submit_metaagent_event_prompt(
        &self,
        session_id: &str,
        event_id: &str,
        prompt: crate::session::PromptQueueItem,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        if let Some(dispatches) = self.steer_active_metaagent_prompt(session_id, &prompt)? {
            self.update_metaagent_event_prompt_delivery(
                event_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Steered,
                None,
            );
            return Ok(dispatches);
        }
        let prepared = crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.to_string(),
            prompt,
            force_queue: false,
        };
        let mut submission = match self.submit_local_prepared_prompt(&prepared)? {
            Some(submission) => submission,
            None => match self.submit_remote_prepared_prompt(&prepared)? {
                Some(submission) => submission,
                None => {
                    self.update_metaagent_event_prompt_delivery(
                        event_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                        Some("no local or remote prompt route available".to_string()),
                    );
                    return Ok(WorkflowPromptDispatches::default());
                }
            },
        };
        let mut dispatches = WorkflowPromptDispatches::default();
        if let (crate::session::PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&submission.outcome, submission.dispatch.as_ref())
        {
            self.active_turns.start(
                crate::app::ActiveTurnState::new(
                    dispatch.session_id.clone(),
                    dispatch.agent_id.clone(),
                    prompt.id().to_string(),
                    dispatch.provider_run_id.clone(),
                )
                .with_trace_id("metaagent-event"),
            );
        }
        let delivery_status = match &submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Submitted
            }
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
            }
        };
        self.update_metaagent_event_prompt_delivery(event_id, delivery_status, None);
        if let Some(dispatch) = submission.dispatch.take() {
            dispatches.local.push(dispatch);
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            dispatches.remote.push(dispatch);
        }
        if matches!(
            submission.outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ) {
            if let Some(run) = self
                .provider_store
                .get_run_for_agent(&prepared.session_id, prepared.prompt.target_agent_id())
            {
                if run.state() == crate::provider::ProviderRunState::Starting {
                    dispatches.starting_provider_runs.push(run.id().to_string());
                }
            }
        }
        Ok(dispatches)
    }

    pub(super) fn update_metaagent_event_prompt_delivery(
        &self,
        event_id: &str,
        status: crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus,
        error: Option<String>,
    ) {
        if let Some(record) = self
            .metaagent_events
            .update_prompt_delivery_status(event_id, status, error)
        {
            self.persist_metaagent_event_record("metaagent.event.delivery_updated", &record);
        }
    }

    pub(super) fn update_metaagent_event_prompt_delivery_for_prompt(
        &self,
        prompt_id: &str,
        status: crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus,
        error: Option<String>,
    ) {
        if let Some(record) = self
            .metaagent_events
            .update_prompt_delivery_status_for_prompt(prompt_id, status, error)
        {
            self.persist_metaagent_event_record("metaagent.event.delivery_updated", &record);
        }
    }

    pub(super) fn retry_pending_metaagent_event_prompts_for_provider_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let Some(metaagent_id) = run.agent_instance_id() else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let metaagent = self.agent_store.get_agent(metaagent_id)?;
        if !metaagent.is_metaagent() || metaagent.session_id() != run.session_id() {
            return Ok(WorkflowPromptDispatches::default());
        }
        if run.state() != crate::provider::ProviderRunState::Running {
            return Ok(WorkflowPromptDispatches::default());
        }
        let mut records = self
            .metaagent_events
            .list(metaagent.id(), None, Some("failed"), 100);
        records.extend(
            self.metaagent_events
                .list(metaagent.id(), None, Some("recorded"), 100),
        );
        records.retain(|record| record.session_id == run.session_id());
        records.sort_by_key(|record| record.sequence);
        records.dedup_by(|left, right| left.event_id == right.event_id);

        let mut dispatches = WorkflowPromptDispatches::default();
        for record in records {
            let prompt_id = self.session_store.reserve_prompt_id();
            if let Some(retry_record) = self
                .metaagent_events
                .prepare_prompt_delivery_retry(&record.event_id, prompt_id.clone())
            {
                self.persist_metaagent_event_record(
                    "metaagent.event.delivery_retry_prepared",
                    &retry_record,
                );
            }
            let source = record
                .source_agent_id
                .as_deref()
                .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok())
                .map(|agent| agent.agent_ref().to_string())
                .unwrap_or_else(|| "runtime".to_string());
            let assembly =
                crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
                    crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                        event_id: record.event_id.clone(),
                        event_kind: record.kind.clone(),
                        source,
                        title: record.title.clone(),
                        body: record.summary.clone(),
                    },
                );
            let source_attachment_id =
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(&format!(
                    "metaagent-event-retry-{}",
                    record.event_id
                ));
            let prompt = crate::session::PromptQueueItem::new(
                prompt_id,
                &source_attachment_id,
                metaagent.id(),
                assembly.visible_user_prompt,
                crate::session::PromptStatus::Queued,
            );
            dispatches.extend(self.submit_metaagent_event_prompt(
                run.session_id(),
                &record.event_id,
                prompt,
            )?);
        }
        Ok(dispatches)
    }

    fn persist_metaagent_event_record(
        &self,
        kind: &'static str,
        record: &crate::runtime::metaagent_event::MetaagentEventRecord,
    ) {
        if let Err(error) = self.durable_state_store.append_event(
            kind,
            Some(record.event_id.clone()),
            serde_json::json!({
                "record": record,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.event",
                "failed to persist metaagent event record",
                serde_json::json!({
                    "kind": kind,
                    "event_id": &record.event_id,
                    "metaagent_id": &record.metaagent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn steer_active_metaagent_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<WorkflowPromptDispatches>, DaemonError> {
        let target_agent_id = prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
        else {
            return Ok(None);
        };
        if let Some(workflow_run_id) = active_prompt.workflow_run_id() {
            return Err(DaemonError::LocalTransport {
                operation: "metaagent prompt active worker",
                message: format!(
                    "agent `{target_agent_id}` is currently executing workflow run `{workflow_run_id}`; normal metaagent prompts cannot steer an active workflow turn. Use `workflow get-run {workflow_run_id}`, wait for workflow events, or cancel/resume the workflow instead."
                ),
            });
        }
        let Some(provider_run) = self
            .provider_store
            .get_run_for_agent(session_id, &target_agent_id)
        else {
            return Ok(None);
        };
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Ok(None);
        }
        self.append_user_prompt_history(
            session_id,
            prompt.source_attachment_id(),
            &target_agent_id,
            prompt.prompt(),
            prompt.attachments(),
            Some(prompt.id()),
            prompt.workflow_run_id(),
            prompt.workflow_node_run_id(),
        )?;
        let mut dispatches = WorkflowPromptDispatches::default();
        dispatches.local.push(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run.id().to_string(),
            agent_id: target_agent_id,
            prompt_id: prompt.id().to_string(),
            source_attachment_id: prompt.source_attachment_id().to_string(),
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
        });
        Ok(Some(dispatches))
    }
}
