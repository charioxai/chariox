use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_workspace_live_sync_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let resolved_agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .or_else(|| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_agent_id(&session)
            });
        let workspace_context = self
            .workspace_live_sync_workspace_for_provider_run(provider_run)
            .await?;
        if !workspace_context.valid {
            return Ok(workspace_live_sync_workspace_identity_rejected(
                &workspace_context,
            ));
        }
        let permission_level = match resolved_agent_id.as_deref() {
            Some(agent_id) => {
                self.effective_permission_level_for_agent(provider_run.session_id(), agent_id)
                    .await?
            }
            None => provider_run.permission_level(),
        };
        if let Some(result) = self
            .maybe_gate_workspace_live_sync_mutation(
                provider_run.session_id(),
                resolved_agent_id.as_deref(),
                permission_level,
                tool_name,
                &arguments,
            )
            .await?
        {
            return Ok(result);
        }
        let workspace_root = workspace_context.root.clone();
        let workspace_identity = workspace_context.identity.clone();
        let mut coordinator = self.owned.workspace_live_sync_coordinator.lock().await;
        match tool_name {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                let read = crate::io::WorkspaceLiveSyncFileIo::read_artifact(
                    &mut coordinator,
                    crate::io::WorkspaceLiveSyncFileReadRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        path: PathBuf::from(args.path),
                        domain,
                    },
                )
                .map_err(workspace_live_sync_daemon_error)?;
                self.owned
                    .workspace_live_sync_external_changes
                    .observe_managed_read(
                        provider_run.id(),
                        &workspace_identity,
                        &workspace_root,
                        &read.path,
                    );
                let mut payload = workspace_live_sync_read_payload(read);
                add_workspace_live_sync_workspace_payload(&mut payload, &workspace_context);
                Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_edit_artifact",
                        message: "managed edit currently supports only text artifacts".to_string(),
                    });
                }
                let operation = match (args.range, args.old_text) {
                    (Some(range), Some(old_text)) => crate::io::AgentEditOperation::ReplaceRange {
                        range: crate::io::TextRange::new(range.start, range.end),
                        old_text,
                        new_text: args.new_text,
                    },
                    (None, Some(old_text)) => crate::io::AgentEditOperation::ReplaceText {
                        old_text,
                        new_text: args.new_text,
                    },
                    (Some(_), None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "range edits require old_text".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "managed text edits require old_text or range+old_text"
                                .to_string(),
                        });
                    }
                };
                let path = PathBuf::from(args.path.clone());
                workspace_live_sync_reject_ignored_path(
                    &workspace_root,
                    &path,
                    "runtime_tool_edit_artifact",
                )?;
                let managed_fanout_before =
                    workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)?;
                let before = workspace_live_sync_text_for_diff(&workspace_root, &path, false);
                let reservation_ranges = workspace_live_sync_reservation_ranges_for_operation(
                    &operation,
                    before.as_ref(),
                    crate::io::TextRange::new(0, usize::MAX),
                );
                let reservation = match workspace_live_sync_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    reservation_ranges,
                    workspace_live_sync_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_workspace_live_sync_workspace_payload(
                            &mut output.payload,
                            &workspace_context,
                        );
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .workspace_live_sync_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::WorkspaceLiveSyncFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::WorkspaceLiveSyncFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: workspace_live_sync_snapshot_id_from_arg(args.snapshot_id),
                            operation,
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_workspace_live_sync_external_change_if_rejected(
                    &self.owned.workspace_live_sync_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_workspace_live_sync_write_if_applied(
                    &self.owned.workspace_live_sync_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = workspace_live_sync_result_applied(&result)
                    .then(|| workspace_live_sync_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let managed_fanout_change =
                    workspace_live_sync_result_applied(&result).then(|| {
                        workspace_live_sync_managed_file_change(
                            path.clone(),
                            None,
                            managed_fanout_before,
                            workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)
                                .ok()
                                .flatten(),
                        )
                    });
                let mut output = workspace_live_sync_edit_result(
                    result,
                    WorkspaceLiveSyncChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
                if let Some(file_change) = managed_fanout_change {
                    drop(coordinator);
                    self.record_and_fanout_managed_workspace_live_sync_change(
                        provider_run,
                        &session,
                        resolved_agent_id.as_deref(),
                        &workspace_context,
                        vec![file_change],
                    )
                    .await;
                }
                Ok(output)
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: "managed apply_patch currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                let managed_fanout_before =
                    workspace_live_sync_managed_before_snapshots(&workspace_root, &operations)?;
                let mut output = apply_managed_patch_operations(
                    &mut coordinator,
                    workspace_identity,
                    workspace_root.clone(),
                    domain,
                    operations.clone(),
                    workspace_live_sync_reservation_owner(provider_run, tool_name),
                    &self.owned.workspace_live_sync_external_changes,
                )?;
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
                if workspace_live_sync_runtime_tool_applied(&output) {
                    let file_changes = workspace_live_sync_managed_patch_file_changes(
                        &workspace_root,
                        &operations,
                        managed_fanout_before,
                    )?;
                    drop(coordinator);
                    self.record_and_fanout_managed_workspace_live_sync_change(
                        provider_run,
                        &session,
                        resolved_agent_id.as_deref(),
                        &workspace_context,
                        file_changes,
                    )
                    .await;
                }
                Ok(output)
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                let path = PathBuf::from(args.path);
                let managed_fanout_before =
                    workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Delete { path: path.clone() }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                } else {
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Delete { path: path.clone() }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                };
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
                if workspace_live_sync_runtime_tool_applied(&output) {
                    let file_change = workspace_live_sync_managed_file_change(
                        path.clone(),
                        None,
                        managed_fanout_before,
                        workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)?,
                    );
                    drop(coordinator);
                    self.record_and_fanout_managed_workspace_live_sync_change(
                        provider_run,
                        &session,
                        resolved_agent_id.as_deref(),
                        &workspace_context,
                        vec![file_change],
                    )
                    .await;
                }
                Ok(output)
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                let from_path = PathBuf::from(args.from_path.clone());
                let to_path = PathBuf::from(args.to_path.clone());
                let managed_fanout_before =
                    workspace_live_sync_managed_read_optional_bytes(&workspace_root, &from_path)?;
                let (old_text, new_text) = args.normalized_text_transform_fields();
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: from_path.clone(),
                            to_path: to_path.clone(),
                            old_text,
                            new_text,
                        }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: from_path.clone(),
                            to_path: to_path.clone(),
                        }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                };
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
                if workspace_live_sync_runtime_tool_applied(&output) {
                    let file_change = workspace_live_sync_managed_file_change(
                        to_path.clone(),
                        Some(from_path),
                        managed_fanout_before,
                        workspace_live_sync_managed_read_optional_bytes(&workspace_root, &to_path)?,
                    );
                    drop(coordinator);
                    self.record_and_fanout_managed_workspace_live_sync_change(
                        provider_run,
                        &session,
                        resolved_agent_id.as_deref(),
                        &workspace_context,
                        vec![file_change],
                    )
                    .await;
                }
                Ok(output)
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(
                    args.domain.as_deref(),
                )?;
                let path = PathBuf::from(args.path.clone());
                workspace_live_sync_reject_ignored_path(
                    &workspace_root,
                    &path,
                    "runtime_tool_write_artifact",
                )?;
                let managed_fanout_before =
                    workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)?;
                let before = workspace_live_sync_text_for_diff(&workspace_root, &path, true);
                let content = workspace_live_sync_write_content_from_args(
                    "runtime_tool_write_artifact",
                    domain,
                    &args,
                )?;
                let reservation = match workspace_live_sync_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    workspace_live_sync_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_workspace_live_sync_workspace_payload(
                            &mut output.payload,
                            &workspace_context,
                        );
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .workspace_live_sync_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::WorkspaceLiveSyncFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::WorkspaceLiveSyncFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: workspace_live_sync_write_snapshot_id_from_arg(
                                args.snapshot_id,
                                &path,
                            ),
                            operation: crate::io::AgentEditOperation::WriteArtifact { content },
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_workspace_live_sync_external_change_if_rejected(
                    &self.owned.workspace_live_sync_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_workspace_live_sync_write_if_applied(
                    &self.owned.workspace_live_sync_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = workspace_live_sync_result_applied(&result)
                    .then(|| workspace_live_sync_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let managed_fanout_change =
                    workspace_live_sync_result_applied(&result).then(|| {
                        workspace_live_sync_managed_file_change(
                            path.clone(),
                            None,
                            managed_fanout_before,
                            workspace_live_sync_managed_read_optional_bytes(&workspace_root, &path)
                                .ok()
                                .flatten(),
                        )
                    });
                let mut output = workspace_live_sync_edit_result(
                    result,
                    WorkspaceLiveSyncChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
                if let Some(file_change) = managed_fanout_change {
                    drop(coordinator);
                    self.record_and_fanout_managed_workspace_live_sync_change(
                        provider_run,
                        &session,
                        resolved_agent_id.as_deref(),
                        &workspace_context,
                        vec![file_change],
                    )
                    .await;
                }
                Ok(output)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_workspace_live_sync_runtime_tool_call",
                message: format!("unsupported workspace live sync tool `{other}`"),
            }),
        }
    }

    async fn record_and_fanout_managed_workspace_live_sync_change(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        session: &crate::session::RuntimeSession,
        resolved_agent_id: Option<&str>,
        workspace_context: &WorkspaceLiveSyncWorkspaceContext,
        file_changes: Vec<crate::git_observer::WorkspaceLiveSyncFileChange>,
    ) {
        if file_changes.is_empty() {
            return;
        }
        let agent_id = resolved_agent_id
            .map(str::to_string)
            .or_else(|| provider_run.agent_instance_id().map(str::to_string))
            .unwrap_or_else(|| "unknown-agent".to_string());
        let prompt_id = resolved_agent_id
            .and_then(|agent_id| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent_snapshot(session, agent_id)
            })
            .map(|prompt| prompt.id().to_string())
            .unwrap_or_else(|| provider_run.id().to_string());
        let changed_paths = file_changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        let root = workspace_context.root.to_string_lossy().to_string();
        let change = crate::git_observer::WorkspaceLiveSyncChange {
            session_id: session.id().to_string(),
            agent_id,
            provider_run_id: provider_run.id().to_string(),
            prompt_id,
            repo_root: root.clone(),
            worktree_path: root,
            branch: workspace_context.identity.branch.clone(),
            changed_paths,
            file_changes,
            status_fingerprint: "managed_workspace_live_sync".to_string(),
        };
        self.record_and_fanout_workspace_live_sync_change(change, None)
            .await;
    }
}

fn workspace_live_sync_runtime_tool_applied(
    output: &crate::transport::runtime_tools::RuntimeToolResult,
) -> bool {
    output.ok
        && output
            .payload
            .get("applied")
            .and_then(|value| value.as_bool())
            == Some(true)
}

fn workspace_live_sync_managed_before_snapshots(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
) -> Result<BTreeMap<PathBuf, Option<Vec<u8>>>, DaemonError> {
    let mut snapshots = BTreeMap::new();
    for operation in operations {
        for path in workspace_live_sync_managed_patch_operation_paths(operation) {
            if !snapshots.contains_key(&path) {
                snapshots.insert(
                    path.clone(),
                    workspace_live_sync_managed_read_optional_bytes(workspace_root, &path)?,
                );
            }
        }
    }
    Ok(snapshots)
}

fn workspace_live_sync_managed_patch_operation_paths(
    operation: &ManagedPatchOperation,
) -> Vec<PathBuf> {
    match operation {
        ManagedPatchOperation::Add { path, .. }
        | ManagedPatchOperation::Update { path, .. }
        | ManagedPatchOperation::Delete { path } => vec![path.clone()],
        ManagedPatchOperation::Move {
            from_path, to_path, ..
        } => vec![from_path.clone(), to_path.clone()],
    }
}

fn workspace_live_sync_managed_patch_file_changes(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
    before_snapshots: BTreeMap<PathBuf, Option<Vec<u8>>>,
) -> Result<Vec<crate::git_observer::WorkspaceLiveSyncFileChange>, DaemonError> {
    operations
        .iter()
        .map(|operation| match operation {
            ManagedPatchOperation::Add { path, .. }
            | ManagedPatchOperation::Update { path, .. }
            | ManagedPatchOperation::Delete { path } => {
                Ok(workspace_live_sync_managed_file_change(
                    path.clone(),
                    None,
                    before_snapshots.get(path).cloned().flatten(),
                    workspace_live_sync_managed_read_optional_bytes(workspace_root, path)?,
                ))
            }
            ManagedPatchOperation::Move {
                from_path, to_path, ..
            } => Ok(workspace_live_sync_managed_file_change(
                to_path.clone(),
                Some(from_path.clone()),
                before_snapshots.get(from_path).cloned().flatten(),
                workspace_live_sync_managed_read_optional_bytes(workspace_root, to_path)?,
            )),
        })
        .collect()
}

fn workspace_live_sync_managed_file_change(
    path: PathBuf,
    previous_path: Option<PathBuf>,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
) -> crate::git_observer::WorkspaceLiveSyncFileChange {
    let kind = match (&previous_path, before.as_ref(), after.as_ref()) {
        (Some(_), _, _) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Renamed,
        (None, None, Some(_)) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Added,
        (None, Some(_), None) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Deleted,
        _ => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
    };
    let binary = before.as_ref().is_some_and(|bytes| bytes.contains(&0))
        || after.as_ref().is_some_and(|bytes| bytes.contains(&0));
    crate::git_observer::WorkspaceLiveSyncFileChange {
        path: path.to_string_lossy().to_string(),
        previous_path: previous_path.map(|path| path.to_string_lossy().to_string()),
        kind,
        before_content_base64: before
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        after_content_base64: after
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        binary,
    }
}

fn workspace_live_sync_managed_read_optional_bytes(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<Vec<u8>>, DaemonError> {
    workspace_live_sync_validate_patch_path(workspace_root, path)?;
    let full_path = workspace_root.join(path);
    match std::fs::read(&full_path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "managed_workspace_live_sync_fanout",
            message: format!(
                "failed to read `{}` for fanout: {error}",
                path.to_string_lossy()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_file_change_rebases_non_overlapping_target_edit() {
        let source = temp_workspace("managed-fanout-source");
        let target = temp_workspace("managed-fanout-target");
        std::fs::write(source.join("note.txt"), "one\nsource\nthree\n").expect("write source");
        std::fs::write(target.join("note.txt"), "target\nold\nthree\n").expect("write target");

        let change = workspace_live_sync_managed_file_change(
            PathBuf::from("note.txt"),
            None,
            Some(b"one\nold\nthree\n".to_vec()),
            Some(b"one\nsource\nthree\n".to_vec()),
        );
        let turn_change = managed_test_turn_change(vec![change], &source);

        let results =
            crate::git_observer::apply_workspace_live_sync_change_to_target(&turn_change, &target);

        assert_eq!(
            results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased
        );
        assert_eq!(
            std::fs::read_to_string(target.join("note.txt")).expect("read target"),
            "target\nsource\nthree\n"
        );
        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn managed_move_fanout_renames_target_path() {
        let source = temp_workspace("managed-fanout-move-source");
        let target = temp_workspace("managed-fanout-move-target");
        std::fs::write(target.join("from.bin"), [0, 1, 2]).expect("write target");
        let change = workspace_live_sync_managed_file_change(
            PathBuf::from("to.bin"),
            Some(PathBuf::from("from.bin")),
            Some(vec![0, 1, 2]),
            Some(vec![0, 3, 2]),
        );
        let turn_change = managed_test_turn_change(vec![change], &source);

        let results =
            crate::git_observer::apply_workspace_live_sync_change_to_target(&turn_change, &target);

        assert_eq!(
            results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
        );
        assert!(!target.join("from.bin").exists());
        assert_eq!(
            std::fs::read(target.join("to.bin")).expect("read moved target"),
            vec![0, 3, 2]
        );
        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn managed_fanout_reports_conflict_on_overlapping_target_edit() {
        let source = temp_workspace("managed-fanout-conflict-source");
        let target = temp_workspace("managed-fanout-conflict-target");
        std::fs::write(target.join("note.txt"), "one\ntarget\nthree\n").expect("write target");
        let change = workspace_live_sync_managed_file_change(
            PathBuf::from("note.txt"),
            None,
            Some(b"one\nold\nthree\n".to_vec()),
            Some(b"one\nsource\nthree\n".to_vec()),
        );
        let turn_change = managed_test_turn_change(vec![change], &source);

        let results =
            crate::git_observer::apply_workspace_live_sync_change_to_target(&turn_change, &target);

        assert_eq!(
            results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert_eq!(
            std::fs::read_to_string(target.join("note.txt")).expect("read target"),
            "one\ntarget\nthree\n"
        );
        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(target);
    }

    fn managed_test_turn_change(
        file_changes: Vec<crate::git_observer::WorkspaceLiveSyncFileChange>,
        source: &std::path::Path,
    ) -> crate::git_observer::WorkspaceLiveSyncChange {
        crate::git_observer::WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: source.to_string_lossy().to_string(),
            worktree_path: source.to_string_lossy().to_string(),
            branch: Some("main".to_string()),
            changed_paths: file_changes
                .iter()
                .map(|change| change.path.clone())
                .collect(),
            file_changes,
            status_fingerprint: "managed_workspace_live_sync".to_string(),
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        root
    }
}
