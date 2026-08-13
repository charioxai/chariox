use super::*;

impl KernelRuntimeState {
    pub(crate) async fn undo_turn(
        &self,
        request: crate::local::UndoTurnRequest,
        caller_user_id: &str,
    ) -> Result<crate::local::TurnUndoResult, DaemonError> {
        let agent = self.resolve_action_agent(
            &request.session_id,
            request.agent_ref.as_deref(),
            "turn undo",
        )?;
        self.ensure_action_agent_owner(&agent, caller_user_id, "turn undo")?;
        let turn = self
            .owned
            .completed_git_turn_snapshots
            .resolve(&request.session_id, agent.id(), request.turn_ref.as_deref())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "turn undo",
                message: format!(
                    "no completed turn is available to undo for agent `{}`",
                    agent.agent_ref()
                ),
            })?;
        if turn.undone {
            return Err(DaemonError::LocalTransport {
                operation: "turn undo",
                message: format!("turn `{}` has already been undone", turn.before.turn_id),
            });
        }
        let path_results = if let Some(change) = turn.change.clone() {
            tokio::task::spawn_blocking(move || {
                crate::git_observer::apply_workspace_live_sync_undo_to_target(&change)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "turn undo",
                message: error.to_string(),
            })??
        } else {
            Vec::new()
        };
        let failed = path_results
            .iter()
            .filter(|result| {
                matches!(
                    result.status,
                    crate::workspace_live_sync_journal::WorkspaceLiveSyncApplyStatus::FailedIo
                        | crate::workspace_live_sync_journal::WorkspaceLiveSyncApplyStatus::SkippedConflict
                )
            })
            .map(|result| format!("{}: {}", result.path, result.message))
            .collect::<Vec<_>>();
        if !failed.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "turn undo",
                message: format!("turn undo failed: {}", failed.join("; ")),
            });
        }
        self.owned.completed_git_turn_snapshots.mark_undone(
            &request.session_id,
            agent.id(),
            &turn.before.turn_id,
        );
        Ok(crate::local::TurnUndoResult {
            session_id: request.session_id,
            agent_id: agent.id().to_string(),
            turn_id: turn.before.turn_id,
            prompt_id: turn.before.prompt_id,
            provider_run_id: turn.before.provider_run_id,
            reverted_paths: path_results
                .iter()
                .map(|result| result.path.clone())
                .collect(),
            path_results,
        })
    }

    pub(crate) async fn fork_agent(
        &self,
        request: crate::local::ForkAgentRequest,
        caller_user_id: String,
    ) -> Result<
        (
            String,
            crate::agent::AgentInstance,
            crate::provider::RuntimeProviderRun,
            crate::session::RuntimeSession,
        ),
        DaemonError,
    > {
        let source_agent = self.resolve_action_agent(
            &request.session_id,
            request.source_agent_ref.as_deref(),
            "agent fork",
        )?;
        self.ensure_action_agent_owner(&source_agent, &caller_user_id, "agent fork")?;
        if source_agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "agent fork",
                message: "metaagents cannot be forked".to_string(),
            });
        }
        if source_agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "agent fork",
                message: format!(
                    "agent `{}` is remote-backed and cannot be forked locally",
                    source_agent.agent_ref()
                ),
            });
        }
        let source_run = self
            .owned
            .provider_store
            .get_run_for_agent(&request.session_id, source_agent.id())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: request.session_id.clone(),
            })?;

        let mut create_request =
            crate::agent::CreateAgentRequest::new(&request.session_id, source_agent.provider())
                .with_owner_user_id(caller_user_id.clone());
        if let Some(alias) = request.alias.and_then(|alias| {
            let trimmed = alias.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }) {
            create_request = create_request.with_alias(alias);
        }
        if let Some(model) = source_agent.model() {
            create_request = create_request.with_model(model);
        }
        if let Some(effort) = source_agent.effort() {
            create_request = create_request.with_effort(effort);
        }
        if let Some(mode) = source_agent.execution_mode_override() {
            create_request = create_request.with_execution_mode_override(mode);
        }
        if let Some(permission) = source_agent.permission_level_override() {
            create_request = create_request.with_permission_level_override(permission);
        }
        if let Some(worktree_id) = source_agent.worktree_id() {
            create_request = create_request.with_worktree(worktree_id);
        }
        let mut forked_agent = self.spawn_agent(create_request).await?;
        for grant in source_agent.extension_grants() {
            forked_agent = self
                .owned
                .agent_store
                .grant_extension(forked_agent.id(), grant.clone())?;
        }
        for substitute in source_agent.substitutes() {
            forked_agent = self
                .owned
                .agent_store
                .add_agent_substitute(forked_agent.id(), substitute.clone())?;
        }
        if source_agent.substitution_timeout_ms().is_some() {
            forked_agent = self.owned.agent_store.set_agent_substitution_timeout(
                forked_agent.id(),
                source_agent.substitution_timeout_ms(),
            )?;
        }
        if let Some(index) = source_agent.active_substitute_index() {
            if index < forked_agent.substitutes().len() {
                forked_agent = self
                    .owned
                    .agent_store
                    .activate_agent_substitute(
                        forked_agent.id(),
                        index,
                        "forked from source agent",
                    )?
                    .0;
            }
        }

        let launch_request = crate::local::LaunchProviderRunRequest {
            session_id: request.session_id.clone(),
            agent_id: Some(forked_agent.id().to_string()),
            adapter_key: source_run.adapter_key().to_string(),
            provider: source_run.provider().to_string(),
            account_profile: source_run.account_profile().to_string(),
            model: source_run.model().to_string(),
            variant: source_run.variant().map(str::to_string),
            structured_endpoint: source_run.structured_endpoint().map(str::to_string),
            provider_session_id: None,
            native_tui: !source_run.client_interface().is_chariox(),
        };
        let provider_run = self
            .launch_provider_for_fork(launch_request, caller_user_id)
            .await?;
        self.owned.prepare_agent_fork_context_handoff(
            &source_run,
            forked_agent.id(),
            &provider_run,
        );
        let session = self.session_snapshot(&request.session_id).await?;
        Ok((
            source_agent.id().to_string(),
            forked_agent,
            provider_run,
            session,
        ))
    }

    fn resolve_action_agent(
        &self,
        session_id: &str,
        agent_ref: Option<&str>,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let agents = self.owned.agent_store.get_session_agents(session_id);
        let reference = agent_ref.map(str::trim).filter(|value| !value.is_empty());
        let Some(reference) = reference else {
            let Some(focused_agent_id) = session.focused_agent_id() else {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "agent reference is required because no agent is focused".to_string(),
                });
            };
            return agents
                .into_iter()
                .find(|agent| agent.id() == focused_agent_id)
                .ok_or_else(|| DaemonError::AgentNotInSession {
                    session_id: session_id.to_string(),
                    agent_id: focused_agent_id.to_string(),
                });
        };
        let matches = agents
            .into_iter()
            .filter(|agent| {
                agent.id() == reference
                    || agent.agent_ref() == reference
                    || agent.alias() == Some(reference)
                    || agent.id().starts_with(reference)
                    || agent.agent_ref().starts_with(reference)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [agent] => Ok(agent.clone()),
            [] => Err(DaemonError::LocalTransport {
                operation,
                message: format!("agent `{reference}` not found in session `{session_id}`"),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation,
                message: format!("agent reference `{reference}` is ambiguous"),
            }),
        }
    }

    fn ensure_action_agent_owner(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        if agent.owner_user_id() != caller_user_id {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: caller_user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("agent `{}`", agent.id()),
                operation,
            });
        }
        Ok(())
    }

    async fn launch_provider_for_fork(
        &self,
        request: crate::local::LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        if let Some(response) = self
            .launch_remote_native_provider_run(&request, &caller_user_id)
            .await?
        {
            return match response {
                crate::local::LocalDaemonResponse::ProviderRunLaunched { provider_run } => {
                    Ok(provider_run)
                }
                other => Err(DaemonError::LocalTransport {
                    operation: "agent fork",
                    message: format!("unexpected remote provider launch response: {other:?}"),
                }),
            };
        }
        let start_outcome = self.start_provider_launch(request, caller_user_id).await?;
        let (started, runtime_init_delay_ms) = match start_outcome {
            ProviderLaunchStartOutcome::Reused(provider_run) => return Ok(provider_run),
            ProviderLaunchStartOutcome::Started(started, runtime_init_delay_ms) => {
                (started, runtime_init_delay_ms)
            }
        };
        let accepted = started.run.clone();
        let state = self.clone();
        tokio::spawn(async move {
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let run = started.run.clone();
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize forked provider runtime",
                message: error.to_string(),
            });
            match binding {
                Ok(Ok(binding)) => state.finish_provider_launch(&started, binding).await,
                Ok(Err(error)) => state.fail_provider_launch(&started, &error).await,
                Err(error) => state.fail_provider_launch(&started, &error).await,
            }
        });
        Ok(accepted)
    }
}
