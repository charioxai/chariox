//! Resolve the same coordination identity at dispatch and finalization.

use super::*;

impl KernelRuntimeState {
    pub(super) fn authorize_forwarded_workspace(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    ) -> Result<(), DaemonError> {
        super::super::super::home_extension_authorizer::authorize_remote_home_context(
            self,
            &crate::transport::relay_peer::RemoteExtensionInvocationContext {
                home_kernel_id: context.home_kernel_id.clone(),
                home_session_id: context.home_session_id.clone(),
                home_agent_id: context.home_agent_id.clone(),
                leased_agent_id: context.leased_agent_id.clone(),
                worker_provider_run_id: context.worker_provider_run_id.clone(),
                worker_kernel_id: Some(context.worker_kernel_id.clone()),
                worker_machine_id: Some(context.worker_machine_id.clone()),
            },
            "forwarded workspace coordination",
        )
        .map(|_| ())
    }

    pub(super) async fn forwarded_workspace_context(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    ) -> Result<Option<WorkspaceLiveSyncWorkspaceContext>, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        self.authorize_forwarded_workspace(context)?;
        let root = PathBuf::from(session.worktree_id());
        let home = workspace_identity_for_root_off_thread(root.clone()).await?;
        // Off-thread Git inspection can complete after this run was replaced.
        self.authorize_forwarded_workspace(context)?;
        let home = workspace_live_sync_identity_for_session_workspace_link(home, &session, &root);
        let worker = workspace_live_sync_identity_for_session_workspace_link(
            context.worker_workspace_identity.clone(),
            &session,
            Path::new(&context.worker_worktree_path),
        );
        let identity = if workspace_live_sync_workspace_identities_match(&home, &worker) {
            worker
        } else if self.managed_plain_workspace_matches(context, &home, &worker)? {
            // All local and slice actors editing this host folder must share its
            // identity. Using `/workspace` would conflate unrelated slice mounts.
            home
        } else {
            return Ok(None);
        };
        Ok(Some(WorkspaceLiveSyncWorkspaceContext {
            root,
            identity,
            generation: 0,
            identity_changed: false,
            valid: true,
        }))
    }

    fn managed_plain_workspace_matches(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        home: &crate::io::WorkspaceIdentity,
        worker: &crate::io::WorkspaceIdentity,
    ) -> Result<bool, DaemonError> {
        // This is a physical mount mapping, not an exception to Git/link identity.
        if !plain_workspace(home)
            || !plain_workspace(worker)
            || context.worker_worktree_path != "/workspace"
            || worker.worktree_root_fingerprint != "/workspace"
        {
            return Ok(false);
        }
        let config = self.owned.config_projection.snapshot();
        let mapped = self
            .owned
            .slice_store
            .list_by_session(&context.home_session_id)
            .into_iter()
            .any(|slice| {
                slice.backend == crate::slice::SliceBackendKind::LocalDocker
                    && slice.status == crate::slice::SliceStatus::Running
                    && slice.owner_kernel_id == config.daemon_id
                    && slice.owner_machine_id == config.host_machine_id
                    && slice.worker_kernel_id.as_deref() == Some(context.worker_kernel_id.as_str())
                    && slice.worker_machine_id.as_deref()
                        == Some(context.worker_machine_id.as_str())
                    && slice.agent_ids.contains(&context.home_agent_id)
                    && slice
                        .workspace_mount
                        .as_deref()
                        .and_then(|mount| Path::new(mount).canonicalize().ok())
                        .is_some_and(|mount| mount == Path::new(&home.worktree_root_fingerprint))
            });
        Ok(mapped)
    }
}

fn plain_workspace(identity: &crate::io::WorkspaceIdentity) -> bool {
    identity.vcs_provider.is_none()
        && identity.repo_id.is_none()
        && identity.repo_url.is_none()
        && identity.branch.is_none()
        && identity.head_commit.is_none()
}
