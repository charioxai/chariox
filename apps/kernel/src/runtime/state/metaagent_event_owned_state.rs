use super::*;

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
            Some(source_agent_id),
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
        let assembly = crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
            crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                event_id: record.event_id,
                event_kind: record.kind,
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
        self.submit_metaagent_event_prompt(session_id, prompt)
            .unwrap_or_default()
    }

    fn submit_metaagent_event_prompt(
        &self,
        session_id: &str,
        prompt: crate::session::PromptQueueItem,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        if let Some(dispatches) = self.steer_active_metaagent_event_prompt(session_id, &prompt)? {
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
                None => return Ok(WorkflowPromptDispatches::default()),
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

    fn steer_active_metaagent_event_prompt(
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
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_none()
        {
            return Ok(None);
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
