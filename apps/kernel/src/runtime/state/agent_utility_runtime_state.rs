use super::*;

impl KernelRuntimeState {
    pub(crate) async fn agent_utility_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<
        (
            crate::agent::AgentInstance,
            crate::provider::RuntimeProviderRun,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .iter()
            .find(|agent| agent.id() == agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` does not belong to session `{session_id}`"),
            })?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "agent `{agent_id}` is remote-backed; hidden utilities must run on its worker kernel"
                ),
            });
        }
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` is busy"),
            });
        }
        let provider_run = if let Some(provider_run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        {
            provider_run
        } else {
            let session_id = session_id.to_string();
            let agent_id = agent_id.to_string();
            let provider_run_id = self
                .with_app_side_effect(move |app| {
                    app.ensure_prompt_provider_run_for_agent(&session_id, &agent_id)
                })
                .await?;
            self.owned.provider_store.get_run(&provider_run_id)?
        };
        Ok((agent, provider_run))
    }

    pub(crate) async fn run_structured_provider_utility_prompt(
        &self,
        provider_run: crate::provider::RuntimeProviderRun,
        visible_user_prompt: String,
        hidden_system_context: String,
        timeout: tokio::time::Duration,
    ) -> Result<String, DaemonError> {
        let provider_store = self.owned.provider_store.clone();
        tokio::task::spawn_blocking(move || {
            provider_store.run_structured_utility_prompt(
                &provider_run,
                &visible_user_prompt,
                &hidden_system_context,
                timeout,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run structured provider utility prompt",
            message: format!("provider utility prompt task failed: {error}"),
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn utility_provider_run_rejects_prompt_owner_active_prompt_when_mirror_is_stale() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-utility-owner-busy",
                "worktree-utility-owner-busy",
            ))
            .expect("session should be created");
        let external_prompt = crate::session::PromptQueueItem::external_observed_running(
            "codex",
            "codex-thread-utility-owner-busy",
            "codex-turn-utility-owner-busy",
            agent.id(),
            "external prompt still running",
        );
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
        .expect("external active prompt should sync");
        app.sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear stale session prompt mirror");
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session should load")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "session mirror should not expose the active prompt"
        );

        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        let error = runtime
            .agent_utility_provider_run(session.id(), agent.id(), "test utility")
            .await
            .expect_err("utility should be rejected while prompt owner has active prompt");

        assert!(
            error.to_string().contains("is busy"),
            "expected busy rejection, got {error}"
        );
    }
}
