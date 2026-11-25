//! Optional connected service operations; no catalog, policy or compute vendor lives here.
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{
    ControlPlaneRequest, ControlPlaneResponse, LocalDaemonResponse, MachineAcquisitionRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::state::KernelRuntimeState;

/// What Cloud's control-plane route receives; the owner's kernel enrollment is the credential.
fn cloud_request_body(
    account_id: &str,
    credential: &str,
    request: impl serde::Serialize,
) -> serde_json::Value {
    serde_json::json!({"accountId": account_id, "kernelCredential": credential, "request": request})
}

pub(crate) async fn kernel_was_released(config: &DaemonConfig, kernel_id: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct ReleaseStatus {
        released: bool,
    }

    let Some(profile) = crate::runtime::cloud_relay_control::active_cloud_profile(config) else {
        return false;
    };
    let Some(credential) = profile.kernel_credential.as_deref() else {
        return false;
    };
    crate::runtime::cloud_api_client::post_cloud_json_with_timeout::<ReleaseStatus>(
        profile.api_url.clone(),
        "/control-plane",
        cloud_request_body(
            &profile.account_id,
            credential,
            serde_json::json!({"action": "released_kernel", "kernel_id": kernel_id}),
        ),
        std::time::Duration::from_secs(2),
    )
    .await
    .is_ok_and(|status| status.released)
}

/// Machine acquisition for the kernel owner or an agent acting for them through a grant: the same
/// acquisition path the agent's `chariox.machines.*` tools take, one control-plane request at a time.
pub(crate) async fn execute_acquire(
    config: &DaemonConfig,
    state: &KernelRuntimeState,
    command: &KernelCommand,
    gateway: &crate::runtime::kernel_gateway::KernelGateway,
    request: &MachineAcquisitionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    // Owner and grant holders acquire as the owner, so a retry from another process finds its request.
    let key = format!("owner:{}", state.kernel_owner_user_id());
    let result = state
        .machine_acquire(&key, request.clone(), |request, _| async move {
            match execute(config, state, command, gateway, &request).await? {
                LocalDaemonResponse::ControlPlane { result } => Ok(result),
                _ => Err(unavailable(
                    "control_plane_binding_invalid",
                    "The connected service returned another response type",
                )),
            }
        })
        .await?;
    Ok(LocalDaemonResponse::MachineAcquisition { result })
}

pub(crate) async fn execute(
    config: &DaemonConfig,
    state: &KernelRuntimeState,
    command: &KernelCommand,
    gateway: &crate::runtime::kernel_gateway::KernelGateway,
    request: &ControlPlaneRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut request = request;
    let replayed: ControlPlaneRequest;
    let capabilities = matches!(request, ControlPlaneRequest::Capabilities {});
    // People use their authenticated owner connection; agents use the same process-bound
    // grant as Rooms, exec and the gateway. A Room bearer alone spends no enrollment.
    let request_shape = crate::local::LocalDaemonRequest::ControlPlane(request.clone());
    let grant = command.external_grant_id();
    if let Some(grant) = &grant {
        state.authorize_external_request(grant, &request_shape)?;
    } else if let Some(reach) = command.reach_caller_id() {
        state.authorize_gateway_request(&reach, &request_shape)?;
    } else if !crate::runtime::cloud_relay_authorization::kernel_cloud_owner(config, command) {
        return Err(unavailable(
            "control_plane_grant_required",
            "Use the kernel owner or the owner's Chariox access grant for machine requests",
        ));
    }
    let profile = crate::runtime::cloud_relay_control::active_cloud_profile(config);
    let credential = profile
        .and_then(|p| p.kernel_credential.as_deref())
        .filter(|c| !c.is_empty());
    let (Some(profile), Some(credential)) = (profile, credential) else {
        return if capabilities {
            Ok(LocalDaemonResponse::ControlPlane {
                result: ControlPlaneResponse::Capabilities {
                    machine_allocation: None,
                },
            })
        } else {
            Err(unavailable(
                "control_plane_unavailable",
                "This kernel has no connected machine service",
            ))
        };
    };
    let mut acquisition_hold = None;
    let mut approval_guard = None;
    if matches!(
        request,
        ControlPlaneRequest::Acquire { .. } | ControlPlaneRequest::AcquireJob { .. }
    ) {
        // A machine acquired without asking that ended elsewhere frees its place before the limits
        // apply. Only machines accepted before the list is sent are settled by it.
        let expected = state.acquired_machines_to_reconcile();
        if !expected.is_empty() {
            if let ControlPlaneResponse::Machines { machines, .. } =
                crate::runtime::cloud_api_client::post_cloud_json::<ControlPlaneResponse>(
                    profile.api_url.clone(),
                    "/control-plane",
                    cloud_request_body(
                        &profile.account_id,
                        credential,
                        &ControlPlaneRequest::List {},
                    ),
                )
                .await?
            {
                state.reconcile_acquired_machines(&expected, &machines);
            }
        }
        let offered = crate::runtime::cloud_api_client::post_cloud_json::<ControlPlaneResponse>(
            profile.api_url.clone(),
            "/control-plane",
            cloud_request_body(
                &profile.account_id,
                credential,
                &ControlPlaneRequest::Capabilities {},
            ),
        )
        .await
        // A Cloud code on an acquire's error is the service's answer to that acquire, and the
        // refusal the agent reads names it. This answer is not about the acquire, so it passes on
        // without its code.
        .map_err(|error| DaemonError::RelayTransport {
            operation: "connected control plane",
            code: "control_plane_offer_unavailable".into(),
            retryable: true,
            message: format!(
                "The connected service did not return its machine offer, so this try was not sent; retry it: {error}"
            ),
        })?;
        let ControlPlaneResponse::Capabilities { machine_allocation } = offered else {
            return Err(unavailable(
                "control_plane_binding_invalid",
                "The connected service did not return its machine offer",
            ));
        };
        let approval = state
            .approve_machine_acquisition(command, request, machine_allocation.as_ref())
            .await?;
        approval_guard = state.machine_approval_guard(command, request);
        // The approved request goes again as the approved one, and the service refuses it unless
        // it holds that request.
        if let crate::runtime::state::Approval::Replay {
            capability_revision,
        } = &approval
        {
            replayed = request.with_capability_revision(capability_revision);
            request = &replayed;
        }
        let still_allowed = (|| {
            if let Some(grant) = &grant {
                state.authorize_external_request(grant, &request_shape)?;
            } else if let Some(reach) = command.reach_caller_id() {
                state.authorize_gateway_request(&reach, &request_shape)?;
            }
            if let (ControlPlaneRequest::AcquireJob { job_ref, .. }, Some(job_id)) =
                (request, command.caller.caller_id.strip_prefix("job:"))
            {
                let (room, agent, owner) = state.job_acquisition_subject(job_id)?;
                if job_ref != job_id
                    || command.session_id.as_deref() != Some(room.as_str())
                    || command.agent_id.as_deref() != Some(agent.as_str())
                    || command.caller.metaagent_id.as_deref() != Some(agent.as_str())
                    || command.caller.user_id.as_deref() != Some(owner.as_str())
                {
                    return Err(unavailable(
                        "control_plane_binding_invalid",
                        "The accepted job no longer matches its Room, agent and owner",
                    ));
                }
            }
            Ok(())
        })();
        if let crate::runtime::state::Approval::WithoutAsking(hold) = approval {
            if let Err(error) = still_allowed {
                state.withdraw_acquisition(hold);
                return Err(error);
            }
            acquisition_hold = Some(hold);
        }
        still_allowed?;
    }
    if let Some(guard) = approval_guard.as_mut() {
        guard.posting();
    }
    let send = crate::runtime::cloud_api_client::post_cloud_json::<ControlPlaneResponse>(
        profile.api_url.clone(),
        "/control-plane",
        cloud_request_body(&profile.account_id, credential, request),
    );
    let sent = match acquisition_hold {
        // The service's answer settles the admitted request even when its caller is cancelled.
        Some(hold) => {
            let state = state.clone();
            tokio::spawn(async move { state.settle_acquisition(hold, send.await) })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "control_plane.acquire",
                    message: error.to_string(),
                })?
        }
        None => send.await,
    };
    // The owner's approved request has reached the service, whatever it answered.
    state.machine_approval_sent(command, request);
    if let Ok(ControlPlaneResponse::Machine {
        allocation_ref,
        state: machine,
        ..
    }) = &sent
    {
        state.observe_acquired_machine(allocation_ref, machine, crate::session::unix_epoch_ms());
    }
    let mut result = sent.map_err(
        |error| match crate::runtime::cloud_api_client::cloud_error_code(&error) {
            Some(
                "authorization_denied" | "session_invalid" | "client_revoked" | "identity_revoked",
            ) => unavailable(
                "control_plane_authorization_denied",
                "Check this kernel's owner enrollment and refresh machine capabilities",
            ),
            _ => error,
        },
    )?;
    let response_matches = matches!(
        (&result, request),
        (
            ControlPlaneResponse::JobCancelled { .. },
            ControlPlaneRequest::CancelJob { .. }
        ) | (
            ControlPlaneResponse::Capabilities { .. },
            ControlPlaneRequest::Capabilities {}
        ) | (
            ControlPlaneResponse::Machines { .. },
            ControlPlaneRequest::List {}
        )
    ) || (matches!(result, ControlPlaneResponse::Machine { .. })
        && !matches!(
            request,
            ControlPlaneRequest::Capabilities {}
                | ControlPlaneRequest::List {}
                | ControlPlaneRequest::CancelJob { .. }
        ));
    if !response_matches {
        return Err(unavailable(
            "control_plane_binding_invalid",
            "The connected service returned another response type",
        ));
    }
    if let ControlPlaneResponse::Machine {
        allocation_ref,
        kernel_target,
        access_state,
        access_reason,
        state: machine_state,
        purpose,
        ..
    } = &mut result
    {
        let expected = match request {
            ControlPlaneRequest::Get { allocation_ref }
            | ControlPlaneRequest::JobStarted { allocation_ref, .. }
            | ControlPlaneRequest::Release { allocation_ref, .. }
            | ControlPlaneRequest::Start { allocation_ref, .. }
            | ControlPlaneRequest::Stop { allocation_ref, .. }
            | ControlPlaneRequest::StartSavedState { allocation_ref, .. }
            | ControlPlaneRequest::DeleteSavedState { allocation_ref, .. } => Some(allocation_ref),
            _ => None,
        };
        if expected.is_some_and(|expected| expected != allocation_ref)
            || kernel_target.as_ref().is_some_and(|t| {
                t.relay_realm_id != profile.realm_id
                    || t.key_thumbprint.len() != 64
                    || !t.key_thumbprint.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(unavailable(
                "control_plane_binding_invalid",
                "The connected service returned another machine or realm",
            ));
        }
        *access_state = "pending".into();
        if !matches!(machine_state.as_str(), "deleting" | "deleted" | "failed") {
            *access_reason = None;
        }
        if let Some(target) = kernel_target.as_ref().filter(|_| machine_state == "ready") {
            let grant = grant
                .or_else(|| command.reach_caller_id())
                .or_else(|| {
                    crate::runtime::cloud_relay_authorization::kernel_cloud_owner(config, command)
                        .then(|| {
                            format!(
                                "{}{}",
                                crate::runtime::state::reach::OWNER_GRANT_PREFIX,
                                state.kernel_owner_user_id()
                            )
                        })
                })
                .unwrap_or_else(|| {
                    let user_id = command
                        .caller
                        .user_id
                        .as_deref()
                        .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
                    format!(
                        "{}{user_id}",
                        crate::runtime::state::reach::OWNER_GRANT_PREFIX
                    )
                });
            let origin = state.gateway_origin(&grant).ok_or_else(|| {
                unavailable(
                    "kernel_access_ended",
                    "The machine caller's kernel access ended",
                )
            })?;
            let attached = if purpose.as_deref() == Some("offload_job") {
                gateway
                    .attach_job_machine(
                        target,
                        &origin,
                        &format!("{}:machine-access", command.command_id),
                    )
                    .await
            } else {
                gateway
                    .attach_machine(
                        target,
                        &origin,
                        &format!("{}:machine-access", command.command_id),
                    )
                    .await
            };
            match attached {
                Ok(()) => *access_state = "available".into(),
                Err(error) => *access_reason = Some(error.code().to_string()),
            }
        }
        // The service tracks compute lifecycle. A ready response also requires verified access
        // to the kernel, which can disappear independently of the compute instance.
        if machine_state == "ready" && access_state != "available" {
            *machine_state = "reconnecting".into();
            access_reason.get_or_insert_with(|| "kernel_target_unverified".into());
        }
    }
    Ok(LocalDaemonResponse::ControlPlane { result })
}

/// An acquisition by an agent names its room and agent, so the service lists the machine under them.
/// The name comes from the kernel's own record of the caller, never from the request.
fn unavailable(code: &str, message: &str) -> DaemonError {
    DaemonError::RelayTransport {
        operation: "connected control plane",
        code: code.into(),
        retryable: false,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn mp08_mp11_self_hosted_discovery_is_empty_and_membership_cannot_spend() {
        let config = DaemonConfig::for_tests();
        let app = crate::test_support::bootstrap_authenticated_app(config.clone()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        let request =
            crate::local::LocalDaemonRequest::ControlPlane(ControlPlaneRequest::Capabilities {});
        let mut command = KernelCommand::from_local_request("discover", None, None, &request);
        let result = execute(
            &config,
            &state,
            &command,
            router.kernel_gateway(),
            &ControlPlaneRequest::Capabilities {},
        )
        .await
        .unwrap();
        assert!(matches!(
            result,
            LocalDaemonResponse::ControlPlane {
                result: ControlPlaneResponse::Capabilities {
                    machine_allocation: None
                }
            }
        ));
        let acquire = ControlPlaneRequest::Acquire {
            capability_revision: "forged".into(),
            request_id: "one".into(),
            class_id: None,
            region: None,
            timeout_s: None,
        };
        let error = execute(&config, &state, &command, router.kernel_gateway(), &acquire)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("control_plane_unavailable"),
            "the owner reaches the connected-service boundary without a separate agent grant: {error}"
        );
        command.source = crate::runtime::command::KernelCommandSource::RelayClient;
        command.caller.caller_kind = crate::runtime::command::KernelCallerKind::RemoteClient;
        command.caller.user_id = Some("another-user".into());
        let error = execute(&config, &state, &command, router.kernel_gateway(), &acquire)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("control_plane_grant_required"));
    }

    // Cloud's route test sends this file, so what Cloud accepts is what the kernel builds.
    #[test]
    fn the_acquire_request_matches_the_shared_contract() {
        let request = ControlPlaneRequest::Acquire {
            capability_revision: "a".repeat(128),
            request_id: "kernel-acquire-1".into(),
            class_id: None,
            region: None,
            timeout_s: Some(300),
        };
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/cloud-contract/agent-acquire-request.json"
        ))
        .unwrap();
        assert_eq!(
            cloud_request_body("account-1", "kcred_synthetic", &request),
            fixture
        );
    }
}
