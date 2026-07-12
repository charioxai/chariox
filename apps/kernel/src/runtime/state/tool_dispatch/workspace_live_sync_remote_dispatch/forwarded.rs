//! Home-kernel execution for forwarded workspace live sync runtime tool calls.

use super::super::*;
use sha2::{Digest, Sha256};

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

enum RemoteWorkspaceLiveSyncInvocationDisposition {
    Execute,
    Return(RemoteWorkspaceLiveSyncInvocationResult),
    Wait(tokio::sync::watch::Receiver<Option<RemoteWorkspaceLiveSyncInvocationResult>>),
}

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
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
        let permission_level = self
            .effective_permission_level_for_agent(&context.home_session_id, &context.home_agent_id)
            .await?;
        match self
            .begin_remote_workspace_live_sync_invocation(
                &context,
                &metadata,
                &tool_name,
                &arguments,
                &artifact_states,
            )
            .await?
        {
            RemoteWorkspaceLiveSyncInvocationDisposition::Execute => {}
            RemoteWorkspaceLiveSyncInvocationDisposition::Return(cached) => return Ok(cached),
            RemoteWorkspaceLiveSyncInvocationDisposition::Wait(completion_rx) => {
                return self
                    .wait_for_remote_workspace_live_sync_invocation(completion_rx)
                    .await;
            }
        }
        let workspace_context = WorkspaceLiveSyncWorkspaceContext {
            root: home_root,
            identity: worker_identity,
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let permission_result = self
            .maybe_gate_workspace_live_sync_mutation(
                &context.home_session_id,
                Some(&context.home_agent_id),
                permission_level,
                tool_name.as_str(),
                &arguments,
            )
            .await;
        let permission_result = match permission_result {
            Ok(result) => result,
            Err(error) => {
                self.forget_remote_workspace_live_sync_invocation(&context, &metadata, &tool_name)
                    .await;
                return Err(error);
            }
        };
        if let Some(result) = permission_result {
            let forwarded_result = (result, artifact_states);
            self.complete_remote_workspace_live_sync_invocation(
                &context,
                &metadata,
                &tool_name,
                forwarded_result.clone(),
            )
            .await;
            return Ok(forwarded_result);
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
        };

        let forwarded_result = match forwarded_result {
            Ok(forwarded_result) => forwarded_result,
            Err(error) => {
                self.forget_remote_workspace_live_sync_invocation(&context, &metadata, &tool_name)
                    .await;
                return Err(error);
            }
        };
        self.complete_remote_workspace_live_sync_invocation(
            &context,
            &metadata,
            &tool_name,
            forwarded_result.clone(),
        )
        .await;
        Ok(forwarded_result)
    }

    pub(crate) async fn finalize_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
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
        if self
            .remote_workspace_live_sync_invocation_already_finalized(
                &context,
                &metadata,
                &tool_name,
                &arguments,
                &initial_artifact_states,
                &final_artifact_states,
            )
            .await?
        {
            return Ok(());
        }
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
        self.mark_remote_workspace_live_sync_invocation_finalized(&context, &metadata, &tool_name)
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

    async fn begin_remote_workspace_live_sync_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: &str,
        arguments: &serde_json::Value,
        artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    ) -> Result<RemoteWorkspaceLiveSyncInvocationDisposition, DaemonError> {
        let key = remote_workspace_live_sync_invocation_key(context, metadata, tool_name);
        let request_fingerprint =
            remote_workspace_live_sync_request_fingerprint(tool_name, arguments, artifact_states)?;
        let mut invocations = self
            .owned
            .remote_workspace_live_sync_invocations
            .lock()
            .await;
        let Some(existing) = invocations.get(&key) else {
            let (completion_tx, _) = tokio::sync::watch::channel(None);
            invocations.insert(
                key,
                RemoteWorkspaceLiveSyncInvocationState {
                    request_fingerprint,
                    result: None,
                    completion_tx,
                    finalized: false,
                },
            );
            return Ok(RemoteWorkspaceLiveSyncInvocationDisposition::Execute);
        };
        if existing.request_fingerprint != request_fingerprint {
            return Ok(RemoteWorkspaceLiveSyncInvocationDisposition::Return(
                remote_workspace_live_sync_duplicate_rejected_result(
                    metadata,
                    "workspace live sync invocation metadata was reused with different tool arguments or initial artifact state",
                ),
            ));
        }
        if let Some(result) = &existing.result {
            return Ok(RemoteWorkspaceLiveSyncInvocationDisposition::Return(
                result.clone(),
            ));
        }
        Ok(RemoteWorkspaceLiveSyncInvocationDisposition::Wait(
            existing.completion_tx.subscribe(),
        ))
    }

    async fn wait_for_remote_workspace_live_sync_invocation(
        &self,
        mut completion_rx: tokio::sync::watch::Receiver<
            Option<RemoteWorkspaceLiveSyncInvocationResult>,
        >,
    ) -> ForwardedWorkspaceLiveSyncResult {
        loop {
            if let Some(result) = completion_rx.borrow().clone() {
                return Ok(result);
            }
            completion_rx
                .changed()
                .await
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "wait for forwarded workspace live sync invocation",
                    message: "the original workspace live sync invocation ended before publishing a result"
                        .to_string(),
                })?;
        }
    }

    async fn complete_remote_workspace_live_sync_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: &str,
        result: (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
        ),
    ) {
        let key = remote_workspace_live_sync_invocation_key(context, metadata, tool_name);
        let mut invocations = self
            .owned
            .remote_workspace_live_sync_invocations
            .lock()
            .await;
        if let Some(existing) = invocations.get_mut(&key) {
            existing.result = Some(result.clone());
            existing.completion_tx.send_replace(Some(result));
        }
    }

    async fn forget_remote_workspace_live_sync_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: &str,
    ) {
        let key = remote_workspace_live_sync_invocation_key(context, metadata, tool_name);
        let mut invocations = self
            .owned
            .remote_workspace_live_sync_invocations
            .lock()
            .await;
        invocations.remove(&key);
    }

    async fn remote_workspace_live_sync_invocation_already_finalized(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: &str,
        arguments: &serde_json::Value,
        initial_artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
        final_artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    ) -> Result<bool, DaemonError> {
        let key = remote_workspace_live_sync_invocation_key(context, metadata, tool_name);
        let request_fingerprint = remote_workspace_live_sync_request_fingerprint(
            tool_name,
            arguments,
            initial_artifact_states,
        )?;
        let invocations = self
            .owned
            .remote_workspace_live_sync_invocations
            .lock()
            .await;
        let Some(existing) = invocations.get(&key) else {
            return Ok(false);
        };
        if existing.request_fingerprint != request_fingerprint {
            return Err(DaemonError::LocalTransport {
                operation: "finalize_forwarded_workspace_live_sync_runtime_tool_call",
                message:
                    "workspace live sync invocation metadata was reused with different finalize arguments or initial artifact state"
                        .to_string(),
            });
        }
        if let Some((_, expected_final_artifact_states)) = &existing.result {
            if expected_final_artifact_states != final_artifact_states {
                return Err(DaemonError::LocalTransport {
                    operation: "finalize_forwarded_workspace_live_sync_runtime_tool_call",
                    message:
                        "workspace live sync invocation metadata was reused with different final artifact state"
                            .to_string(),
                });
            }
        }
        Ok(existing.finalized)
    }

    async fn mark_remote_workspace_live_sync_invocation_finalized(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: &str,
    ) {
        let key = remote_workspace_live_sync_invocation_key(context, metadata, tool_name);
        let mut invocations = self
            .owned
            .remote_workspace_live_sync_invocations
            .lock()
            .await;
        if let Some(existing) = invocations.get_mut(&key) {
            existing.finalized = true;
        }
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

fn remote_workspace_live_sync_invocation_key(
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
    tool_name: &str,
) -> String {
    let stable_call_id = metadata
        .idempotency_key
        .as_deref()
        .or(metadata.provider_tool_call_id.as_deref())
        .unwrap_or(metadata.invocation_id.as_str());
    format!(
        "{}:{}:{}:{}:{}:{}",
        context.home_session_id,
        context.home_agent_id,
        context.leased_agent_id,
        context.worker_provider_run_id,
        tool_name,
        stable_call_id
    )
}

fn remote_workspace_live_sync_request_fingerprint(
    tool_name: &str,
    arguments: &serde_json::Value,
    artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
) -> Result<String, DaemonError> {
    let value = serde_json::json!({
        "tool_name": tool_name,
        "arguments": arguments,
        "artifact_states": artifact_states,
    });
    let bytes = serde_json::to_vec(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "remote_workspace_live_sync_request_fingerprint",
        message: format!("failed to serialize workspace live sync invocation fingerprint: {error}"),
    })?;
    let mut hash = Sha256::new();
    hash.update(bytes);
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn remote_workspace_live_sync_duplicate_rejected_result(
    metadata: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
    message: &str,
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
                    "kind": "duplicate_remote_workspace_live_sync_invocation",
                    "message": message,
                },
                "invocation": {
                    "id": metadata.invocation_id,
                    "provider_tool_call_id": metadata.provider_tool_call_id,
                    "attempt": metadata.attempt,
                    "idempotency_key": metadata.idempotency_key,
                },
                "next_action": "Refresh the workspace state and retry the edit as a new workspace live sync tool call.",
            }),
        },
        Vec::new(),
    )
}
