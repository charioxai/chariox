use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::PromptQueueItem;
use crate::transport::flow_control;

use super::super::{select_next_queued_prompt_candidate, KernelAgentService};

impl<'a> KernelAgentService<'a> {
    pub(crate) fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let target_agent_id = peeked.target_agent_id().to_string();
            let is_workflow_prompt = crate::app::workflow_runtime::is_workflow_prompt_source(
                peeked.source_attachment_id(),
            );
            let provider_run_id = match self
                .app
                .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)
            {
                Ok(provider_run_id) => provider_run_id,
                Err(DaemonError::NoActiveProviderRun { .. }) if is_workflow_prompt => {
                    match crate::app::workflow_runtime::ensure_workflow_provider_run_from_runtime(
                        self.app,
                        session_id,
                        &target_agent_id,
                    ) {
                        Ok(provider_run_id) => provider_run_id,
                        Err(error) => {
                            self.app.record_notice(
                                session_id,
                                None,
                                self.app.attachments.list_session_attachment_ids(session_id),
                                format!(
                                    "Deferred queued workflow prompt `{}` because Arroba could not launch the provider run for agent `{}`: {}",
                                    peeked.id(),
                                    target_agent_id,
                                    error
                                ),
                            );
                            return Ok(None);
                        }
                    }
                }
                Err(error) => {
                    self.app.record_notice(
                        session_id,
                        None,
                        self.app.attachments.list_session_attachment_ids(session_id),
                        format!(
                            "Deferred queued prompt `{}` because Arroba could not activate the provider run for agent `{}`: {}",
                            peeked.id(),
                            target_agent_id,
                            error
                        ),
                    );
                    return Ok(None);
                }
            };
            if self.app.providers.get_run(&provider_run_id)?.state() == ProviderRunState::Starting {
                return Ok(None);
            }
            let (_session, next_candidate) = self.activate_next_queued_prompt_for_mirror(
                session_id,
                &target_agent_id,
                expected_next,
            )?;
            let Some(next) = next_candidate else {
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            };
            let source_attachment_id = self
                .app
                .promoted_prompt_source_attachment_id(session_id, next.source_attachment_id())?;

            if let Err(error) = crate::app::ProviderPromptDispatcher::new(self.app)
                .dispatch_prompt_to_provider(
                    session_id,
                    &provider_run_id,
                    &source_attachment_id,
                    next.prompt(),
                    next.hidden_system_context(),
                    next.attachments(),
                )
            {
                self.app.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.app.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` after PTY delivery failure: {}",
                        next.id(),
                        error
                    ),
                );
                let cancelled = self
                    .app
                    .prompt_owner_cancel_active_prompt_only(session_id, &target_agent_id);
                if is_workflow_prompt {
                    if let Ok(cancelled) = cancelled {
                        crate::app::workflow_runtime::cancel_workflow_prompt_from_runtime(
                            self.app, session_id, &cancelled,
                        )?;
                    }
                    flow_control::clear_prompt_activity(self.app, &provider_run_id);
                    return Err(error);
                }
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            }

            let active = self
                .app
                .prompt_owner_mark_active_prompt_running(session_id, &target_agent_id)?;
            self.app.spawn_user_prompt_history_append_with_prompt_id(
                session_id,
                &source_attachment_id,
                active.target_agent_id(),
                active.prompt(),
                active.attachments(),
                active.id(),
                active.workflow_run_id(),
                active.workflow_node_run_id(),
            )?;
            self.app.echo_promoted_queued_prompt_to_attachments(
                session_id,
                &provider_run_id,
                active.id(),
                &source_attachment_id,
                active.prompt(),
                active.attachments(),
            );
            let prompt_sent_at_ms = crate::session::unix_epoch_ms();
            self.app
                .agents
                .note_prompt_sent_at(&target_agent_id, prompt_sent_at_ms)?;
            self.app
                .sessions
                .note_prompt_sent(session_id, &target_agent_id, prompt_sent_at_ms)?;
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.app.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            crate::app::workflow_runtime::start_workflow_prompt_from_runtime(
                self.app, session_id, &active,
            )?;
            flow_control::note_prompt_started(self.app, &provider_run_id);
            crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
            return Ok(Some(active));
        }
    }

    fn peek_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(prompt) = self
            .app
            .agent_runtime_projection_store()
            .next_queued_prompt(session_id, agent_id)
        {
            return Ok(Some(prompt));
        }
        self.app
            .prompt_owner_peek_next_queued_prompt(session_id, agent_id)
    }

    pub(super) fn next_queued_prompt_candidate(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(expected_next) = expected_next {
            return Ok(select_next_queued_prompt_candidate(
                Some(expected_next),
                None,
            ));
        }
        Ok(select_next_queued_prompt_candidate(
            None,
            self.peek_next_queued_prompt(session_id, agent_id)?,
        ))
    }

    pub(super) fn activate_next_queued_prompt_for_mirror(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            Option<crate::session::PromptQueueItem>,
        ),
        DaemonError,
    > {
        let prompt_id = self.app.sessions_mut().reserve_prompt_id();
        self.activate_next_queued_prompt_for_mirror_with_prompt_id(
            session_id,
            agent_id,
            expected_next,
            prompt_id,
        )
    }

    pub(super) fn activate_next_queued_prompt_for_mirror_with_prompt_id(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
        prompt_id: String,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            Option<crate::session::PromptQueueItem>,
        ),
        DaemonError,
    > {
        let expected_prompt_id = expected_next.map(PromptQueueItem::id);
        let next = self
            .app
            .prompt_owner_activate_next_queued_prompt_with_prompt_id(
                session_id,
                agent_id,
                expected_prompt_id,
                prompt_id,
            )?;
        let session =
            crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok((session, next))
    }
}
