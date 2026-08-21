//! Inbound relay peer request dispatch for leased runtimes and forwarded tools.

use std::sync::Arc;

use base64::Engine;
use chariox_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity};
use tokio::sync::RwLock;

use crate::runtime::router::CommandRouter;
use crate::transport::relay_crypto;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::daemon_requests::RelayRequestOutcome;
use super::peer_events::emit_leased_projection_event;
use super::request_errors::{map_relay_error, relay_error};
use super::sender_identity::{require_bound_kernel_sender, validate_optional_daemon_sender};
use super::{RelayClientState, RelayOutgoingSender};

pub(super) async fn handle_daemon_peer_request(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &RelayOutgoingSender,
    from_daemon_id: &str,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    if let Err(error) =
        validate_optional_daemon_sender(caller_identity.as_ref(), &encrypted_request)
    {
        return RelayRequestOutcome {
            encrypted_response: None,
            error: Some(error),
        };
    }
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
    let managed_context_identity = if managed_context_request(&request) {
        let identity =
            match require_bound_kernel_sender(caller_identity.as_ref(), &encrypted_request) {
                Ok(identity) => identity.clone(),
                Err(error) => {
                    return encrypt_peer_response(
                        &daemon_private_key,
                        &requester_public_key,
                        managed_context_failure_from_relay(&error),
                    )
                }
            };
        if stable_peer_daemon_id(from_daemon_id) != identity.subject {
            return encrypt_peer_response(
                &daemon_private_key,
                &requester_public_key,
                RelayPeerResponse::ManagedContextImportFailed {
                    code: "unauthorized".to_string(),
                    retryable: false,
                },
            );
        }
        Some(identity)
    } else {
        None
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
            account_profile,
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
                    &account_profile,
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
            account_profile,
            model,
            effort,
        } => {
            let updated = router
                .relay_update_leased_agent_profile(
                    &leased_agent_id,
                    provider,
                    account_profile,
                    model,
                    effort,
                )
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
            hidden_system_context,
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
                    &hidden_system_context,
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
        RelayPeerRequest::EnsureRemoteProviderAccount {
            context,
            materialization,
        } => {
            let ensured = router
                .relay_ensure_remote_provider_account(context, materialization)
                .await;
            match ensured {
                Ok(profile) => RelayPeerResponse::RemoteProviderAccountEnsured {
                    provider: profile.provider,
                    account_profile: profile.profile_id,
                },
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
        RelayPeerRequest::ArmManagedContextImport {
            context_id,
            target_environment_id,
            target_kernel_id,
            target_key_thumbprint,
            project_id,
            archive_sha256,
            archive_size_bytes,
        } => {
            let result = router
                .relay_arm_managed_context_import(
                    managed_context_identity
                        .clone()
                        .expect("managed context identity checked before dispatch"),
                    context_id,
                    target_environment_id,
                    target_kernel_id,
                    target_key_thumbprint,
                    project_id,
                    archive_sha256,
                    archive_size_bytes,
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::BeginManagedContextImport {
            transfer_id,
            capability,
        } => {
            let result = router
                .relay_begin_managed_context_import(
                    managed_context_identity
                        .clone()
                        .expect("managed context identity checked before dispatch"),
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::UploadManagedContextChunk {
            transfer_id,
            capability,
            offset,
            data_base64,
            chunk_sha256,
        } => {
            const MAX_ENCODED_CHUNK_BYTES: usize =
                ((crate::managed_context::transfer::MAX_TRANSFER_CHUNK_BYTES + 2) / 3) * 4;
            let data_base64 = data_base64.into_inner();
            let bytes = if data_base64.len() > MAX_ENCODED_CHUNK_BYTES {
                None
            } else {
                match base64::engine::general_purpose::STANDARD.decode(data_base64) {
                    Ok(bytes)
                        if !bytes.is_empty()
                            && bytes.len()
                                <= crate::managed_context::transfer::MAX_TRANSFER_CHUNK_BYTES =>
                    {
                        Some(bytes)
                    }
                    _ => None,
                }
            };
            let Some(bytes) = bytes else {
                return encrypt_peer_response(
                    &daemon_private_key,
                    &requester_public_key,
                    RelayPeerResponse::ManagedContextImportFailed {
                        code: "invalid_request".to_string(),
                        retryable: false,
                    },
                );
            };
            let result = router
                .relay_upload_managed_context_chunk(
                    managed_context_identity
                        .clone()
                        .expect("managed context identity checked before dispatch"),
                    transfer_id,
                    capability.into_inner(),
                    offset,
                    bytes,
                    chunk_sha256,
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::FinalizeManagedContextImport {
            transfer_id,
            capability,
        } => {
            let result = router
                .relay_finalize_managed_context_import(
                    managed_context_identity
                        .clone()
                        .expect("managed context identity checked before dispatch"),
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::GetManagedContextImportStatus {
            transfer_id,
            capability,
        } => {
            let result = router
                .relay_get_managed_context_import_status(
                    managed_context_identity
                        .clone()
                        .expect("managed context identity checked before dispatch"),
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
    };
    encrypt_peer_response(&daemon_private_key, &requester_public_key, response)
}

fn managed_context_failure_response(error: &crate::error::DaemonError) -> RelayPeerResponse {
    let projected = map_relay_error(error);
    RelayPeerResponse::ManagedContextImportFailed {
        code: projected.code,
        retryable: projected.retryable,
    }
}

fn managed_context_failure_from_relay(
    error: &chariox_relay::protocol::RelayError,
) -> RelayPeerResponse {
    RelayPeerResponse::ManagedContextImportFailed {
        code: error.code.clone(),
        retryable: error.retryable,
    }
}

fn encrypt_peer_response(
    daemon_private_key: &str,
    requester_public_key: &str,
    response: RelayPeerResponse,
) -> RelayRequestOutcome {
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

fn managed_context_request(request: &RelayPeerRequest) -> bool {
    matches!(
        request,
        RelayPeerRequest::ArmManagedContextImport { .. }
            | RelayPeerRequest::BeginManagedContextImport { .. }
            | RelayPeerRequest::UploadManagedContextChunk { .. }
            | RelayPeerRequest::FinalizeManagedContextImport { .. }
            | RelayPeerRequest::GetManagedContextImportStatus { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::Engine;
    use chariox_relay::auth::RelaySubjectKind;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::process::Command;
    use tokio::sync::Mutex;

    use crate::config::PersistedCloudRelayProfile;
    use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
    use crate::managed_context::development::{
        export_development_context, DevelopmentContextExportRequest, DevelopmentRepositoryRole,
        DevelopmentRepositorySelection,
    };
    use crate::managed_context::kernel::{
        KernelContextCompatibility, KernelContextPayload, KernelContextSnapshot,
    };
    use crate::managed_context::package::{
        export_managed_context_package, ManagedContextPackageExportRequest,
        ManagedContextPackageKernel,
    };
    use crate::runtime::terminal_pairings::public_key_thumbprint;
    use crate::secret::{
        export_transferred_vault_snapshot, lock_chariox_encrypted_vault,
        unlock_chariox_encrypted_vault, VaultUnlockLease,
    };
    use crate::transport::relay_peer::{
        RelayManagedContextCapability, RelayManagedContextChunk, RelayManagedContextTransferPhase,
    };
    use crate::{DaemonApp, DaemonConfig};

    fn scoped_kernel_identity(
        public_key_thumbprint: Option<String>,
        expires_at_ms: u64,
    ) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: "source-kernel-1".to_string(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms,
            token_id: Some("token-1".to_string()),
            user_id: Some("user-1".to_string()),
            public_key_thumbprint,
        }
    }

    #[tokio::test]
    async fn peer_handler_rejects_invalid_scoped_kernel_identity_before_decryption() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("test daemon should bootstrap"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let malformed_request = EncryptedRelayPayload {
            sender_public_key: "request-sender-public-key".to_string(),
            nonce: "not-valid-base64".to_string(),
            ciphertext: "not-valid-base64".to_string(),
        };
        let identities = [
            scoped_kernel_identity(
                Some(public_key_thumbprint("different-public-key")),
                u64::MAX,
            ),
            scoped_kernel_identity(None, 1),
        ];

        for identity in identities {
            let outcome = handle_daemon_peer_request(
                &router,
                &state,
                &outgoing_tx,
                "source-kernel-1",
                Some(identity),
                malformed_request.clone(),
            )
            .await;
            let error = outcome
                .error
                .expect("invalid scoped identity should be rejected");
            assert_eq!(error.code, "unauthorized");
            assert!(!error.retryable);
            assert!(outcome.encrypted_response.is_none());
        }
    }

    #[tokio::test]
    async fn managed_context_peer_request_requires_a_bound_kernel_identity() {
        let app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("test daemon should bootstrap");
        let target_public_key = app.config().relay_public_key.clone();
        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let request = RelayPeerRequest::ArmManagedContextImport {
            context_id: "context-1".to_string(),
            target_environment_id: "environment-1".to_string(),
            target_kernel_id: "target-kernel-1".to_string(),
            target_key_thumbprint: "a".repeat(64),
            project_id: "project-1".to_string(),
            archive_sha256: "b".repeat(64),
            archive_size_bytes: 42,
        };
        let source_private_key = relay_crypto::generate_private_key_base64();
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &source_private_key,
            &target_public_key,
            &serde_json::to_vec(&request).expect("serialize managed context request"),
        )
        .expect("encrypt managed context request");

        let outcome = handle_daemon_peer_request(
            &router,
            &state,
            &outgoing_tx,
            "source-kernel-1",
            None,
            encrypted_request,
        )
        .await;
        assert!(outcome.error.is_none());
        let encrypted_response = outcome
            .encrypted_response
            .expect("identity rejection should stay encrypted");
        let decrypted =
            relay_crypto::decrypt_payload_for_private_key(&source_private_key, &encrypted_response)
                .expect("decrypt identity rejection");
        let response: RelayPeerResponse =
            serde_json::from_slice(&decrypted.plaintext).expect("decode identity rejection");
        assert!(matches!(
            response,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: false,
            } if code == "unauthorized"
        ));
    }

    #[test]
    fn managed_context_failure_projection_does_not_expose_internal_details() {
        let error = crate::error::DaemonError::ManagedContext {
            code: "invalid_managed_context",
            operation: "import fixture",
            message: "/private/workspace/path and git stderr canary".to_string(),
            retryable: false,
        };
        let response = managed_context_failure_response(&error);
        let serialized = serde_json::to_string(&response).expect("serialize failure response");
        assert!(serialized.contains("invalid_managed_context"));
        assert!(!serialized.contains("/private/workspace/path"));
        assert!(!serialized.contains("git stderr canary"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn encrypted_managed_context_peer_transfer_imports_repository_kernel_context_and_vault() {
        let _env_guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-peer-import-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let repository = root.join("source-repository");
        fs::create_dir_all(&repository).expect("create source repository");
        git(&repository, &["init", "-b", "main"]);
        git(
            &repository,
            &["config", "user.email", "tests@chariox.local"],
        );
        git(&repository, &["config", "user.name", "Chariox Tests"]);
        fs::write(repository.join("tracked.txt"), "managed context\n").expect("write source file");
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "-m", "initial"]);
        let archive_path = root.join("development.tar.gz");
        let exported = export_development_context(DevelopmentContextExportRequest {
            project_id: "project-managed-peer".to_string(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: "workspace-primary".to_string(),
                worktree_path: repository,
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: archive_path.clone(),
        })
        .expect("export managed peer context");

        let mut config = DaemonConfig::for_tests();
        config.session_history_root = root.join("sessions");
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
        config.cloud_relay = Some(test_cloud_profile());
        let target_kernel_id = config.daemon_id.clone();
        let target_machine_id = config.host_machine_id.clone();
        let target_public_key = config.relay_public_key.clone();
        let target_key_thumbprint = public_key_thumbprint(&target_public_key);
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("managed target daemon should bootstrap"),
        ));
        let router = Arc::new(
            CommandRouter::with_interactive_capacity(app, 1).with_managed_kernel_registration(
                ConfirmedManagedKernelRegistration {
                    environment_id: "environment-managed-1".to_string(),
                    machine_id: target_machine_id,
                    kernel_id: target_kernel_id.clone(),
                },
            ),
        );
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let source_private_key = relay_crypto::generate_private_key_base64();
        let source_public_key =
            relay_crypto::public_key_from_private_key_base64(&source_private_key)
                .expect("source public key");
        let source_key_thumbprint = public_key_thumbprint(&source_public_key);
        let identity = scoped_kernel_identity(Some(source_key_thumbprint.clone()), u64::MAX);
        let context_id = "context-managed-peer".to_string();
        let source_vault_path = root.join("source-vault.json");
        let target_vault_path = root.join("target-vault.json");
        let capability_root = root.join("target-capabilities");
        let _capability_env = ScopedEnv::set(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            capability_root.as_os_str(),
        );
        let _vault_env =
            ScopedEnv::set("CHARIOX_MANAGED_VAULT_PATH", target_vault_path.as_os_str());
        unlock_chariox_encrypted_vault(
            &source_vault_path,
            "managed-peer-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("unlock source Vault");
        crate::secret::set_chariox_encrypted_vault_secret_for_test(
            source_vault_path.clone(),
            "managed-peer",
            "token",
            "managed-peer-secret-canary",
        )
        .expect("store source Vault canary");
        let transferred_vault = export_transferred_vault_snapshot(
            &source_vault_path,
            &context_id,
            &identity.subject,
            &source_private_key,
            &target_kernel_id,
            &target_public_key,
        )
        .expect("export transferred Vault");
        let payload = KernelContextPayload {
            schema_version: 1,
            context_id: context_id.clone(),
            source_kernel_id: identity.subject.clone(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            compatibility: KernelContextCompatibility {
                source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions: Vec::new(),
            dependencies: Vec::new(),
            vault: transferred_vault,
        };
        let snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&payload).expect("serialize kernel payload"))
        );
        let kernel_context = KernelContextSnapshot {
            payload,
            snapshot_sha256: snapshot_sha256.clone(),
        };
        let package = export_managed_context_package(ManagedContextPackageExportRequest {
            context_id: context_id.clone(),
            project_id: "project-managed-peer".to_string(),
            target_environment_id: "environment-managed-1".to_string(),
            source_kernel_id: identity.subject.clone(),
            source_key_thumbprint,
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            development_archive_path: archive_path,
            development_archive_sha256: exported.archive_sha256,
            kernel_context: ManagedContextPackageKernel::FromKernel(kernel_context),
            package_path: root.join("context.chariox"),
        })
        .expect("compose managed context package");

        let armed = send_managed_peer_request(
            &router,
            &state,
            &outgoing_tx,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::ArmManagedContextImport {
                context_id,
                target_environment_id: "environment-managed-1".to_string(),
                target_kernel_id,
                target_key_thumbprint,
                project_id: "project-managed-peer".to_string(),
                archive_sha256: package.package_sha256.clone(),
                archive_size_bytes: package.package_size_bytes,
            },
        )
        .await;
        let (transfer_id, capability, max_chunk_bytes) = match armed {
            RelayPeerResponse::ManagedContextImportArmed {
                transfer_id,
                capability,
                max_chunk_bytes,
                relay_peer_protocol_version,
                ..
            } => {
                assert_eq!(relay_peer_protocol_version, RELAY_PEER_PROTOCOL_VERSION);
                (transfer_id, capability.into_inner(), max_chunk_bytes)
            }
            response => panic!("unexpected arm response: {response:?}"),
        };
        send_managed_peer_request(
            &router,
            &state,
            &outgoing_tx,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::BeginManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: RelayManagedContextCapability::new(capability.clone()),
            },
        )
        .await;
        let archive = fs::read(&package.package_path).expect("read exported package");
        let mut offset = 0_u64;
        for chunk in archive.chunks(max_chunk_bytes) {
            send_managed_peer_request(
                &router,
                &state,
                &outgoing_tx,
                &identity,
                &source_private_key,
                &target_public_key,
                RelayPeerRequest::UploadManagedContextChunk {
                    transfer_id: transfer_id.clone(),
                    capability: RelayManagedContextCapability::new(capability.clone()),
                    offset,
                    data_base64: RelayManagedContextChunk::new(
                        base64::engine::general_purpose::STANDARD.encode(chunk),
                    ),
                    chunk_sha256: format!("{:x}", Sha256::digest(chunk)),
                },
            )
            .await;
            offset += chunk.len() as u64;
        }
        let completed = send_managed_peer_request(
            &router,
            &state,
            &outgoing_tx,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: RelayManagedContextCapability::new(capability.clone()),
            },
        )
        .await;
        let receipt = match completed {
            RelayPeerResponse::ManagedContextImportStatus { status } => {
                assert_eq!(status.phase, RelayManagedContextTransferPhase::Consumed);
                assert_eq!(status.accepted_bytes, archive.len() as u64);
                status.receipt.expect("completed import receipt")
            }
            response => panic!("unexpected finalize response: {response:?}"),
        };
        assert_eq!(receipt.transfer_id, transfer_id);
        assert_eq!(receipt.project_id, "project-managed-peer");
        assert_eq!(receipt.repositories.len(), 1);
        assert!(matches!(
            receipt.kernel_context,
            crate::transport::relay_peer::RelayManagedKernelContextImportReceipt::FromKernel {
                snapshot_sha256: ref imported_snapshot_sha256,
                extension_count: 0,
                dependency_count: 0,
                ..
            } if imported_snapshot_sha256 == &snapshot_sha256
        ));
        assert!(capability_root.join("kernel-context-import.json").is_file());
        assert_eq!(
            crate::secret::get_chariox_encrypted_vault_secret_for_test(
                target_vault_path.clone(),
                "managed-peer",
                "token",
            )
            .expect("read imported Vault canary"),
            "managed-peer-secret-canary"
        );
        assert_eq!(
            fs::read_to_string(
                std::path::Path::new(&receipt.repositories[0].destination_path).join("tracked.txt")
            )
            .expect("read imported repository file"),
            "managed context\n"
        );
        let replayed = send_managed_peer_request(
            &router,
            &state,
            &outgoing_tx,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id,
                capability: RelayManagedContextCapability::new(capability),
            },
        )
        .await;
        assert!(matches!(
            replayed,
            RelayPeerResponse::ManagedContextImportStatus {
                status: crate::transport::relay_peer::RelayManagedContextTransferStatus {
                    phase: RelayManagedContextTransferPhase::Consumed,
                    ..
                }
            }
        ));
        drop(router);
        lock_chariox_encrypted_vault(&source_vault_path).expect("lock source Vault");
        lock_chariox_encrypted_vault(&target_vault_path).expect("lock target Vault");
        fs::remove_dir_all(root).expect("remove managed peer fixture");
    }

    struct ScopedEnv {
        name: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl ScopedEnv {
        fn set(name: &'static str, value: &std::ffi::OsStr) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    async fn send_managed_peer_request(
        router: &Arc<CommandRouter>,
        state: &Arc<RwLock<RelayClientState>>,
        outgoing_tx: &RelayOutgoingSender,
        identity: &RelayCallerIdentity,
        source_private_key: &str,
        target_public_key: &str,
        request: RelayPeerRequest,
    ) -> RelayPeerResponse {
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            source_private_key,
            target_public_key,
            &serde_json::to_vec(&request).expect("serialize peer request"),
        )
        .expect("encrypt peer request");
        let outcome = handle_daemon_peer_request(
            router,
            state,
            outgoing_tx,
            &identity.subject,
            Some(identity.clone()),
            encrypted_request,
        )
        .await;
        if let Some(error) = outcome.error {
            panic!("managed peer request failed: {error:?}");
        }
        let encrypted_response = outcome
            .encrypted_response
            .expect("managed peer encrypted response");
        let decrypted =
            relay_crypto::decrypt_payload_for_private_key(source_private_key, &encrypted_response)
                .expect("decrypt peer response");
        serde_json::from_slice(&decrypted.plaintext).expect("decode peer response")
    }

    fn test_cloud_profile() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "account".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some("machine-test".to_string()),
            machine_alias: Some("Managed test".to_string()),
            machine_credential: Some("machine-credential".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        }
    }

    fn git(path: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
