//! Home-kernel execution for forwarded workspace live sync runtime tool calls.

use super::super::*;

mod mutation;
mod read;
mod text;

pub(super) type ForwardedWorkspaceLiveSyncResult = Result<
    (
        crate::transport::runtime_tools::RuntimeToolResult,
        Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    ),
    DaemonError,
>;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    ) -> ForwardedWorkspaceLiveSyncResult {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let home_root = PathBuf::from(session.worktree_id());
        let home_identity = workspace_identity_for_root_off_thread(home_root.clone()).await?;
        let home_identity = workspace_live_sync_identity_for_session_workspace_link(
            home_identity,
            &session,
            &home_root,
        );
        let worker_identity = workspace_live_sync_identity_for_session_workspace_link(
            context.worker_workspace_identity.clone(),
            &session,
            std::path::Path::new(&context.worker_worktree_path),
        );
        if !workspace_live_sync_workspace_identities_match(&home_identity, &worker_identity) {
            return Ok(remote_workspace_not_coordinated_result());
        }
        let workspace_context = WorkspaceLiveSyncWorkspaceContext {
            root: home_root,
            identity: worker_identity,
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let permission_level = self
            .effective_permission_level_for_agent(&context.home_session_id, &context.home_agent_id)
            .await?;
        if let Some(result) = self
            .maybe_gate_workspace_live_sync_mutation(
                &context.home_session_id,
                Some(&context.home_agent_id),
                permission_level,
                tool_name.as_str(),
                &arguments,
            )
            .await?
        {
            return Ok((result, artifact_states));
        }

        let forwarded_result = {
            let mut coordinator = self.owned.workspace_live_sync_coordinator.lock().await;
            match tool_name.as_str() {
                crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                    read::dispatch_forwarded_read(
                        &mut coordinator,
                        &context,
                        arguments,
                        &artifact_states,
                        &workspace_context,
                    )
                }
                crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                    text::dispatch_forwarded_edit(
                        &mut coordinator,
                        &context,
                        tool_name.as_str(),
                        arguments,
                        &artifact_states,
                        &workspace_context,
                    )
                }
                crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                    text::dispatch_forwarded_write(
                        &mut coordinator,
                        &context,
                        tool_name.as_str(),
                        arguments,
                        &artifact_states,
                        &workspace_context,
                    )
                }
                crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                    mutation::dispatch_forwarded_apply_patch(
                        &mut coordinator,
                        &context,
                        tool_name.as_str(),
                        arguments,
                        artifact_states,
                        &workspace_context,
                    )
                }
                crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                    mutation::dispatch_forwarded_delete(
                        &mut coordinator,
                        &context,
                        tool_name.as_str(),
                        arguments,
                        artifact_states,
                        &workspace_context,
                    )
                }
                crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                    mutation::dispatch_forwarded_move(
                        &mut coordinator,
                        &context,
                        tool_name.as_str(),
                        arguments,
                        artifact_states,
                        &workspace_context,
                    )
                }
                _ => Ok(unsupported_remote_workspace_live_sync_tool(&tool_name)),
            }
        }?;

        Ok(forwarded_result)
    }

    pub(crate) async fn finalize_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
        initial_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
        final_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
    ) -> Result<(), DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let home_root = PathBuf::from(session.worktree_id());
        let home_identity = workspace_identity_for_root_off_thread(home_root.clone()).await?;
        let home_identity = workspace_live_sync_identity_for_session_workspace_link(
            home_identity,
            &session,
            &home_root,
        );
        let worker_identity = workspace_live_sync_identity_for_session_workspace_link(
            context.worker_workspace_identity.clone(),
            &session,
            std::path::Path::new(&context.worker_worktree_path),
        );
        if !workspace_live_sync_workspace_identities_match(&home_identity, &worker_identity) {
            return Ok(());
        }
        let workspace_context = WorkspaceLiveSyncWorkspaceContext {
            root: home_root,
            identity: worker_identity,
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let file_changes = workspace_live_sync_managed_mode_remote_tool_file_changes(
            tool_name.as_str(),
            &arguments,
            &initial_artifact_states,
            &final_artifact_states,
        )?;
        self.record_and_fanout_remote_managed_workspace_live_sync_change(
            &context,
            &workspace_context,
            file_changes,
        )
        .await;
        Ok(())
    }

    async fn record_and_fanout_remote_managed_workspace_live_sync_change(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        workspace_context: &WorkspaceLiveSyncWorkspaceContext,
        file_changes: Vec<crate::git_observer::WorkspaceLiveSyncFileChange>,
    ) {
        if file_changes.is_empty() {
            return;
        }
        let changed_paths = file_changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        let change = crate::git_observer::WorkspaceLiveSyncChange {
            session_id: context.home_session_id.clone(),
            agent_id: context.home_agent_id.clone(),
            provider_run_id: context.worker_provider_run_id.clone(),
            prompt_id: context.worker_provider_run_id.clone(),
            repo_root: context.worker_worktree_path.clone(),
            worktree_path: context.worker_worktree_path.clone(),
            branch: workspace_context.identity.branch.clone(),
            changed_paths,
            file_changes,
            status_fingerprint: "remote_managed_workspace_live_sync".to_string(),
        };
        self.record_and_fanout_workspace_live_sync_change(
            change,
            Some(context.worker_kernel_id.as_str()),
            Some(context.worker_machine_id.as_str()),
        )
        .await;
    }
}

fn remote_workspace_not_coordinated_result() -> (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
) {
    let result = crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "applied": false,
            "reason": {
                "kind": "remote_workspace_not_coordinated",
                "message": "The remote agent workspace does not match the home session repo/branch and is not attached to the same workspace link, so Arroba will not coordinate this workspace live sync operation through the home kernel."
            },
            "next_action": "Move the remote agent to the same repo/branch as the home session or attach both worktrees to the same workspace link, then retry through Arroba workspace live sync.",
        }),
    };
    (result, Vec::new())
}

fn unsupported_remote_workspace_live_sync_tool(
    tool_name: &str,
) -> (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
) {
    (
        crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "applied": false,
                "reason": {
                    "kind": "unsupported_remote_workspace_live_sync_tool",
                    "message": format!("remote coordinated workspace live sync does not yet support `{tool_name}`")
                },
                "next_action": "Use arroba.read_artifact, arroba.edit_artifact, arroba.write_artifact, arroba.apply_patch, arroba.move_artifact, or arroba.delete_artifact for remote coordinated workspace live sync.",
            }),
        },
        Vec::new(),
    )
}
