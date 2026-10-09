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
            // App bindings take the checked, audited binding path.
            if grant.kind == crate::extension::ExtensionKind::App {
                if let Some(agent) = self
                    .copy_agent_app_grant(forked_agent.id(), grant.clone(), &caller_user_id)
                    .await?
                {
                    forked_agent = agent;
                }
                continue;
            }
            forked_agent = self
                .owned
                .agent_store
                .grant_extension(forked_agent.id(), grant.clone())?;
        }
        // The fork is the person's new focus agent: like a person's spawn, it
        // gets the App of the Room's focused App Tab.
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if self
            .bind_foreground_app(&request.session_id)
            .await
            .as_deref()
            == Some(forked_agent.id())
        {
            forked_agent = self.owned.agent_store.get_agent(forked_agent.id())?;
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
        let matches = agents_named(agents, reference);
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
                crate::local::LocalDaemonResponse::ProviderRunLaunched { provider_run } => self
                    .owned
                    .provider_run_projection
                    .get(provider_run.id())
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: provider_run.id().to_owned(),
                    }),
                other => Err(DaemonError::LocalTransport {
                    operation: "agent fork",
                    message: format!("unexpected remote provider launch response: {other:?}"),
                }),
            };
        }
        let start_outcome = self.start_provider_launch(request, caller_user_id).await?;
        let (started, runtime_init_delay_ms) = match start_outcome {
            ProviderLaunchStartOutcome::WaitingForLogin(provider_run)
            | ProviderLaunchStartOutcome::Reused(provider_run) => return Ok(provider_run),
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

/// The agents a reference names: those whose id, ref or alias is exactly the
/// reference, otherwise those whose id or ref starts with it. `agent-1` names
/// agent-1 even when agent-10 and agent-17 exist.
fn agents_named(
    agents: Vec<crate::agent::AgentInstance>,
    reference: &str,
) -> Vec<crate::agent::AgentInstance> {
    let (exact, others): (Vec<_>, Vec<_>) = agents.into_iter().partition(|agent| {
        agent.id() == reference
            || agent.agent_ref() == reference
            || agent.alias() == Some(reference)
    });
    if !exact.is_empty() {
        return exact;
    }
    others
        .into_iter()
        .filter(|agent| {
            agent.id().starts_with(reference) || agent.agent_ref().starts_with(reference)
        })
        .collect()
}

#[cfg(test)]
mod agent_reference_tests {
    use super::agents_named;

    fn agent(id: &str, agent_ref: &str, alias: Option<&str>) -> crate::agent::AgentInstance {
        crate::agent::AgentInstance::new(
            id,
            agent_ref,
            "session",
            alias.map(str::to_owned),
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )
    }

    fn named(reference: &str) -> Vec<String> {
        let agents = vec![
            agent("agent-1", "13821dea", None),
            agent("agent-10", "e443b972", Some("stub-deploy")),
            agent("agent-17", "2126576e", Some("fresh-todo")),
        ];
        agents_named(agents, reference)
            .iter()
            .map(|agent| agent.id().to_owned())
            .collect()
    }

    #[test]
    fn an_exact_agent_id_ref_or_alias_wins_over_prefixes() {
        assert_eq!(named("agent-1"), ["agent-1"]);
        assert_eq!(named("13821dea"), ["agent-1"]);
        assert_eq!(named("fresh-todo"), ["agent-17"]);
        // Prefixes still resolve when nothing matches exactly.
        assert_eq!(named("1382"), ["agent-1"]);
        assert_eq!(named("agent-17"), ["agent-17"]);
        assert_eq!(named("agent-"), ["agent-1", "agent-10", "agent-17"]);
        assert!(named("agent-2").is_empty());
    }
}
