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
            return Ok(workspace_live_sync_workspace_identity_rejected(&workspace_context));
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
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
                self.owned.workspace_live_sync_external_changes.observe_managed_read(
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
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
                        add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: "managed apply_patch currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                let mut output = apply_managed_patch_operations(
                    &mut coordinator,
                    workspace_identity,
                    workspace_root.clone(),
                    domain,
                    operations,
                    workspace_live_sync_reservation_owner(provider_run, tool_name),
                    &self.owned.workspace_live_sync_external_changes,
                )?;
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                } else {
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                };
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
                let (old_text, new_text) = args.normalized_text_transform_fields();
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
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
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        workspace_live_sync_reservation_owner(provider_run, tool_name),
                        &self.owned.workspace_live_sync_external_changes,
                    )?
                };
                add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
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
                let domain =
                    KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
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
                        add_workspace_live_sync_workspace_payload(&mut output.payload, &workspace_context);
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
                Ok(output)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_workspace_live_sync_runtime_tool_call",
                message: format!("unsupported workspace live sync tool `{other}`"),
            }),
        }
    }
}
