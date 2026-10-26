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
    pub(super) fn spawn_idle_local_prompt_queue_promotion(
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
            && self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_none()
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
        loop {
            let session = self.owned.session_store.get_session(session_id).ok()?;
            // Settlement, profile completion and ordinary backlog retries share
            // this guard. Failed/cancelled preparation drops it without losing work.
            let promotion = self
                .owned
                .prompt_state_owner
                .try_claim_idle_queue_promotion(&session, agent_id)?;
            let mut waiting_for_launch = false;
            let result: Result<Option<crate::session::PromptQueueItem>, DaemonError> = async {
                loop {
                    let session = self.owned.session_store.get_session(session_id)?;
                    let Some(queued) = self
                        .owned
                        .prompt_state_owner
                        .peek_next_queued_prompt(&session, agent_id)
                    else {
                        return Ok(None);
                    };
                    let provider_run_id = self
                        .with_project_prompt_environment(session_id, agent_id, |app| {
                            crate::app::KernelAgentService::new(app)
                                .prepare_queued_prompt_provider_run(session_id, agent_id, &queued)
                        })
                        .await?;
                    let run = self.owned.provider_store.get_run(&provider_run_id)?;
                    if run.state() == crate::provider::ProviderRunState::Starting {
                        waiting_for_launch = true;
                        return Ok(None);
                    }
                    let Some(dispatch) = self.owned.advance_queued_prompt_dispatch_for_head(
                        session_id,
                        agent_id,
                        &provider_run_id,
                        Some(queued.id()),
                    )?
                    else {
                        let current = self.owned.session_store.get_session(session_id)?;
                        let changed = self
                            .owned
                            .prompt_state_owner
                            .peek_next_queued_prompt(&current, agent_id)
                            .is_some_and(|head| head.id() != queued.id());
                        let idle = self
                            .owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&current, agent_id)
                            .is_none();
                        if changed && idle {
                            continue;
                        }
                        return Ok(None);
                    };
                    // Worker workflow context is indexed by the original queued id,
                    // before activation assigned the prompt its fresh mirror id.
                    self.with_app_side_effect(|app| {
                        crate::app::RemoteLeaseRuntime::new(app)
                            .activate_leased_workflow_prompt(queued.id(), &provider_run_id);
                    })
                    .await;
                    let started = self.owned.prompt_state_owner.active_prompt_for_agent(
                        &self.owned.session_store.get_session(session_id)?,
                        agent_id,
                    );
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                        return Ok(None);
                    }
                    return Ok(started);
                }
            }
            .await;
            #[cfg(test)]
            if waiting_for_launch {
                if let Some((entered, release)) = &self.queue_promotion_exit_probe {
                    entered.notify_one();
                    release.notified().await;
                }
            }
            // Readiness may have skipped its advance while this claim was held.
            // Drop first: later readiness owns the normal path; an already-ready
            // run is retried here under a fresh claim, closing both sides of the race.
            drop(promotion);
            if waiting_for_launch
                && self
                    .owned
                    .provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Running)
            {
                continue;
            }
            return match result {
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
            };
        }
    }
}
