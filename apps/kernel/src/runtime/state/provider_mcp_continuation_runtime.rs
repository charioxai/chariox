use super::*;

impl KernelRuntimeState {
    pub(super) fn remember_pending_mcp_continuation(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        mcp_name: &str,
        previous_prompt: &str,
    ) {
        self.remember_mcp_continuation_with_reason(
            session_id,
            agent_id,
            source_attachment_id,
            mcp_name,
            previous_prompt,
            ProviderReloadReason::LaunchInputs(format!("MCP `{mcp_name}`")),
        );
    }

    pub(super) fn remember_pending_runtime_tools_continuation(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        previous_prompt: &str,
    ) {
        self.remember_mcp_continuation_with_reason(
            session_id,
            agent_id,
            source_attachment_id,
            "chariox-runtime",
            previous_prompt,
            ProviderReloadReason::RuntimeToolCatalog,
        );
    }

    fn remember_mcp_continuation_with_reason(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        mcp_name: &str,
        previous_prompt: &str,
        reload_reason: ProviderReloadReason,
    ) {
        let mut pending = self.owned.pending_mcp_continuations.write();
        let reload_reason = pending
            .get(agent_id)
            .map_or(reload_reason.clone(), |previous| {
                reload_reason.merge(&previous.reload_reason)
            });
        pending.insert(
            agent_id.to_string(),
            PendingMcpContinuation {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                source_attachment_id: source_attachment_id.to_string(),
                mcp_name: mcp_name.to_string(),
                previous_prompt: previous_prompt.to_string(),
                reload_reason,
            },
        );
        drop(pending);
        self.ensure_pending_mcp_continuation_poller(session_id, agent_id);
    }

    fn ensure_pending_mcp_continuation_poller(&self, session_id: &str, agent_id: &str) {
        let pending = self.owned.pending_mcp_continuations.write();
        if !pending.contains_key(agent_id) {
            return;
        }
        let poller = self.owned.pending_mcp_continuations.pollers.claim(agent_id);
        drop(pending);
        let Some(mut poller) = poller else {
            return;
        };
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            while poller.next().await {
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
                }
                let queued = state.owned.pending_mcp_continuations.write();
                if !queued.contains_key(&agent_id) {
                    poller.release();
                    return;
                }
            }
            let queued = state.owned.pending_mcp_continuations.write();
            poller.release();
            if queued.contains_key(&agent_id) {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "pending MCP continuation polling expired",
                    serde_json::json!({"session_id": session_id, "agent_id": agent_id}),
                );
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
                if crate::provider::provider_run_reuses_run_for_mcp_continuation_reload(&run) {
                    None
                } else {
                    Some(run.id().to_string())
                }
            });
        let outcome = self
            .reload_agent_provider_if_idle_for_reason(
                &continuation.session_id,
                &continuation.agent_id,
                &continuation.reload_reason,
            )
            .await?;
        if outcome == ProviderReloadOutcome::Deferred {
            let mut queued = self.owned.pending_mcp_continuations.write();
            if let Some(newer) = queued.get_mut(agent_id) {
                newer.reload_reason = newer
                    .reload_reason
                    .clone()
                    .merge(&continuation.reload_reason);
            } else {
                queued.insert(agent_id.to_owned(), continuation);
            }
            drop(queued);
            // Usually the existing poller still owns the original budget. A
            // prompt-completion callback may restart previously expired work,
            // but a Deferred poll attempt never spawns a replacement task.
            self.ensure_pending_mcp_continuation_poller(session_id, agent_id);
            return Ok(());
        }
        if outcome == ProviderReloadOutcome::Reloaded {
            self.wait_for_agent_provider_relaunch(
                &continuation.session_id,
                &continuation.agent_id,
                previous_provider_run_id.as_deref(),
            )
            .await?;
        }

        let (hidden_system_context, _manifest) =
            crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_mcp_skill_continuation_context(&continuation.mcp_name)?;
        let prompt = crate::session::PromptQueueItem::new(
            format!(
                "pending-draft:mcp-continuation:{}:{}",
                continuation.session_id, continuation.agent_id
            ),
            &continuation.source_attachment_id,
            &continuation.agent_id,
            continuation.previous_prompt,
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(hidden_system_context);
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: continuation.session_id,
                prompt,
                force_queue: false,
                refresh_projection: true,
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
