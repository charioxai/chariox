use super::*;

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app).leased_managed_io_context_for_provider_run(
                    provider_run.id(),
                    workspace_context.identity.clone(),
                )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(managed_io_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let artifact_states = remote_managed_io_artifact_states_for_tool(
            &workspace_context.root,
            tool_name,
            &arguments,
        )?;
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardManagedIoRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                            artifact_states: artifact_states.clone(),
                        },
                    ),
                )
            })
            .await?;
        let (mut result, final_states) = match response {
            RelayPeerResponse::ManagedIoRuntimeToolHandled {
                result,
                final_artifact_states,
            } => (result, final_artifact_states),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased managed I/O runtime tool",
                    message: format!("unexpected forwarded managed I/O response: {other:?}"),
                });
            }
        };
        if result.ok && !final_states.is_empty() {
            if let Some(rejection) = apply_remote_managed_io_final_states(
                &workspace_context.root,
                &artifact_states,
                &final_states,
            )? {
                result = rejection;
            }
        }
        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
        Ok(Some(result))
    }

    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
        ),
        DaemonError,
    > {
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
            return Ok((result, Vec::new()));
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
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &PathBuf::from(&args.path))
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_read_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        })?;
                let content = remote_managed_io_content_from_state(state, domain)?;
                let read = coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity,
                    path: PathBuf::from(args.path),
                    domain,
                    content,
                });
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok((
                    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
                    Vec::new(),
                ))
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_edit_artifact",
                        message: "remote managed edit currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operation = managed_io_edit_operation_from_args(args.clone())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_edit_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    managed_io_reservation_ranges_for_operation(
                        &operation,
                        before.as_ref(),
                        crate::io::TextRange::new(0, usize::MAX),
                    ),
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_snapshot_id_from_arg(args.snapshot_id.clone()),
                        operation,
                    },
                });
                coordinator.release_reservation(reservation);
                let after = managed_io_result_applied(&result)
                    .then(|| {
                        let artifact_id =
                            coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                        coordinator
                            .current_content(&artifact_id)
                            .and_then(|content| content.as_text().map(str::to_string))
                            .map(|text| ManagedIoTextSnapshot {
                                existed: true,
                                text,
                            })
                    })
                    .flatten();
                let final_states = after
                    .as_ref()
                    .map(|after| vec![remote_managed_io_state(&path, Some(after.text.clone()))])
                    .unwrap_or_default();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_write_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_write_snapshot_id_from_arg(
                            args.snapshot_id.clone(),
                            &path,
                        ),
                        operation: crate::io::AgentEditOperation::WriteArtifact {
                            content: managed_io_write_content_from_args(
                                "forwarded_managed_io_write_artifact",
                                domain,
                                &args,
                            )?,
                        },
                    },
                });
                coordinator.release_reservation(reservation);
                let (after, final_states) = if managed_io_result_applied(&result) {
                    let artifact_id =
                        coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                    let content = coordinator.current_content(&artifact_id).cloned();
                    let after = content.as_ref().and_then(|content| match content {
                        crate::io::ArtifactContent::Text(text) => Some(ManagedIoTextSnapshot {
                            existed: true,
                            text: text.clone(),
                        }),
                        crate::io::ArtifactContent::Bytes(_) => None,
                    });
                    let final_states = content
                        .map(|content| {
                            vec![remote_managed_io_state_from_content_with_domain(
                                &path,
                                Some(content),
                                domain,
                            )]
                        })
                        .unwrap_or_default();
                    (after, final_states)
                } else {
                    (None, Vec::new())
                };
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_apply_patch",
                        message:
                            "remote managed apply_patch currently supports only text artifacts"
                                .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                apply_remote_managed_patch_operations(
                    &mut coordinator,
                    context.worker_workspace_identity,
                    domain,
                    operations,
                    artifact_states,
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id),
                        tool_name,
                    ),
                    &workspace_context,
                )
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let (old_text, new_text) = args.normalized_text_transform_fields();
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                            old_text,
                            new_text,
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            _ => Ok((
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
            )),
        }
    }
}
