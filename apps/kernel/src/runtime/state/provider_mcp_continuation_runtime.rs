use super::*;

impl KernelRuntimeState {
    pub(super) fn activate_agent_mcp_grants_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        requested_mcp_name: &str,
    ) -> Result<bool, DaemonError> {
        let reason = format!("MCP `{requested_mcp_name}`");
        Ok(matches!(
            self.reload_agent_provider_if_idle(session_id, agent_id, &reason)?,
            ProviderReloadOutcome::Reloaded
        ))
    }

    pub(super) fn remember_pending_mcp_continuation(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        mcp_name: &str,
        previous_prompt: &str,
    ) {
        self.owned.pending_mcp_continuations.write().insert(
            agent_id.to_string(),
            PendingMcpContinuation {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                source_attachment_id: source_attachment_id.to_string(),
                mcp_name: mcp_name.to_string(),
                previous_prompt: previous_prompt.to_string(),
            },
        );
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            for _ in 0..240 {
                let is_idle = state
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .ok()
                    .is_some_and(|session| {
                        state
                            .owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, &agent_id)
                            .is_none()
                    });
                if is_idle {
                    if let Err(error) = state
                        .run_pending_mcp_continuation_after_completion(&session_id, &agent_id)
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.provider",
                            "pending MCP continuation failed",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    async fn take_pending_mcp_continuation_after_completion(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<PendingMcpContinuation> {
        let mut pending = self.owned.pending_mcp_continuations.write();
        let continuation = pending.get(agent_id)?;
        if continuation.session_id != session_id {
            return None;
        }
        pending.remove(agent_id)
    }

    pub(super) async fn run_pending_mcp_continuation_after_completion(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(continuation) = self
            .take_pending_mcp_continuation_after_completion(session_id, agent_id)
            .await
        else {
            return Ok(());
        };
        let previous_provider_run_id = self
            .owned
            .provider_store
            .get_run_for_agent(&continuation.session_id, &continuation.agent_id)
            .and_then(|run| {
                if run.adapter_key() == "opencode" {
                    None
                } else {
                    Some(run.id().to_string())
                }
            });
        self.activate_agent_mcp_grants_if_idle(
            &continuation.session_id,
            &continuation.agent_id,
            &continuation.mcp_name,
        )?;
        self.wait_for_agent_provider_relaunch(
            &continuation.session_id,
            &continuation.agent_id,
            previous_provider_run_id.as_deref(),
        )
        .await?;

        let prompt = crate::session::PromptQueueItem::new(
            self.owned.session_store.reserve_prompt_id(),
            &continuation.source_attachment_id,
            &continuation.agent_id,
            format!(
                "MCP `{}` is now loaded. Continue this request exactly:\n\n{}\n\nUse the newly available provider-native MCP tool if requested, then complete any required Arroba workspace live sync file write before replying.",
                continuation.mcp_name, continuation.previous_prompt
            ),
            crate::session::PromptStatus::Queued,
        );
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: continuation.session_id,
                prompt,
                force_queue: false,
            })
            .await?;
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.owned.provider_store.run_operation_lanes());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }

    async fn wait_for_agent_provider_relaunch(
        &self,
        session_id: &str,
        agent_id: &str,
        previous_provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let ready = self
                .owned
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .is_some_and(|run| {
                    run.state() == crate::provider::ProviderRunState::Running
                        && previous_provider_run_id.is_none_or(|previous| run.id() != previous)
                });
            if ready {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(DaemonError::LocalTransport {
                    operation: "wait_for_mcp_provider_relaunch",
                    message: format!(
                        "timed out waiting for provider relaunch for agent `{agent_id}`"
                    ),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
}
