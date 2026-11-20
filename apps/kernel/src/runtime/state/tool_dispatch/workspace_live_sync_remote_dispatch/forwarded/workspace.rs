//! Resolve the home coordination identity shared by dispatch and finalization.
use super::*;
use crate::io::WorkspaceIdentity;
use crate::slice::{SliceBackendKind, SliceStatus};
use crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext;
use std::path::Path;

impl KernelRuntimeState {
    pub(super) async fn forwarded_workspace_context(
        &self,
        context: &RemoteWorkspaceLiveSyncContext,
    ) -> Result<Option<WorkspaceLiveSyncWorkspaceContext>, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let root = PathBuf::from(session.worktree_id());
        let home = workspace_live_sync_identity_for_session_workspace_link(
            workspace_identity_for_root_off_thread(root.clone()).await?,
            &session,
            &root,
        );
        let worker = workspace_live_sync_identity_for_session_workspace_link(
            context.worker_workspace_identity.clone(),
            &session,
            Path::new(&context.worker_worktree_path),
        );
        let resolved = if workspace_live_sync_workspace_identities_match(&home, &worker) {
            Some((root, worker))
        } else if self
            .plain_slice_mount_matches(context, &home, &worker)
            .await?
        {
            // Host and slice actors use a single coordination key for this mount.
            Some((PathBuf::from(&home.worktree_root_fingerprint), home))
        } else {
            None
        };
        // The lease or provider run can change while filesystem inspection is in flight.
        self.authorize_forwarded_workspace_context(context)?;
        Ok(
            resolved.map(|(root, identity)| WorkspaceLiveSyncWorkspaceContext {
                root,
                identity,
                generation: 0,
                identity_changed: false,
                valid: true,
            }),
        )
    }

    async fn plain_slice_mount_matches(
        &self,
        context: &RemoteWorkspaceLiveSyncContext,
        home: &WorkspaceIdentity,
        worker: &WorkspaceIdentity,
    ) -> Result<bool, DaemonError> {
        if *home != WorkspaceIdentity::local(&home.worktree_root_fingerprint)
            || context.worker_worktree_path != "/workspace"
            || *worker != WorkspaceIdentity::local("/workspace")
        {
            return Ok(false);
        }
        let owner = self.owned.config_projection.snapshot();
        let mounts: Vec<_> = self
            .owned
            .slice_store
            .list_by_session(&context.home_session_id)
            .into_iter()
            .filter(|slice| {
                slice.backend == SliceBackendKind::LocalDocker
                    && slice.status == SliceStatus::Running
                    && slice.owner_kernel_id == owner.daemon_id
                    && slice.owner_machine_id == owner.host_machine_id
                    && slice.worker_kernel_id.as_deref() == Some(context.worker_kernel_id.as_str())
                    && slice.worker_machine_id.as_deref()
                        == Some(context.worker_machine_id.as_str())
                    && slice.agent_ids.contains(&context.home_agent_id)
            })
            .filter_map(|slice| slice.workspace_mount)
            .collect();
        if mounts.is_empty() {
            return Ok(false);
        }
        let root = PathBuf::from(&home.worktree_root_fingerprint);
        tokio::task::spawn_blocking(move || {
            mounts.into_iter().any(|mount| {
                Path::new(&mount)
                    .canonicalize()
                    .is_ok_and(|mount| mount == root)
            })
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "inspect managed slice workspace mount",
            message: error.to_string(),
        })
    }
}
