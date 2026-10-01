use super::*;

impl KernelRuntimeState {
    pub(crate) fn runtime_tool_catalog_changed_for_auth_token(&self, auth_token: &str) {
        for run in self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token)
        {
            if !crate::provider::provider_runtime_catalog_requires_reload(run.provider()) {
                continue;
            }
            let Some(agent_id) = run.agent_instance_id() else {
                continue;
            };
            let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
                continue;
            };
            if self
                .owned
                .provider_run_projection
                .is_leased_provider_run(run.id())
            {
                // The home owns prompt continuation. Workers only refresh their provider.
                self.remember_pending_provider_catalog_reload(agent.session_id(), agent.id());
            } else {
                self.remember_runtime_catalog_continuation(&agent, "runtime tool catalog");
            }
        }
    }

    pub(super) fn runtime_catalog_grant_effect(
        &self,
        agent: &crate::agent::AgentInstance,
        already_granted: bool,
    ) -> (&'static str, bool) {
        let pending = self
            .owned
            .pending_mcp_continuations
            .write()
            .contains_key(agent.id())
            || self
                .owned
                .pending_provider_reloads
                .write()
                .contains_key(agent.id());
        if crate::provider::provider_runtime_catalog_requires_reload(agent.provider())
            && (!already_granted || pending)
        {
            ("after_provider_reload", true)
        } else {
            ("now", false)
        }
    }

    pub(super) fn remember_runtime_catalog_continuation(
        &self,
        agent: &crate::agent::AgentInstance,
        name: &str,
    ) {
        if !crate::provider::provider_runtime_catalog_requires_reload(agent.provider()) {
            return;
        }
        let Ok(session) = self.owned.session_store.get_session(agent.session_id()) else {
            return;
        };
        // Idle changes reload the conversation without replaying a finished user request.
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
        else {
            self.remember_pending_provider_catalog_reload(session.id(), agent.id());
            return;
        };
        self.remember_pending_mcp_continuation(
            session.id(),
            agent.id(),
            prompt.source_attachment_id(),
            name,
            prompt.prompt(),
        );
    }

    pub(super) async fn activate_agent_mcp_grants_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        requested_mcp_name: &str,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        let reason = format!("MCP `{requested_mcp_name}`");
        let provider = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .provider()
            .to_string();
        if crate::provider::provider_runtime_catalog_requires_reload(&provider) {
            self.reload_agent_provider_catalog_if_idle(session_id, agent_id, &reason)
                .await
        } else {
            self.reload_agent_provider_if_idle(session_id, agent_id, &reason)
                .await
        }
    }

    pub(super) fn remember_pending_mcp_continuation(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        mcp_name: &str,
        previous_prompt: &str,
    ) {
        let mut pending = self.owned.pending_mcp_continuations.write();
        if pending.contains_key(agent_id) {
            return;
        }
        pending.insert(
            agent_id.to_string(),
            PendingMcpContinuation {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                source_attachment_id: source_attachment_id.to_string(),
                mcp_name: mcp_name.to_string(),
                previous_prompt: previous_prompt.to_string(),
            },
        );
        drop(pending);
        self.schedule_pending_mcp_continuation_when_idle(
            session_id.to_string(),
            agent_id.to_string(),
        );
    }

    fn requeue_pending_mcp_continuation(&self, continuation: PendingMcpContinuation) {
        let session_id = continuation.session_id.clone();
        let agent_id = continuation.agent_id.clone();
        self.owned
            .pending_mcp_continuations
            .write()
            .entry(agent_id.clone())
            .or_insert(continuation);
        self.schedule_pending_mcp_continuation_when_idle(session_id, agent_id);
    }

    fn schedule_pending_mcp_continuation_when_idle(&self, session_id: String, agent_id: String) {
        let state = self.clone();
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
                if crate::provider::provider_run_reuses_run_for_mcp_continuation_reload(&run) {
                    None
                } else {
                    Some(run.id().to_string())
                }
            });
        match self
            .activate_agent_mcp_grants_if_idle(
                &continuation.session_id,
                &continuation.agent_id,
                &continuation.mcp_name,
            )
            .await?
        {
            ProviderReloadOutcome::Deferred => {
                self.requeue_pending_mcp_continuation(continuation);
                return Ok(());
            }
            ProviderReloadOutcome::Reloaded => {
                self.wait_for_agent_provider_relaunch(
                    &continuation.session_id,
                    &continuation.agent_id,
                    previous_provider_run_id.as_deref(),
                )
                .await?;
            }
            ProviderReloadOutcome::Unaffected => {}
        }

        let (hidden_system_context, _manifest) =
            crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_mcp_skill_continuation_context(&continuation.mcp_name)?;
        let prompt = crate::session::PromptQueueItem::new(
            format!(
                "pending-draft:mcp-continuation:{}:{}",
                continuation.session_id, continuation.agent_id
            ),
            self.ensure_mcp_continuation_attachment(&continuation)?,
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

    // An accepted capability continuation belongs to the kernel, even after the
    // client that submitted the original turn disconnects. Use the same automation
    // attachment/admission path as schedules and metaagent tasks.
    pub(super) fn ensure_mcp_continuation_attachment(
        &self,
        continuation: &PendingMcpContinuation,
    ) -> Result<String, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&continuation.agent_id)?;
        if agent.session_id() != continuation.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: continuation.session_id.clone(),
                agent_id: continuation.agent_id.clone(),
            });
        }
        let client_id = format!("mcp-continuation:{}", agent.id());
        if let Some(attachment) = self
            .owned
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == continuation.session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let attachment = self
            .owned
            .attach(crate::attachment::AttachRequest::for_user(
                &continuation.session_id,
                client_id,
                crate::attachment::ClientCapabilityLevel::AutomationOnly,
                agent.owner_user_id(),
            ))?;
        Ok(attachment.id().to_string())
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
