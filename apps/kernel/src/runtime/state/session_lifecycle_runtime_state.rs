use super::*;

type ProviderProcessRemovalResult = (
    String,
    Result<(bool, Option<String>), crate::error::DaemonError>,
);

pub(crate) struct AgentOutputSeenAck {
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) changed: bool,
}

impl KernelRuntimeState {
    pub(crate) async fn create_session_response(
        &self,
        mut request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let slice_ref = request.slice_ref.clone();
        let kernel_ref = request.kernel_ref.clone();
        if slice_ref.is_some() && kernel_ref.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "session.create",
                message: "use either kernel_ref or slice_ref, not both".to_string(),
            });
        }
        if request.metaagent {
            return Err(DaemonError::LocalTransport {
                operation: "session.create",
                message: "creating separate metaagents is deprecated; create a regular session and send `/meta <task>` to enter meta mode".to_string(),
            });
        }
        if slice_ref.is_none() && kernel_ref.is_none() {
            request = prepare_local_session_worktree_placement(request)?;
        }
        request = canonicalize_session_workspace(request);
        let existing_project_ids = self
            .owned
            .session_store
            .durable_projects()
            .into_iter()
            .map(|project| project.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if matches!(
            request.project_selection,
            crate::session::SessionProjectSelection::Default
        ) {
            let name = crate::runtime::workspace_git_common::workspace_display_label(
                &request.workspace_id,
            );
            request = request.with_default_project_name_hint(name);
        }
        if let Some(slice_ref) = slice_ref.as_deref() {
            let slice = self
                .ensure_slice_worktree_scope(slice_ref, &request.workspace_id, &request.worktree_id)
                .await?;
            request = codex_linux_slice_live_sync_request(request, &slice)?;
            self.wait_for_slice_worker_ready(slice_ref, session_request_provider(&request))
                .await?;
        }
        let response = if let Some(slice_ref) = slice_ref.as_deref() {
            let worker_kernel_ref = self.resolve_slice_worker_kernel_ref(slice_ref).await?;
            self.create_sliced_session_response(request, worker_kernel_ref)
                .await?
        } else if let Some(kernel_ref) = kernel_ref {
            self.create_remote_session_response(request, kernel_ref)
                .await?
        } else {
            self.owned.create_session_response(request)?
        };
        if let LocalDaemonResponse::SessionCreated { session, agent } = &response {
            let project = if session.is_hidden() {
                None
            } else {
                Some(self.owned.session_store.get_project(session.project_id())?)
            };
            if let Some(project) = project
                .as_ref()
                .filter(|project| !existing_project_ids.contains(project.id()))
            {
                self.owned.durable_state_store.append_event(
                    "project.created",
                    Some(project.id().to_string()),
                    serde_json::json!({ "project": project }),
                )?;
            }
            if let Some(slice_ref) = slice_ref {
                let session_id = session.id().to_string();
                let agent_id = agent.id().to_string();
                let slice = self
                    .with_app_side_effect(move |app| {
                        app.slices().attach_agent(
                            &slice_ref,
                            &session_id,
                            &agent_id,
                            crate::session::unix_epoch_ms(),
                        )
                    })
                    .await?;
                self.owned.durable_state_store.append_event(
                    "slice.updated",
                    Some(slice.id.clone()),
                    serde_json::json!({ "slice": &slice }),
                )?;
            }
            let mut payload = serde_json::json!({
                "session": session,
                "default_agent": agent,
            });
            if let Some(project) = project {
                payload["project"] = serde_json::json!(project);
            }
            self.owned.durable_state_store.append_event(
                "session.created",
                Some(session.id().to_string()),
                payload,
            )?;
        }
        Ok(response)
    }

    async fn create_remote_session_response(
        &self,
        mut request: crate::session::CreateSessionRequest,
        worker_kernel_ref: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create remote session started",
            serde_json::json!({
                "workspace_id": request.workspace_id,
                "worktree_id": request.worktree_id,
                "worker_kernel_ref": worker_kernel_ref,
                "has_alias": request.alias.as_ref().is_some_and(|alias| !alias.trim().is_empty()),
                "has_agent_defaults": request.agent_defaults.is_some(),
            }),
        );
        let worktree_placement = request.worktree_placement.clone();
        request.kernel_ref = None;
        request.worktree_placement = None;
        let mut session =
            SessionStateOwner::new(self.owned.session_store.clone()).create_session(request)?;
        let mut unpublished_session =
            UnpublishedSessionGuard::new(&self.owned.session_store, session.id());
        let mut agent_request =
            session::agent_request_from_session_defaults(&session, Some(session.owner_user_id()))
                .with_worktree(session.worktree_id())
                .with_kernel(worker_kernel_ref);
        if let Some(placement) = worktree_placement {
            agent_request = agent_request.with_worktree_placement(placement);
        }
        let agent = self.spawn_agent(agent_request).await?;
        self.owned
            .session_store
            .write()
            .set_focused_agent(session.id(), Some(agent.id().to_string()))?;
        session = self.owned.session_snapshot(session.id())?;
        unpublished_session.publish();
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create remote session completed",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_count": session.agents().len(),
            }),
        );
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    async fn wait_for_slice_worker_ready(
        &self,
        slice_ref: &str,
        provider: Option<&str>,
    ) -> Result<(), DaemonError> {
        const ATTEMPTS: usize = 40;
        const DELAY_MS: u64 = 250;
        for attempt in 0..ATTEMPTS {
            let slice = self.resolve_slice(slice_ref)?;
            if slice_worker_ready(&slice, provider) {
                return Ok(());
            }
            if attempt == 0 {
                crate::logging::info_with_fields(
                    "daemon.kernel_session",
                    "slice worker warming up",
                    serde_json::json!({
                        "slice_ref": slice_ref,
                        "provider": provider,
                        "worker_kernel_id": slice.worker_kernel_id,
                        "providers": slice.providers,
                    }),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
        }
        let slice = self.resolve_slice(slice_ref)?;
        Err(DaemonError::LocalTransport {
            operation: "session.create",
            message: format!(
                "slice worker warming up timed out for `{}`; worker={}, providers={}",
                slice.name,
                slice.worker_kernel_id.as_deref().unwrap_or("missing"),
                if slice.providers.is_empty() {
                    "missing".to_string()
                } else {
                    slice.providers.join(",")
                }
            ),
        })
    }

    async fn create_sliced_session_response(
        &self,
        mut request: crate::session::CreateSessionRequest,
        worker_kernel_ref: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create sliced session started",
            serde_json::json!({
                "workspace_id": request.workspace_id,
                "worktree_id": request.worktree_id,
                "slice_ref": request.slice_ref,
                "worker_kernel_ref": worker_kernel_ref,
                "has_alias": request.alias.as_ref().is_some_and(|alias| !alias.trim().is_empty()),
                "has_agent_defaults": request.agent_defaults.is_some(),
            }),
        );
        let worktree_placement = request.worktree_placement.clone();
        request.slice_ref = None;
        request.kernel_ref = None;
        request.worktree_placement = None;
        let mut session =
            SessionStateOwner::new(self.owned.session_store.clone()).create_session(request)?;
        let mut unpublished_session =
            UnpublishedSessionGuard::new(&self.owned.session_store, session.id());
        let mut agent_request =
            session::agent_request_from_session_defaults(&session, Some(session.owner_user_id()))
                .with_worktree(session.worktree_id())
                .with_kernel(worker_kernel_ref);
        if let Some(placement) = worktree_placement {
            agent_request = agent_request.with_worktree_placement(placement);
        }
        let agent = self.spawn_agent(agent_request).await?;
        self.owned
            .session_store
            .write()
            .set_focused_agent(session.id(), Some(agent.id().to_string()))?;
        session = self.owned.session_snapshot(session.id())?;
        unpublished_session.publish();
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create sliced session completed",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_count": session.agents().len(),
            }),
        );
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    pub(crate) async fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let attachment = self.owned.attach(request)?;
        let runtime_state = self.clone();
        let app = Arc::clone(&self.app);
        let session_id = attachment.session_id().to_string();
        tokio::spawn(async move {
            crate::runtime::external_provider_session_control::refresh_attached_external_provider_histories_for_session(
                &app,
                Some(&runtime_state),
                &session_id,
            )
            .await;
            if let Err(error) = runtime_state.owned.session_snapshot(&session_id) {
                crate::logging::warn_with_fields(
                    "daemon.external_provider_sessions",
                    "failed to refresh session projection after external provider attach catch-up",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            }
        });
        Ok(attachment)
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.owned.detach(attachment_id)
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned.focus_agent(session_id, agent_id, caller_user_id)
    }

    pub(crate) async fn acknowledge_agent_output_seen(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<AgentOutputSeenAck, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let (session, changed) = {
            // Read, mutate, and replace while holding one write guard. The previous
            // clone -> durable append -> restore sequence could overwrite a provider
            // or workflow transition that landed while the large session snapshot
            // was being serialized.
            let mut sessions = self.owned.session_store.write();
            let mut session = sessions.get_session(session_id)?;
            let collaboration_level = session
                .collaboration_level_for_user(caller_user_id)
                .unwrap_or(crate::session::CollaborationLevel::Private);
            if agent.owner_user_id() != caller_user_id
                && !collaboration_level.can_view_agent_trace()
            {
                return Err(DaemonError::OwnershipAccessDenied {
                    user_id: caller_user_id.to_string(),
                    owner_user_id: agent.owner_user_id().to_string(),
                    resource: agent_id.to_string(),
                    operation: "acknowledge agent output",
                });
            }
            let changed = session.acknowledge_agent_output_seen(caller_user_id, agent_id);
            if changed {
                session.touch();
                sessions.restore_session(session.clone());
            }
            (session, changed)
        };
        if !changed {
            return Ok(AgentOutputSeenAck { session, changed });
        }
        self.append_session_durable_event(
            "session.updated",
            &session,
            "agent_output_seen_acknowledged",
        )
        .await?;
        Ok(AgentOutputSeenAck {
            session: self.owned.session_snapshot(session_id)?,
            changed,
        })
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        self.owned.cycle_agent_focus(session_id, caller_user_id)
    }

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.alias_session(session_id, alias)
    }

    pub(crate) async fn spawn_agent(
        &self,
        mut request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.normalize_local_kernel_ref(&mut request);
        if request.kernel_ref.is_none() {
            request = self.prepare_local_agent_worktree_placement(request)?;
            return self.owned.spawn_agent(request);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).spawn_agent(request)
        })
        .await
    }

    pub(crate) async fn spawn_agents(
        &self,
        mut requests: Vec<crate::agent::CreateAgentRequest>,
        caller_user_id: &str,
    ) -> Result<Vec<crate::agent::AgentInstance>, DaemonError> {
        for request in &mut requests {
            self.normalize_local_kernel_ref(request);
        }
        if requests.iter().all(|request| request.kernel_ref.is_none()) {
            let mut prepared_requests = Vec::with_capacity(requests.len());
            for request in requests {
                prepared_requests.push(self.prepare_local_agent_worktree_placement(request)?);
            }
            return self.owned.spawn_agents(prepared_requests);
        }

        let mut ordered_agents = vec![None; requests.len()];
        let mut local_requests = Vec::new();
        let mut local_indices = Vec::new();
        for (index, request) in requests.into_iter().enumerate() {
            if request.kernel_ref.is_none() {
                local_requests.push(self.prepare_local_agent_worktree_placement(request)?);
                local_indices.push(index);
            } else {
                ordered_agents[index] = Some(self.spawn_agent(request).await?);
            }
        }
        if !local_requests.is_empty() {
            let local_agents = self.owned.spawn_agents(local_requests)?;
            for (index, agent) in local_indices.into_iter().zip(local_agents.into_iter()) {
                ordered_agents[index] = Some(agent);
            }
        }
        let agents = ordered_agents
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .expect("every batch spawn slot should be populated");
        if let Some(last_agent) = agents.last() {
            self.owned
                .focus_agent(last_agent.session_id(), last_agent.id(), caller_user_id)?;
        }
        Ok(agents)
    }

    fn normalize_local_kernel_ref(&self, request: &mut crate::agent::CreateAgentRequest) {
        let Some(kernel_ref) = request.kernel_ref.as_deref() else {
            return;
        };
        let config = self.owned.config_projection.snapshot();
        if kernel_ref_matches_local_config(&config, kernel_ref) {
            request.kernel_ref = None;
        }
    }

    fn prepare_local_agent_worktree_placement(
        &self,
        mut request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::CreateAgentRequest, DaemonError> {
        let Some(placement) = request.worktree_placement.take() else {
            return Ok(request);
        };
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let base_worktree = request
            .worktree_id
            .as_deref()
            .unwrap_or_else(|| session.worktree_id());
        let resolved = crate::git_worktree_placement::prepare_git_worktree(
            &placement,
            std::path::Path::new(base_worktree),
            request.worktree_id.as_deref(),
            "agent.spawn",
        )?;
        request.worktree_id = Some(resolved);
        Ok(request)
    }

    pub(crate) async fn resolve_slice_worker_kernel_ref(
        &self,
        slice_ref: &str,
    ) -> Result<String, DaemonError> {
        let slice_ref = slice_ref.to_string();
        self.with_app_side_effect(move |app| app.slices().resolve_worker_kernel_ref(&slice_ref))
            .await
    }

    pub(crate) async fn ensure_slice_worktree_scope(
        &self,
        slice_ref: &str,
        workspace_id: &str,
        worktree_id: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice_ref = slice_ref.to_string();
        let workspace_id = workspace_id.to_string();
        let worktree_id = worktree_id.to_string();
        self.with_app_side_effect(move |app| {
            app.slices()
                .ensure_worktree_scope(&slice_ref, Some(&workspace_id), Some(&worktree_id))
        })
        .await
    }

    pub(crate) async fn attach_slice_agent(
        &self,
        slice_ref: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice_ref = slice_ref.to_string();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        let slice = self
            .with_app_side_effect(move |app| {
                app.slices().attach_agent(
                    &slice_ref,
                    &session_id,
                    &agent_id,
                    crate::session::unix_epoch_ms(),
                )
            })
            .await?;
        self.owned.durable_state_store.append_event(
            "slice.updated",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        Ok(slice)
    }

    pub(crate) async fn attach_slice_agents(
        &self,
        attachments: Vec<crate::slice::SliceAgentAttachment>,
    ) -> Result<Vec<crate::slice::SliceRecord>, DaemonError> {
        if attachments.is_empty() {
            return Ok(Vec::new());
        }
        let slices = self
            .with_app_side_effect(move |app| {
                app.slices()
                    .attach_agents(attachments, crate::session::unix_epoch_ms())
            })
            .await?;
        for slice in &slices {
            self.owned.durable_state_store.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": slice }),
            )?;
        }
        Ok(slices)
    }

    pub(crate) async fn move_agent_to_remote(
        &self,
        session_id: &str,
        agent_ref: &str,
        machine_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let local_agent =
            self.owned
                .ensure_agent_ref_owner(agent_ref, caller_user_id, "move agent to remote")?;
        let terminated_run_ids = self
            .owned
            .terminate_idle_provider_runs_for_agent_before_remote_move(session_id, &local_agent)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        let target_slice_id = self
            .owned
            .slice_store
            .resolve_by_worker_kernel_ref(machine_ref)
            .map(|slice| slice.id);
        let agent = self
            .with_app_side_effect(|app| {
                app.move_agent_to_remote(session_id, agent_ref, machine_ref)
            })
            .await?;
        if let Some(slice_ref) = target_slice_id {
            self.attach_slice_agent(&slice_ref, session_id, agent.id())
                .await?;
        }
        Ok(agent)
    }

    pub(crate) async fn move_agent_to_local(
        &self,
        session_id: &str,
        agent_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let remote_agent =
            self.owned
                .ensure_agent_ref_owner(agent_ref, caller_user_id, "move agent to local")?;
        let slice_ref = remote_agent
            .remote_execution()
            .and_then(|remote| {
                self.owned
                    .slice_store
                    .resolve_by_worker_kernel_ref(&remote.worker_kernel_id)
                    .or_else(|| {
                        self.owned
                            .slice_store
                            .resolve_by_worker_kernel_ref(&remote.worker_machine_id)
                    })
            })
            .map(|slice| slice.id);
        self.owned
            .terminate_idle_remote_provider_projection_for_agent_before_local_move(
                session_id,
                &remote_agent,
            )?;
        let agent = self
            .with_app_side_effect(|app| app.move_agent_to_local(session_id, agent_ref))
            .await?;
        if let Some(slice_ref) = slice_ref {
            let slice = self.owned.slice_store.detach_agent(
                &slice_ref,
                agent.id(),
                crate::session::unix_epoch_ms(),
            )?;
            self.owned.durable_state_store.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": &slice }),
            )?;
        }
        let _ = self.owned.session_snapshot(session_id)?;
        Ok(agent)
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let local_provider_run_ids = if agent.remote_execution().is_none() {
            self.owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|run| run.agent_instance_id() == Some(agent_id))
                .map(|run| run.id().to_string())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Slice membership is the durable attachment authority. A worker kernel id can
        // legitimately be stale after a slice or home-kernel restart, so inferring the
        // attachment from remote execution would leave a deleted agent pinned to its
        // slice forever.
        let slice_refs = self
            .owned
            .slice_store
            .list()
            .into_iter()
            .filter(|slice| slice.agent_ids.iter().any(|value| value == agent_id))
            .map(|slice| slice.id)
            .collect::<Vec<_>>();
        self.owned
            .ensure_agent_owner(agent.id(), caller_user_id, "destroy agent")?;
        let destroyed = if agent.remote_execution().is_none() {
            self.owned.destroy_agent(agent_id, caller_user_id)?
        } else {
            let destroyed = self
                .with_app_side_effect(|app| {
                    crate::app::KernelSessionService::new(app).destroy_agent(agent_id)
                })
                .await?;
            self.owned.destroy_agent(agent_id, caller_user_id)?;
            destroyed
        };
        for slice_ref in slice_refs {
            let slice = self.owned.slice_store.detach_agent(
                &slice_ref,
                destroyed.id(),
                crate::session::unix_epoch_ms(),
            )?;
            self.owned.durable_state_store.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": &slice }),
            )?;
        }
        if agent.remote_execution().is_none() {
            self.remove_destroyed_agent_provider_processes(local_provider_run_ids);
            self.append_agent_durable_event("agent.deleted", &destroyed, None)
                .await?;
        }
        Ok(destroyed)
    }

    fn remove_destroyed_agent_provider_processes(&self, provider_run_ids: Vec<String>) {
        if provider_run_ids.is_empty() {
            return;
        }
        let immediate_provider_run_ids = provider_run_ids.clone();
        if let Some(results) = self.try_with_app_side_effect(move |app| {
            Self::remove_destroyed_agent_provider_processes_from_app(
                app,
                &immediate_provider_run_ids,
            )
        }) {
            self.finish_destroyed_agent_provider_process_removal(results);
            return;
        }

        let state = self.clone();
        tokio::spawn(async move {
            let results = state
                .with_app_side_effect(move |app| {
                    Self::remove_destroyed_agent_provider_processes_from_app(app, &provider_run_ids)
                })
                .await;
            state.finish_destroyed_agent_provider_process_removal(results);
        });
    }

    fn remove_destroyed_agent_provider_processes_from_app(
        app: &mut crate::app::DaemonApp,
        provider_run_ids: &[String],
    ) -> Vec<ProviderProcessRemovalResult> {
        provider_run_ids
            .iter()
            .map(|provider_run_id| {
                (
                    provider_run_id.clone(),
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id),
                )
            })
            .collect()
    }

    fn finish_destroyed_agent_provider_process_removal(
        &self,
        results: Vec<ProviderProcessRemovalResult>,
    ) {
        for (provider_run_id, result) in results {
            match result {
                Ok((_, process_key)) => self
                    .owned
                    .remove_provider_process_tracking_for_run(&provider_run_id, process_key),
                Err(error) => crate::logging::error_with_fields(
                    "daemon.provider_process_gc",
                    "failed to remove provider process for destroyed agent",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                ),
            }
        }
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids) = owned.end_session(session_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        self.append_session_durable_event("session.ended", &session, "runtime_end_session")
            .await?;
        self.detach_session_slices(&session).await?;
        Ok(session)
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids, removed_project) =
            owned.delete_session_ref(session_ref, workspace_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        self.append_session_durable_event("session.deleted", &session, "runtime_delete_session")
            .await?;
        if let Some(project) = removed_project {
            self.append_project_durable_event("project.deleted", &project)?;
        }
        self.detach_session_slices(&session).await?;
        Ok(session)
    }

    async fn detach_session_slices(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Result<(), DaemonError> {
        let session_id = session.id();
        let attached_slices = self.owned.slice_store.list_by_session(session_id);
        for slice in attached_slices {
            let mut detached = slice;
            for agent in session.agents() {
                detached = self.owned.slice_store.detach_agent(
                    &detached.id,
                    agent.id(),
                    crate::session::unix_epoch_ms(),
                )?;
            }
            let detached = self.owned.slice_store.detach_session(
                &detached.id,
                session_id,
                crate::session::unix_epoch_ms(),
            )?;
            self.owned.durable_state_store.append_event(
                "slice.updated",
                Some(detached.id.clone()),
                serde_json::json!({ "slice": &detached }),
            )?;
        }
        Ok(())
    }

    pub(crate) async fn delete_current_kernel_sessions(
        &self,
    ) -> Result<Vec<crate::session::RuntimeSession>, DaemonError> {
        let session_ids: Vec<String> = self
            .owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .map(|session| session.id().to_string())
            .collect();
        let mut deleted_sessions = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let session = self.delete_session_ref(&session_id, None).await?;
            deleted_sessions.push(session);
        }
        Ok(deleted_sessions)
    }
}

struct UnpublishedSessionGuard {
    store: crate::session::SessionStateStore,
    session_id: String,
    published: bool,
}

impl UnpublishedSessionGuard {
    fn new(store: &crate::session::SessionStateStore, session_id: &str) -> Self {
        Self {
            store: store.clone(),
            session_id: session_id.to_string(),
            published: false,
        }
    }

    fn publish(&mut self) {
        self.published = true;
    }
}

impl Drop for UnpublishedSessionGuard {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        if let Err(error) = self
            .store
            .write()
            .delete_session_with_project_cleanup(&self.session_id)
        {
            crate::logging::warn_with_fields(
                "daemon.kernel_session",
                "failed to roll back unpublished session",
                serde_json::json!({
                    "session_id": self.session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

fn prepare_local_session_worktree_placement(
    mut request: crate::session::CreateSessionRequest,
) -> Result<crate::session::CreateSessionRequest, DaemonError> {
    let Some(placement) = request.worktree_placement.take() else {
        return Ok(request);
    };
    let resolved = crate::git_worktree_placement::prepare_git_worktree(
        &placement,
        std::path::Path::new(&request.worktree_id),
        None,
        "session.create",
    )?;
    request.worktree_id = resolved;
    Ok(request)
}

fn canonicalize_session_workspace(
    mut request: crate::session::CreateSessionRequest,
) -> crate::session::CreateSessionRequest {
    let Some(workspace_id) = crate::runtime::workspace_git_common::canonical_workspace_path(
        &request.workspace_id,
        &request.worktree_id,
    ) else {
        return request;
    };
    if workspace_id != request.workspace_id {
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "canonicalized session workspace from linked worktree",
            serde_json::json!({
                "requested_workspace_id": request.workspace_id,
                "worktree_id": request.worktree_id,
                "workspace_id": workspace_id,
            }),
        );
        request.workspace_id = workspace_id;
    }
    request
}

fn codex_linux_slice_live_sync_request(
    request: crate::session::CreateSessionRequest,
    slice: &crate::slice::SliceRecord,
) -> Result<crate::session::CreateSessionRequest, DaemonError> {
    if !is_codex_linux_local_docker_slice_session(&request, slice) {
        return Ok(request);
    }
    if request.workspace_live_sync_mode == Some(crate::config::WorkspaceLiveSyncMode::Managed) {
        return Err(DaemonError::LocalTransport {
            operation: "session.create",
            message: "Codex Linux slice sessions do not support managed workspace live sync yet; use tracked live sync for this slice".to_string(),
        });
    }
    Ok(request)
}

fn is_codex_linux_local_docker_slice_session(
    request: &crate::session::CreateSessionRequest,
    slice: &crate::slice::SliceRecord,
) -> bool {
    slice.backend == crate::slice::SliceBackendKind::LocalDocker
        && slice.os.eq_ignore_ascii_case("linux")
        && request
            .agent_defaults
            .as_ref()
            .map(|defaults| defaults.provider.as_str())
            .is_some_and(|provider| provider == "codex")
}

fn session_request_provider(request: &crate::session::CreateSessionRequest) -> Option<&str> {
    request
        .agent_defaults
        .as_ref()
        .map(|defaults| defaults.provider.as_str())
        .filter(|provider| !provider.trim().is_empty() && *provider != "default")
}

fn kernel_ref_matches_local_config(config: &crate::config::DaemonConfig, kernel_ref: &str) -> bool {
    let kernel_ref = kernel_ref.trim();
    !kernel_ref.is_empty()
        && (config.daemon_id == kernel_ref || config.daemon_alias.as_deref() == Some(kernel_ref))
}

fn slice_worker_ready(slice: &crate::slice::SliceRecord, provider: Option<&str>) -> bool {
    if slice.worker_kernel_id.as_deref().is_none_or(str::is_empty) {
        return false;
    }
    let Some(provider) = provider else {
        return true;
    };
    slice
        .providers
        .iter()
        .any(|candidate| candidate == provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_session_worktree_placement_creates_git_worktree_before_storing_session() {
        let repo = temp_git_repo("session-placement");
        let target = repo.with_file_name(format!(
            "{}-feature",
            repo.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        let request =
            crate::session::CreateSessionRequest::new("workspace", repo.display().to_string())
                .with_worktree_placement(crate::agent::GitWorktreePlacement {
                    target_directory: Some(target.display().to_string()),
                    branch: Some("feature/session-placement".to_string()),
                    from_ref: Some("HEAD".to_string()),
                });

        let adjusted = prepare_local_session_worktree_placement(request)
            .expect("session placement should create git worktree");

        assert_eq!(adjusted.worktree_id, target.display().to_string());
        assert!(target.is_dir());
        assert_eq!(
            git_output(&target, &["branch", "--show-current"]),
            "feature/session-placement"
        );
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn linked_worktree_workspace_is_canonicalized_to_the_main_repository() {
        let repo = temp_git_repo("session-workspace-canonicalization");
        let worktree = repo.with_file_name(format!(
            "{}-linked",
            repo.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feature/session-workspace-canonicalization",
                worktree.to_str().unwrap(),
            ],
        );
        let request = crate::session::CreateSessionRequest::new(
            worktree.display().to_string(),
            worktree.display().to_string(),
        );

        let adjusted = canonicalize_session_workspace(request);

        assert_eq!(
            adjusted.workspace_id,
            std::fs::canonicalize(&repo).unwrap().display().to_string()
        );
        assert_eq!(adjusted.worktree_id, worktree.display().to_string());
        let _ = std::fs::remove_dir_all(&worktree);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn codex_linux_slice_sessions_keep_live_sync_off_by_default() {
        let request = crate::session::CreateSessionRequest::new("workspace", "worktree")
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("codex"))
            .with_slice_ref("linux-slice");

        let adjusted = codex_linux_slice_live_sync_request(request, &slice("linux"))
            .expect("codex linux slice should be adjusted");

        assert_eq!(adjusted.workspace_live_sync_mode, None);
    }

    #[test]
    fn codex_linux_slice_sessions_reject_explicit_managed_live_sync() {
        let request = crate::session::CreateSessionRequest::new("workspace", "worktree")
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("codex"))
            .with_slice_ref("linux-slice")
            .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Managed);

        let error = codex_linux_slice_live_sync_request(request, &slice("linux"))
            .expect_err("managed codex linux slice should be rejected");

        assert!(error
            .to_string()
            .contains("do not support managed workspace live sync"));
    }

    #[test]
    fn codex_linux_slice_sessions_keep_explicit_tracked_live_sync() {
        let request = crate::session::CreateSessionRequest::new("workspace", "worktree")
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("codex"))
            .with_slice_ref("linux-slice")
            .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked);

        let adjusted = codex_linux_slice_live_sync_request(request, &slice("linux"))
            .expect("codex linux slice should accept tracked mode");

        assert_eq!(
            adjusted.workspace_live_sync_mode,
            Some(crate::config::WorkspaceLiveSyncMode::Tracked)
        );
    }

    #[test]
    fn non_codex_linux_slice_sessions_keep_requested_live_sync() {
        let request = crate::session::CreateSessionRequest::new("workspace", "worktree")
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("opencode"))
            .with_slice_ref("linux-slice");

        let adjusted = codex_linux_slice_live_sync_request(request, &slice("linux"))
            .expect("non-codex slice should not be adjusted");

        assert_eq!(adjusted.workspace_live_sync_mode, None);
    }

    #[test]
    fn slice_worker_readiness_requires_worker_presence_and_requested_provider() {
        let mut ready = slice("linux");
        assert!(slice_worker_ready(&ready, Some("codex")));
        assert!(slice_worker_ready(&ready, None));

        ready.worker_kernel_id = None;
        assert!(!slice_worker_ready(&ready, Some("codex")));

        ready.worker_kernel_id = Some("kernel-worker".to_string());
        assert!(!slice_worker_ready(&ready, Some("opencode")));
    }

    #[test]
    fn unpublished_remote_session_guard_rolls_back_later_initialization_errors() {
        let config = crate::config::DaemonConfig::for_tests();
        let store =
            crate::session::SessionStateStore::new(crate::session::SessionService::new(&config));
        let session = SessionStateOwner::new(store.clone())
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("session should create");
        let session_id = session.id().to_string();
        let project_id = session.project_id().to_string();

        let result = {
            let _guard = UnpublishedSessionGuard::new(&store, &session_id);
            Err::<(), DaemonError>(DaemonError::LocalTransport {
                operation: "session.create",
                message: "post-spawn initialization failed".to_string(),
            })
        };

        assert!(result.is_err());
        assert!(store.read().get_session(&session_id).is_err());
        assert!(store.read().get_project(&project_id).is_err());
    }

    fn slice(os: &str) -> crate::slice::SliceRecord {
        crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-slice".to_string(),
            owner_kernel_id: "kernel-home".to_string(),
            owner_machine_id: "machine-home".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: os.to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace".to_string()),
            worktree_id: Some("worktree".to_string()),
            workspace_mount: None,
            development: None,
            development_storage_root: None,
            development_publication: None,
            worker_kernel_ref: "slice:linux-slice".to_string(),
            worker_kernel_id: Some("kernel-worker".to_string()),
            worker_machine_id: Some("machine-worker".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn temp_git_repo(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "chariox-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("temp repo should be created");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "tests@example.invalid"]);
        run_git(&root, &["config", "user.name", "Chariox Tests"]);
        std::fs::write(root.join("README.md"), "worktree placement\n")
            .expect("fixture file should be written");
        run_git(&root, &["add", "README.md"]);
        run_git(&root, &["commit", "-m", "initial"]);
        root
    }

    fn git_output(cwd: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
