//! MP-08/MP-10/MP-11: Project-aware queue promotion through normal provider activation.
use super::owned::OwnedPromptCompletion;
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn project_prompt_provider_requires_resolution(
        &self,
        session: &crate::session::RuntimeSession,
        run: &crate::provider::RuntimeProviderRun,
    ) -> bool {
        if run.project_environment_revision().is_some() {
            return true;
        }
        // A failed manifest read must defer rather than reuse stale inputs.
        let config = self.config_projection.snapshot();
        !matches!(
            crate::project_environment::ProjectEnvironmentStore::new(
                &config.private_runtime_state_root(),
            )
            .load(session.project_id()),
            Ok(None)
        )
    }
}

impl KernelRuntimeState {
    pub(super) fn spawn_project_queued_prompt_after_profile_transition(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
            .is_some()
        {
            let state = self.clone();
            let session_id = session_id.to_string();
            let agent_id = agent_id.to_string();
            tokio::spawn(async move {
                state
                    .advance_project_queued_prompt_after_settlement(&session_id, &agent_id)
                    .await;
            });
        }
        Ok(())
    }

    pub(super) async fn complete_local_prompt_with_queued_advance_if_matches(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next: &crate::session::PromptQueueItem,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let completion = self
            .owned
            .complete_local_prompt_with_queued_advance_if_matches(
                session_id,
                agent_id,
                provider_run_id,
                next,
                expected_prompt_id,
            )?;
        let Some(mut completion) = completion else {
            return Ok(None);
        };
        if completion.completion.started_next.is_none() {
            let session = self.owned.session_store.get_session(session_id)?;
            if self
                .owned
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .is_some_and(|run| {
                    self.owned
                        .project_prompt_provider_requires_resolution(&session, &run)
                })
            {
                completion.completion.started_next = self
                    .advance_project_queued_prompt_after_settlement(session_id, agent_id)
                    .await;
            }
        }
        Ok(Some(completion))
    }

    pub(super) async fn advance_project_queued_prompt_after_settlement(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<crate::session::PromptQueueItem> {
        let session = self.owned.session_store.get_session(session_id).ok()?;
        if self
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
            .is_none()
        {
            return None;
        }
        let result = self
            .with_project_prompt_environment(session_id, agent_id, |app| {
                // The existing queue boundary resolves/compares revisions before activation,
                // replaces the idle process, and resumes the agent's native conversation.
                app.advance_next_queued_prompt(session_id, agent_id)
            })
            .await;
        match result {
            Ok(started) => started,
            Err(error) => {
                self.owned.record_notice(
                    session_id,
                    None,
                    self.owned
                        .attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued prompt remained pending while preparing Project inputs: {error}"
                    ),
                );
                None
            }
        }
    }
}
