//! Inbound relay peer request dispatch for leased runtimes and forwarded tools.

use std::sync::Arc;

use arroba_relay::protocol::EncryptedRelayPayload;
use tokio::sync::RwLock;

use crate::runtime::router::CommandRouter;
use crate::transport::relay_crypto;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::daemon_requests::RelayRequestOutcome;
use super::peer_events::emit_leased_projection_event;
use super::request_errors::{map_relay_error, relay_error};
use super::{RelayClientState, RelayOutgoingSender};

pub(super) async fn handle_daemon_peer_request(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &RelayOutgoingSender,
    from_daemon_id: &str,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, requester_public_key, daemon_private_key, daemon_id) = {
        let daemon_private_key = router.relay_private_key();
        let daemon_id = router.relay_daemon_id();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (
            request,
            decrypted.sender_public_key,
            daemon_private_key,
            daemon_id,
        )
    };
    if !from_daemon_id.trim().is_empty() {
        state.write().await.remember_peer_public_key(
            stable_peer_daemon_id(from_daemon_id),
            requester_public_key.clone(),
        );
    }

    let response = match request {
        RelayPeerRequest::Ping { value } => RelayPeerResponse::Pong { value, daemon_id },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id,
            home_session_id,
            home_agent_id,
            home_agent_metaagent,
            owner_user_id,
        } => {
            let lease = router
                .relay_create_execution_lease(
                    &home_kernel_id,
                    &home_session_id,
                    &home_agent_id,
                    home_agent_metaagent,
                    &owner_user_id,
                )
                .await;
            match lease {
                Ok(lease) => RelayPeerResponse::ExecutionLeaseCreated {
                    lease,
                    relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyExecutionLease { lease_id } => {
            let destroyed = router.relay_destroy_execution_lease(&lease_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::ExecutionLeaseDestroyed { lease_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id,
            provider,
            model,
            effort,
            execution_mode,
            permission_level,
            workspace_live_sync_mode,
            worktree_id,
            worktree_placement,
        } => {
            let leased_agent = router
                .relay_create_leased_agent(
                    &lease_id,
                    &provider,
                    model,
                    effort,
                    execution_mode,
                    permission_level,
                    workspace_live_sync_mode,
                    worktree_id,
                    worktree_placement,
                )
                .await;
            match leased_agent {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentSpawned { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyLeasedAgent { leased_agent_id } => {
            let destroyed = router.relay_destroy_leased_agent(&leased_agent_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id,
            execution_mode,
            permission_level,
        } => {
            let updated = router
                .relay_update_leased_agent_config(
                    &leased_agent_id,
                    execution_mode,
                    permission_level,
                )
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentConfigUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id,
            provider,
            model,
            effort,
        } => {
            let updated = router
                .relay_update_leased_agent_profile(&leased_agent_id, provider, model, effort)
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentProfileUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id,
            active,
        } => {
            let updated = router
                .relay_update_leased_agent_meta_mode(&leased_agent_id, active)
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentMetaModeUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id,
            remote_extension_manifest,
        } => {
            let updated = router
                .relay_update_leased_agent_remote_extension_manifest(
                    &leased_agent_id,
                    remote_extension_manifest,
                )
                .await;
            match updated {
                Ok(()) => {
                    RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated { leased_agent_id }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::LaunchLeasedNativeProviderRun {
            leased_agent_id,
            adapter_key,
            provider,
            account_profile,
            model,
            variant,
            structured_endpoint,
            provider_session_id,
            required_mcps,
            required_skills,
            remote_extension_manifest,
        } => {
            let launched = router
                .relay_launch_leased_native_provider_run(
                    &leased_agent_id,
                    &adapter_key,
                    &provider,
                    &account_profile,
                    &model,
                    variant,
                    structured_endpoint,
                    provider_session_id,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                )
                .await;
            match launched {
                Ok(provider_run) => {
                    RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SendLeasedNativeProviderInput {
            leased_agent_id,
            provider_run_id,
            attachment_id,
            data_base64,
        } => {
            let sent = router
                .relay_send_leased_native_provider_input(
                    &leased_agent_id,
                    &provider_run_id,
                    &attachment_id,
                    &data_base64,
                )
                .await;
            match sent {
                Ok(byte_count) => RelayPeerResponse::LeasedNativeProviderInputSent { byte_count },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ResizeLeasedProviderTerminal {
            leased_agent_id,
            provider_run_id,
            cols,
            rows,
        } => {
            let resized = router
                .relay_resize_leased_provider_terminal(
                    &leased_agent_id,
                    &provider_run_id,
                    cols,
                    rows,
                )
                .await;
            match resized {
                Ok(()) => RelayPeerResponse::LeasedProviderTerminalResized {
                    provider_run_id,
                    cols,
                    rows,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
            required_skills,
            remote_extension_manifest,
        } => {
            let submitted = router
                .relay_submit_leased_prompt(
                    &leased_agent_id,
                    &prompt,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                )
                .await;
            match submitted {
                Ok((provider_run_id, outcome)) => {
                    if let Err(error) = emit_leased_projection_event(
                        router,
                        state,
                        outgoing_tx,
                        &leased_agent_id,
                        &provider_run_id,
                        true,
                    )
                    .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.relay",
                            "failed to emit leased runtime projection after submit",
                            serde_json::json!({
                                "leased_agent_id": leased_agent_id,
                                "provider_run_id": provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id,
                        outcome,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SteerLeasedPrompt {
            leased_agent_id,
            steer_id,
            target_home_prompt_id,
            prompt,
            hidden_system_context,
            attachments,
            required_skills,
        } => {
            let steered = router
                .relay_steer_leased_prompt(
                    &leased_agent_id,
                    &steer_id,
                    &target_home_prompt_id,
                    &prompt,
                    &hidden_system_context,
                    attachments,
                    required_skills,
                )
                .await;
            match steered {
                Ok((provider_run_id, replayed)) => {
                    if let Err(error) = emit_leased_projection_event(
                        router,
                        state,
                        outgoing_tx,
                        &leased_agent_id,
                        &provider_run_id,
                        true,
                    )
                    .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.relay",
                            "failed to emit leased runtime projection after steer",
                            serde_json::json!({
                                "leased_agent_id": leased_agent_id,
                                "provider_run_id": provider_run_id,
                                "steer_id": steer_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    RelayPeerResponse::LeasedPromptSteered {
                        provider_run_id,
                        steer_id,
                        replayed,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id,
            provider_run_id,
            pump_output,
        } => {
            let drained = router
                .relay_drain_leased_runtime_projection(
                    &leased_agent_id,
                    &provider_run_id,
                    pump_output,
                    true,
                )
                .await;
            match drained {
                Ok(event) => RelayPeerResponse::LeasedRuntimeProjectionDrained {
                    event: event.map(|(_target_daemon_id, event)| event),
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CompleteLeasedPrompt { leased_agent_id } => {
            let completion = router.relay_complete_leased_prompt(&leased_agent_id).await;
            match completion {
                Ok(completion) => {
                    let provider_run_id = router
                        .relay_leased_agent_provider_run_id(&leased_agent_id)
                        .await
                        .ok()
                        .flatten();
                    let provider_diagnostic =
                        if let Some(provider_run_id) = provider_run_id.as_deref() {
                            router
                                .relay_provider_run_terminal_diagnostic(provider_run_id)
                                .await
                                .ok()
                                .flatten()
                        } else {
                            None
                        };
                    let (git_observations, workspace_live_sync_change) =
                        if let Some(provider_run_id) = provider_run_id.as_deref() {
                            router
                                .relay_observe_leased_git_after(&leased_agent_id, provider_run_id)
                                .await
                                .unwrap_or_default()
                        } else {
                            (Vec::new(), None)
                        };
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id,
                        provider_diagnostic,
                        git_observations,
                        workspace_live_sync_change,
                        completion,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ObserveLeasedGitAfter {
            leased_agent_id,
            provider_run_id,
        } => match router
            .relay_observe_leased_git_after(&leased_agent_id, &provider_run_id)
            .await
        {
            Ok((git_observations, workspace_live_sync_change)) => {
                RelayPeerResponse::LeasedGitObserved {
                    provider_run_id,
                    git_observations,
                    workspace_live_sync_change,
                }
            }
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                };
            }
        },
        RelayPeerRequest::CancelLeasedPrompt { leased_agent_id } => {
            let cancellation = router.relay_cancel_leased_prompt(&leased_agent_id).await;
            match cancellation {
                Ok(cancellation) => RelayPeerResponse::LeasedPromptCancelled { cancellation },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::WorkflowRuntimeToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowProviderFailure { context, message } => {
            let handled = router
                .dispatch_forwarded_workflow_provider_failure(context, message)
                .await;
            match handled {
                Ok(()) => RelayPeerResponse::WorkflowProviderFailureHandled,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
            context,
            metadata,
            tool_name,
            arguments,
            artifact_states,
        } => {
            let handled = router
                .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    tool_name,
                    arguments,
                    artifact_states,
                )
                .await;
            match handled {
                Ok((result, final_artifact_states)) => {
                    RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
                        result,
                        final_artifact_states,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
            context,
            metadata,
            tool_name,
            arguments,
            initial_artifact_states,
            final_artifact_states,
        } => {
            let finalized = router
                .finalize_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    tool_name,
                    arguments,
                    initial_artifact_states,
                    final_artifact_states,
                )
                .await;
            match finalized {
                Ok(()) => RelayPeerResponse::WorkspaceLiveSyncRuntimeToolFinalized,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardCapabilityRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok((result, skill_package, remote_extension_manifest)) => {
                    RelayPeerResponse::CapabilityRuntimeToolHandled {
                        result,
                        skill_package,
                        remote_extension_manifest,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardMetaRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_meta_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::MetaRuntimeToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeExtensionTool {
            context,
            metadata,
            tool,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_home_extension_tool_call(context, metadata, tool, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::HomeExtensionToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeMcpProxy {
            context,
            metadata,
            name,
            tool,
            payload,
        } => {
            let handled = router
                .dispatch_forwarded_home_mcp_proxy_call(context, metadata, name, tool, payload)
                .await;
            match handled {
                Ok(response) => RelayPeerResponse::HomeMcpProxyHandled { response },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CancelHomeExtensionInvocation { context, metadata } => {
            let invocation_id = metadata.invocation_id.clone();
            let cancelled = router
                .cancel_forwarded_home_extension_invocation(context, metadata)
                .await;
            match cancelled {
                Ok(cancelled) => RelayPeerResponse::HomeExtensionInvocationCancelled {
                    invocation_id,
                    cancelled,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeCredentialTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_home_credential_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::HomeCredentialToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ResolveHomeCredentialSecret {
            context,
            credential_id,
            injection,
        } => {
            let resolved = router
                .resolve_forwarded_home_credential_secret(context, credential_id, injection)
                .await;
            match resolved {
                Ok((credential_id, secret_input)) => {
                    RelayPeerResponse::HomeCredentialSecretResolved {
                        credential_id,
                        secret_input,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ApplyWorkspaceLiveSyncChange { context, change } => {
            let applied = router
                .relay_apply_workspace_live_sync_change(context, change)
                .await;
            match applied {
                Ok(target_result) => {
                    RelayPeerResponse::WorkspaceLiveSyncChangeApplied { target_result }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardNativeInteraction {
            context,
            interaction,
        } => {
            let handled = router
                .relay_forward_native_interaction(context, interaction)
                .await;
            match handled {
                Ok(resolution) => RelayPeerResponse::NativeInteractionResolved { resolution },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::EnsureRemoteSkillPackages { context, packages } => {
            let ensured = router
                .relay_ensure_remote_skill_packages(context, packages)
                .await;
            match ensured {
                Ok(materialized) => RelayPeerResponse::RemoteSkillPackagesEnsured { materialized },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CheckRemoteMcpAvailability {
            context,
            required_mcps,
        } => {
            let checked = router
                .relay_check_remote_mcp_availability(context, required_mcps)
                .await;
            match checked {
                Ok(results) => RelayPeerResponse::RemoteMcpAvailabilityChecked { results },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
    };
    let plaintext = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(error) => {
            return RelayRequestOutcome {
                encrypted_response: None,
                error: Some(relay_error(
                    "relay_request_failed",
                    &format!("failed to serialize relay peer response: {error}"),
                    false,
                )),
            };
        }
    };
    match relay_crypto::encrypt_payload_for_peer(
        &daemon_private_key,
        &requester_public_key,
        &plaintext,
    ) {
        Ok(encrypted_response) => RelayRequestOutcome {
            encrypted_response: Some(encrypted_response),
            error: None,
        },
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(relay_error(
                "relay_request_failed",
                &format!("failed to encrypt relay peer response: {error}"),
                false,
            )),
        },
    }
}

fn stable_peer_daemon_id(from_daemon_id: &str) -> &str {
    from_daemon_id
        .split_once(":peer-tmp:daemon-peer-tmp-")
        .map_or(from_daemon_id, |(daemon_id, _)| daemon_id)
}
