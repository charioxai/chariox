//! Home-kernel execution for forwarded managed-I/O runtime tool calls.

use super::super::*;

mod mutation;
mod read;
mod text;

pub(super) type ForwardedManagedIoResult = Result<
    (
        crate::transport::runtime_tools::RuntimeToolResult,
        Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ),
    DaemonError,
>;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> ForwardedManagedIoResult {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let home_root = PathBuf::from(session.worktree_id());
        let home_identity = workspace_identity_for_root_off_thread(home_root.clone()).await?;
        if !managed_io_workspace_identities_match(
            &home_identity,
            &context.worker_workspace_identity,
        ) {
            return Ok(remote_workspace_not_coordinated_result());
        }
        let workspace_context = ManagedIoWorkspaceContext {
            root: home_root,
            identity: context.worker_workspace_identity.clone(),
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let permission_level = self
            .effective_permission_level_for_agent(&context.home_session_id, &context.home_agent_id)
            .await?;
        if let Some(result) = self
            .maybe_gate_managed_io_mutation(
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
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name.as_str() {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => read::dispatch_forwarded_read(
                &mut coordinator,
                &context,
                arguments,
                &artifact_states,
                &workspace_context,
            ),
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => text::dispatch_forwarded_edit(
                &mut coordinator,
                &context,
                tool_name.as_str(),
                arguments,
                &artifact_states,
                &workspace_context,
            ),
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => text::dispatch_forwarded_write(
                &mut coordinator,
                &context,
                tool_name.as_str(),
                arguments,
                &artifact_states,
                &workspace_context,
            ),
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
            _ => Ok(unsupported_remote_managed_io_tool(&tool_name)),
        }
    }
}

fn remote_workspace_not_coordinated_result() -> (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
) {
    let result = crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "applied": false,
            "reason": {
                "kind": "remote_workspace_not_coordinated",
                "message": "The remote agent workspace does not match the home session repo/branch, so Arroba will not coordinate this managed I/O operation through the home kernel."
            },
            "next_action": "Move the remote agent to the same repo and branch as the home session, then retry through Arroba managed I/O.",
        }),
    };
    (result, Vec::new())
}

fn unsupported_remote_managed_io_tool(
    tool_name: &str,
) -> (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
) {
    (
        crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "applied": false,
                "reason": {
                    "kind": "unsupported_remote_managed_io_tool",
                    "message": format!("remote coordinated managed I/O does not yet support `{tool_name}`")
                },
                "next_action": "Use arroba.read_artifact, arroba.edit_artifact, or arroba.write_artifact for remote coordinated text edits until patch/move/delete remote routing lands.",
            }),
        },
        Vec::new(),
    )
}
