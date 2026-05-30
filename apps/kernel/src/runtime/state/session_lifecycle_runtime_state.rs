use super::*;

impl KernelRuntimeState {
    pub(crate) async fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let slice_ref = request.slice_ref.clone();
        let response = if let Some(slice_ref) = slice_ref.as_deref() {
            {
                let slice_ref = slice_ref.to_string();
                let workspace_id = request.workspace_id.clone();
                let worktree_id = request.worktree_id.clone();
                self.with_app_side_effect(move |app| {
                    app.slices().ensure_worktree_scope(
                        &slice_ref,
                        Some(&workspace_id),
                        Some(&worktree_id),
                    )?;
                    Ok(())
                })
                .await?;
            }
            let worker_kernel_ref = self.resolve_slice_worker_kernel_ref(slice_ref).await?;
            self.create_sliced_session_response(request, worker_kernel_ref)
                .await?
        } else {
            self.owned.create_session_response(request)?
        };
        if let LocalDaemonResponse::SessionCreated { session, agent } = &response {
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
            self.owned.durable_state_store.append_event(
                "session.created",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": session,
                    "default_agent": agent,
                }),
            )?;
        }
        Ok(response)
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
        request.slice_ref = None;
        let mut session =
            SessionStateOwner::new(self.owned.session_store.clone()).create_session(request)?;
        let agent_request =
            session::agent_request_from_session_defaults(&session, Some(session.owner_user_id()))
                .with_worktree(session.worktree_id())
                .with_kernel(worker_kernel_ref);
        let agent = self.spawn_agent(agent_request).await?;
        self.owned
            .session_store
            .write()
            .set_focused_agent(session.id(), Some(agent.id().to_string()))?;
        session = self.owned.session_snapshot(session.id())?;
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
        self.owned.attach(request)
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
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if request.kernel_ref.is_none() {
            return self.owned.spawn_agent(request);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).spawn_agent(request)
        })
        .await
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

    pub(crate) async fn move_agent_to_remote(
        &self,
        session_id: &str,
        agent_ref: &str,
        machine_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_ref_owner(agent_ref, caller_user_id, "move agent to remote")?;
        self.with_app_side_effect(|app| {
            app.move_agent_to_remote(session_id, agent_ref, machine_ref)
        })
        .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let slice_ref = agent
            .remote_execution()
            .and_then(|remote| {
                self.owned
                    .slice_store
                    .resolve_by_worker_kernel_ref(&remote.worker_kernel_id)
            })
            .map(|slice| slice.id);
        self.owned
            .ensure_agent_owner(agent.id(), caller_user_id, "destroy agent")?;
        let destroyed = if agent.remote_execution().is_none() {
            self.owned.destroy_agent(agent_id, caller_user_id)?
        } else {
            self.with_app_side_effect(|app| {
                crate::app::KernelSessionService::new(app).destroy_agent(agent_id)
            })
            .await?
        };
        if let Some(slice_ref) = slice_ref {
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
        Ok(destroyed)
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
        let (session, terminated_run_ids) = owned.delete_session_ref(session_ref, workspace_id)?;
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
