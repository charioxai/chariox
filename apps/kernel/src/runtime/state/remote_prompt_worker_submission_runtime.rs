//! Relay worker submission and stale remote-agent binding refresh for remote prompts.

use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

const REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(30);
const REMOTE_PROMPT_RECEIPT_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug)]
enum RemotePromptSubmissionOutcome {
    Accepted(String),
    DefinitelyPreSend(DaemonError),
    RejectedBeforeAdmission(DaemonError),
    Indeterminate(DaemonError),
}

struct RemotePromptSubmissionAttempt {
    result: Result<String, DaemonError>,
    request_transport_started: bool,
}

pub(super) async fn submit_remote_prompt_to_worker_with_binding_refresh(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
) -> Result<String, DaemonError> {
    if remote_prompt_reconciliation_pending(state, dispatch)? {
        return Err(remote_prompt_reconciliation_pending_error(
            dispatch,
            "a previous submission has no reconciled worker receipt",
        ));
    }
    ensure_remote_prompt_dispatching(state, dispatch)?;
    let mut attempt = 0_u32;
    let mut provider_credential_retry_used = false;
    let mut binding_refresh_used = false;
    let transport_retry_started_at = tokio::time::Instant::now();
    let mut provider_launch_credential =
        remote_prompt_provider_launch_credential_if_needed(state, dispatch).await?;
    loop {
        if let Some(error) = remote_prompt_dispatch_unavailable_slice_error(state, dispatch) {
            return Err(error);
        }
        if attempt > 0 && !remote_prompt_dispatch_is_current(state, dispatch) {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let outcome = state
            .submit_remote_prompt_attempt_with_outcome(
                dispatch,
                prompt.clone(),
                attachments.clone(),
                provider_launch_credential.clone(),
                "unexpected remote prompt response",
            )
            .await;
        match outcome {
            RemotePromptSubmissionOutcome::Accepted(provider_run_id) => {
                // Dispatch settlement clears the hold only after its durable ACK write.
                return Ok(provider_run_id);
            }
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
                if provider_launch_credential.is_none()
                    && !provider_credential_retry_used
                    && remote_prompt_error_requires_provider_launch_credential(&error) =>
            {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                provider_credential_retry_used = true;
                provider_launch_credential = state
                    .resolve_remote_provider_launch_credential(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        "relaunch remote provider run",
                    )
                    .await?;
            }
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
                if !binding_refresh_used && remote_prompt_error_should_refresh_binding(&error) =>
            {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                binding_refresh_used = true;
                refresh_remote_prompt_binding(state, dispatch).await?;
                provider_launch_credential =
                    remote_prompt_provider_launch_credential_if_needed(state, dispatch).await?;
            }
            RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
                if !binding_refresh_used && remote_prompt_error_should_refresh_binding(&error) =>
            {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                binding_refresh_used = true;
                refresh_remote_prompt_binding(state, dispatch).await?;
                provider_launch_credential =
                    remote_prompt_provider_launch_credential_if_needed(state, dispatch).await?;
            }
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
                if remote_prompt_error_should_retry_transport(&error) =>
            {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                attempt = attempt.saturating_add(1);
                if remote_prompt_transport_retry_window_expired(
                    transport_retry_started_at.elapsed(),
                ) {
                    return Err(error);
                }
                if attempt == 1 || attempt % 12 == 0 {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "relay rejected remote prompt before worker admission; retrying active prompt",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": attempt,
                            "error": error.to_string(),
                        }),
                    );
                }
                tokio::time::sleep(remote_prompt_transport_retry_delay(attempt)).await;
            }
            RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
                if remote_prompt_error_should_retry_transport(&error) =>
            {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                attempt = attempt.saturating_add(1);
                if remote_prompt_transport_retry_window_expired(
                    transport_retry_started_at.elapsed(),
                ) {
                    return Err(error);
                }
                if attempt == 1 || attempt % 12 == 0 {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt request was definitely not sent; retrying active prompt",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": attempt,
                            "error": error.to_string(),
                        }),
                    );
                }
                tokio::time::sleep(remote_prompt_transport_retry_delay(attempt)).await;
            }
            RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
            | RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error) => {
                clear_reconciliation_pending_or_defer(state, dispatch)?;
                return Err(error);
            }
            RemotePromptSubmissionOutcome::Indeterminate(error) => {
                let detail = if persist_remote_prompt_reconciliation_pending(state, dispatch, true)
                    .is_ok()
                {
                    "the request may have reached the worker, but no submission acknowledgement was recorded"
                } else {
                    "the request may have reached the worker, but no submission acknowledgement was recorded and the reconciliation marker could not be persisted; the already-durable Dispatching phase still blocks restart replay"
                };
                state.report_remote_prompt_reconciliation_pending(dispatch, detail);
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt delivery is pending worker receipt reconciliation",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "prompt_id": dispatch.prompt_id,
                        "delivery_uncertain": true,
                        "prompt_replayed": false,
                        "transport_error": error.to_string(),
                    }),
                );
                return state.reconcile_remote_prompt_worker_receipt(dispatch).await;
            }
        }
    }
}

pub(super) async fn query_remote_prompt_worker_receipt(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Result<Option<crate::transport::relay_peer::LeasedPromptReceipt>, DaemonError> {
    query_remote_prompt_worker_receipt_with_transport(state, dispatch, |config, target, request| {
        async move {
            let relay_state = state.connected_relay_state_for_config(&config).await;
            match relay_state {
                Some(relay_state) => {
                    crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                        &config,
                        &relay_state,
                        target,
                        request,
                        REMOTE_PROMPT_RECEIPT_QUERY_TIMEOUT,
                    )
                    .await
                }
                None => {
                    crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                        &config,
                        target,
                        request,
                        REMOTE_PROMPT_RECEIPT_QUERY_TIMEOUT,
                    )
                    .await
                }
            }
        }
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn query_remote_queued_steer_receipt(
    state: &KernelRuntimeState,
    agent_id: &str,
    queued_prompt_id: &str,
    worker_kernel_id: &str,
    worker_machine_id: &str,
    leased_agent_id: &str,
    target_home_prompt_id: &str,
    worker_provider_run_id: &str,
    execution_lease_id: &str,
) -> Result<Option<crate::transport::relay_peer::LeasedPromptReceipt>, DaemonError> {
    let agent = state.owned.agent_store.get_agent(agent_id)?;
    let remote_execution = agent
        .remote_execution()
        .cloned()
        .filter(|binding| {
            binding.worker_kernel_id == worker_kernel_id
                && binding.worker_machine_id == worker_machine_id
                && binding.leased_agent_id == leased_agent_id
                && binding.execution_lease_id == execution_lease_id
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "query queued steer receipt",
            message: "the current remote worker binding no longer matches the queued steer"
                .to_string(),
        })?;
    if !remote_execution.relay_peer_protocol_compatible() {
        return Err(DaemonError::LocalTransport {
            operation: "query queued steer receipt",
            message: format!(
                "worker `{worker_kernel_id}` does not support queued-steer receipt reconciliation"
            ),
        });
    }
    let relay_config = state
        .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&remote_execution))
        .await;
    let response =
        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &relay_config,
            ClientTarget {
                daemon_id: Some(worker_kernel_id.to_string()),
                daemon_alias: None,
            },
            RelayPeerRequest::ReconcileLeasedPromptSteerReceipt {
                leased_agent_id: leased_agent_id.to_string(),
                steer_id: queued_prompt_id.to_string(),
                target_home_prompt_id: target_home_prompt_id.to_string(),
                worker_provider_run_id: worker_provider_run_id.to_string(),
                execution_lease_id: execution_lease_id.to_string(),
            },
            REMOTE_PROMPT_RECEIPT_QUERY_TIMEOUT,
        )
        .await?;
    match response {
        RelayPeerResponse::LeasedPromptReceiptQueried { receipt } => {
            if receipt
                .as_ref()
                .is_some_and(|receipt| receipt.home_prompt_id != queued_prompt_id)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "reconcile queued steer receipt",
                    message: "worker returned a receipt for a different queued prompt".to_string(),
                });
            }
            Ok(receipt)
        }
        other => Err(DaemonError::LocalTransport {
            operation: "query queued steer receipt",
            message: format!("unexpected worker receipt response: {other:?}"),
        }),
    }
}

pub(super) async fn query_remote_prompt_worker_receipt_with_transport<F, Fut>(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
    send_request: F,
) -> Result<Option<crate::transport::relay_peer::LeasedPromptReceipt>, DaemonError>
where
    F: FnOnce(crate::config::DaemonConfig, ClientTarget, RelayPeerRequest) -> Fut + Send,
    Fut: Future<Output = Result<RelayPeerResponse, DaemonError>> + Send,
{
    let agent = state.owned.agent_store.get_agent(&dispatch.agent_id)?;
    let remote_execution = agent
        .remote_execution()
        .cloned()
        .filter(|binding| {
            binding.worker_kernel_id == dispatch.worker_kernel_id
                && binding.leased_agent_id == dispatch.leased_agent_id
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "query leased prompt receipt",
            message: "the current remote worker binding no longer matches the dispatch".to_string(),
        })?;
    if !remote_execution.relay_peer_protocol_compatible() {
        return Err(DaemonError::LocalTransport {
            operation: "query leased prompt receipt",
            message: format!(
                "worker `{}` does not support the leased prompt receipt query protocol",
                dispatch.worker_kernel_id
            ),
        });
    }
    let relay_config = state
        .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&remote_execution))
        .await;
    let target = ClientTarget {
        daemon_id: Some(dispatch.worker_kernel_id.clone()),
        daemon_alias: None,
    };
    let request = RelayPeerRequest::GetLeasedPromptReceipt {
        leased_agent_id: dispatch.leased_agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
    };
    // The app lock used to derive relay configuration is released before invoking transport.
    let response = send_request(relay_config, target, request).await?;
    match response {
        RelayPeerResponse::LeasedPromptReceiptQueried { receipt } => {
            if let Some(receipt) = receipt.as_ref() {
                if receipt.home_prompt_id != dispatch.prompt_id
                    || receipt.worker_provider_run_id.trim().is_empty()
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "query leased prompt receipt",
                        message: "worker returned a conflicting or incomplete prompt receipt"
                            .to_string(),
                    });
                }
            }
            Ok(receipt)
        }
        other => Err(DaemonError::LocalTransport {
            operation: "query leased prompt receipt",
            message: format!("unexpected worker receipt query response: {other:?}"),
        }),
    }
}

fn ensure_remote_prompt_dispatching(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Result<(), DaemonError> {
    let session = state
        .owned
        .session_store
        .get_session(&dispatch.session_id)?;
    let active = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &dispatch.agent_id)
        .filter(|prompt| prompt.id() == dispatch.prompt_id)
        .ok_or_else(|| DaemonError::NoActivePrompt {
            session_id: dispatch.session_id.clone(),
        })?;
    if active.durable_delivery_phase()
        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
    {
        return Ok(());
    }
    state.owned.mark_active_prompt_delivery(
        &dispatch.session_id,
        &dispatch.agent_id,
        &dispatch.prompt_id,
        crate::session::DurablePromptDeliveryPhase::Dispatching,
        None,
        None,
    )?;
    Ok(())
}

fn clear_reconciliation_pending_or_defer(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Result<(), DaemonError> {
    if let Err(error) = persist_remote_prompt_reconciliation_pending(state, dispatch, false) {
        let detail = "the submission was rejected before admission or definitely not sent, but the reconciliation hold could not be cleared";
        if remote_prompt_reconciliation_pending(state, dispatch).unwrap_or(true) {
            state.report_remote_prompt_reconciliation_pending(dispatch, detail);
            return Err(remote_prompt_reconciliation_pending_error(dispatch, detail));
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn persist_remote_prompt_reconciliation_pending(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
    pending: bool,
) -> Result<(), DaemonError> {
    let session = state
        .owned
        .session_store
        .get_session(&dispatch.session_id)?;
    let previous = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &dispatch.agent_id)
        .filter(|prompt| prompt.id() == dispatch.prompt_id)
        .ok_or_else(|| DaemonError::NoActivePrompt {
            session_id: dispatch.session_id.clone(),
        })?;
    if previous.durable_delivery_reconciliation_pending() == pending {
        return Ok(());
    }
    let mut replacement = previous.clone();
    replacement.set_durable_delivery_reconciliation_pending(pending);
    if !state
        .owned
        .prompt_state_owner
        .replace_active_prompt_if_matches(
            &session,
            &dispatch.agent_id,
            &previous,
            replacement.clone(),
        )
    {
        return Err(DaemonError::NoActivePrompt {
            session_id: dispatch.session_id.clone(),
        });
    }
    let (active_prompt, queued_prompts) = state
        .owned
        .prompt_state_owner
        .state_parts(&session, &dispatch.agent_id);
    if let Err(error) = state.owned.mirror_prompt_owner_agent_state(
        &dispatch.session_id,
        &dispatch.agent_id,
        active_prompt,
        queued_prompts,
    ) {
        let _ = state
            .owned
            .prompt_state_owner
            .replace_active_prompt_if_matches(&session, &dispatch.agent_id, &replacement, previous);
        let (active_prompt, queued_prompts) = state
            .owned
            .prompt_state_owner
            .state_parts(&session, &dispatch.agent_id);
        let _ = state.owned.session_store.mirror_agent_prompt_state(
            &dispatch.session_id,
            &dispatch.agent_id,
            active_prompt,
            queued_prompts,
        );
        return Err(error);
    }
    Ok(())
}

pub(super) fn remote_prompt_reconciliation_pending(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Result<bool, DaemonError> {
    let session = state
        .owned
        .session_store
        .get_session(&dispatch.session_id)?;
    Ok(state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &dispatch.agent_id)
        .filter(|prompt| prompt.id() == dispatch.prompt_id)
        .is_some_and(|prompt| prompt.durable_delivery_reconciliation_pending()))
}

pub(super) fn remote_prompt_reconciliation_pending_error(
    dispatch: &crate::app::KernelRemotePromptDispatch,
    detail: &str,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "remote prompt delivery reconciliation pending",
        message: format!(
            "Remote delivery of prompt `{}` to worker kernel `{}` is uncertain: {detail}. The same prompt remains active in durable Dispatching state; it was not cancelled, promoted, or replayed. Do not replay until the exact worker receipt is reconciled.",
            dispatch.prompt_id, dispatch.worker_kernel_id
        ),
    }
}

pub(super) fn remote_prompt_error_is_reconciliation_pending(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport {
            operation: "remote prompt delivery reconciliation pending",
            ..
        }
    )
}

fn remote_prompt_error_requires_provider_launch_credential(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::RelayTransport { operation, code, .. }
            if matches!(*operation, "read relay peer response" | "read temporary relay peer response")
                && code == crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
    )
}

async fn remote_prompt_provider_launch_credential_if_needed(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Result<Option<crate::transport::relay_peer::RemoteProviderLaunchCredential>, DaemonError> {
    let agent = state.owned.agent_store.get_agent(&dispatch.agent_id)?;
    if agent
        .remote_execution()
        .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
        .is_some()
    {
        return Ok(None);
    }
    state
        .resolve_remote_provider_launch_credential(
            &dispatch.session_id,
            &dispatch.agent_id,
            "launch remote provider run",
        )
        .await
}

async fn refresh_remote_prompt_binding(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
) -> Result<(), DaemonError> {
    crate::logging::warn_with_fields(
        "daemon.remote_prompt_dispatch",
        "remote prompt lease stale; refreshing binding",
        serde_json::json!({
            "session_id": dispatch.session_id,
            "agent_id": dispatch.agent_id,
            "worker_kernel_id": dispatch.worker_kernel_id,
            "leased_agent_id": dispatch.leased_agent_id,
        }),
    );
    let agent_id = dispatch.agent_id.clone();
    let agent = state
        .with_app_side_effect_blocking(move |app| app.refresh_remote_agent_binding(&agent_id))
        .await?;
    let Some(remote_execution) = agent.remote_execution().cloned() else {
        return Err(DaemonError::LocalTransport {
            operation: "refresh remote prompt binding",
            message: format!(
                "agent `{}` did not have remote execution after binding refresh",
                dispatch.agent_id
            ),
        });
    };
    dispatch.worker_kernel_id = remote_execution.worker_kernel_id;
    dispatch.leased_agent_id = remote_execution.leased_agent_id;
    dispatch.relay_url = remote_execution.relay_url;
    dispatch.relay_token = remote_execution.relay_token;
    Ok(())
}

fn remote_prompt_dispatch_is_current(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> bool {
    let Ok(session) = state.owned.session_store.get_session(&dispatch.session_id) else {
        return false;
    };
    state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &dispatch.agent_id)
        .is_some_and(|prompt| prompt.id() == dispatch.prompt_id)
}

pub(super) fn remote_prompt_transport_retry_delay(attempt: u32) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(250_u64.saturating_mul(multiplier))
}

fn remote_prompt_transport_retry_window_expired(elapsed: std::time::Duration) -> bool {
    elapsed >= REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW
}

fn classify_remote_prompt_submission_outcome(
    result: Result<String, DaemonError>,
    request_transport_started: bool,
) -> RemotePromptSubmissionOutcome {
    match result {
        Ok(provider_run_id) => RemotePromptSubmissionOutcome::Accepted(provider_run_id),
        Err(error) if remote_prompt_error_is_known_pre_admission_rejection(&error) => {
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
        }
        Err(error)
            if !request_transport_started
                || remote_prompt_error_is_definitely_pre_send_after_transport_start(&error) =>
        {
            RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
        }
        Err(error) => RemotePromptSubmissionOutcome::Indeterminate(error),
    }
}

fn remote_prompt_error_is_known_pre_admission_rejection(error: &DaemonError) -> bool {
    let DaemonError::RelayTransport {
        operation, code, ..
    } = error
    else {
        return false;
    };
    matches!(
        *operation,
        "read relay peer response" | "read temporary relay peer response"
    ) && matches!(
        code.as_str(),
        crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
            | "leased_agent_not_found"
            | "execution_lease_not_found"
            | "target_not_connected"
            | "target_not_allowed"
            | "action_not_allowed"
    )
}

fn remote_prompt_error_is_definitely_pre_send_after_transport_start(error: &DaemonError) -> bool {
    let operation = match error {
        DaemonError::LocalTransport { operation, .. }
        | DaemonError::RelayTransport { operation, .. } => *operation,
        _ => return false,
    };
    matches!(
        operation,
        "connect temporary relay peer socket"
            | "serialize temporary relay register"
            | "write temporary relay register"
            | "send relay peer request"
            | "serialize relay peer request"
            | "encrypt relay payload"
            | "get_live_kernel"
            | "relay_metadata_query"
            | "connect relay metadata socket"
            | "write relay metadata request"
            | "read relay metadata response"
            | "decode relay metadata response"
    )
}

pub(super) fn remote_prompt_unavailable_slice_error(
    slice_store: &crate::slice::SliceStore,
    remote_execution: &crate::agent::RemoteAgentBinding,
    session_id: &str,
    agent_id: &str,
) -> Option<DaemonError> {
    let slice = slice_store
        .list_by_session(session_id)
        .into_iter()
        .find(|slice| {
            slice
                .agent_ids
                .iter()
                .any(|candidate| candidate == agent_id)
        })
        .or_else(|| slice_store.resolve_by_worker_kernel_ref(&remote_execution.worker_kernel_id))
        .or_else(|| {
            slice_store.resolve_by_worker_kernel_ref(&remote_execution.worker_machine_id)
        })?;
    if remote_prompt_slice_status_allows_transport_retry(&slice.status) {
        return None;
    }
    let status = format!("{:?}", slice.status).to_ascii_lowercase();
    Some(DaemonError::LocalTransport {
        operation: "submit remote prompt to unavailable slice",
        message: format!(
            "agent `{agent_id}` is deployed in {status} slice `{}`; start the slice before sending a prompt (worker kernel `{}` is unreachable)",
            slice.name, remote_execution.worker_kernel_id
        ),
    })
}

fn remote_prompt_dispatch_unavailable_slice_error(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Option<DaemonError> {
    let agent = state.owned.agent_store.get_agent(&dispatch.agent_id).ok()?;
    let remote_execution = agent.remote_execution()?;
    remote_prompt_unavailable_slice_error(
        &state.owned.slice_store,
        remote_execution,
        &dispatch.session_id,
        &dispatch.agent_id,
    )
}

fn remote_prompt_slice_status_allows_transport_retry(status: &crate::slice::SliceStatus) -> bool {
    matches!(
        status,
        crate::slice::SliceStatus::Starting | crate::slice::SliceStatus::Running
    )
}

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn submit_remote_prompt_attempt(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        prompt: String,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
        unexpected_response_message: &'static str,
    ) -> Result<String, DaemonError> {
        let attempt = self
            .submit_remote_prompt_attempt_raw(
                dispatch,
                prompt,
                attachments,
                provider_launch_credential,
                unexpected_response_message,
            )
            .await;
        attempt.result
    }

    async fn submit_remote_prompt_attempt_with_outcome(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        prompt: String,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
        unexpected_response_message: &'static str,
    ) -> RemotePromptSubmissionOutcome {
        let attempt = self
            .submit_remote_prompt_attempt_raw(
                dispatch,
                prompt,
                attachments,
                provider_launch_credential,
                unexpected_response_message,
            )
            .await;
        classify_remote_prompt_submission_outcome(attempt.result, attempt.request_transport_started)
    }

    async fn submit_remote_prompt_attempt_raw(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        prompt: String,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
        unexpected_response_message: &'static str,
    ) -> RemotePromptSubmissionAttempt {
        let agent_id = dispatch.agent_id.clone();
        let expected_leased_agent_id = dispatch.leased_agent_id.clone();
        let hidden_system_context = dispatch.hidden_system_context.clone();
        let workflow_context = dispatch.workflow_context.clone();
        let git_context = remote_git_turn_context(dispatch);
        let callback_state = self.clone();
        let transport_started = std::sync::Arc::new(AtomicBool::new(false));
        let mark_transport_started = std::sync::Arc::clone(&transport_started);
        let result = self.with_current_remote_extension_manifest(
            &agent_id,
            &expected_leased_agent_id,
            move |agent, remote_execution, manifest| async move {
                if !remote_execution.relay_peer_protocol_compatible() {
                    return Err(DaemonError::LocalTransport {
                        operation: "dispatch remote agent prompt",
                        message: format!(
                            "remote worker `{}` has an incompatible or legacy relay peer protocol {:?}; rebind the remote agent before dispatch (current protocol {})",
                            remote_execution.worker_kernel_id,
                            remote_execution.relay_peer_protocol_version,
                            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                        ),
                    });
                }
                let native_provider_run =
                    callback_state.remote_agent_has_native_provider_run(&agent);
                let required_mcps = if native_provider_run {
                    callback_state.required_remote_mcps_for_native_provider_launch(&agent)?
                } else {
                    callback_state.required_remote_mcps_for_agent(&agent)?
                };
                let required_skills = if native_provider_run {
                    Some(
                        callback_state
                            .required_remote_skills_for_native_provider_launch(&agent)?,
                    )
                } else {
                    None
                };
                let manifest = if native_provider_run {
                    manifest.without_mcp_tools()
                } else {
                    manifest
                };
                let mut config = callback_state.config_snapshot().await;
                if let (Some(relay_url), Some(relay_token)) = (
                    remote_execution.relay_url.clone(),
                    remote_execution.relay_token.clone(),
                ) {
                    config.apply_remote_relay_override(relay_url, relay_token);
                }
                let target = ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                };
                let request = RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id: remote_execution.leased_agent_id,
                    expected_profile:
                        crate::transport::relay_peer::RelayAgentExecutionProfile::from(&agent),
                    prompt,
                    hidden_system_context,
                    attachments,
                    workflow_context,
                    git_context: Some(git_context),
                    required_mcps,
                    required_skills,
                    remote_extension_manifest: manifest,
                    provider_launch_credential,
                };
                let relay_state = callback_state.connected_relay_state_for_config(&config).await;
                match relay_state {
                    Some(relay_state) => {
                        mark_transport_started.store(true, Ordering::Relaxed);
                        crate::transport::relay_client::enqueue_peer_request_via_connected_relay_with_timeout(
                            &config,
                            &relay_state,
                            target,
                            request,
                            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                        )
                        .await
                    }
                    None => {
                        mark_transport_started.store(true, Ordering::Relaxed);
                        // Temporary sockets have no shared FIFO sender with manifest updates, so
                        // keep this fallback serialized through its response.
                        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                            &config,
                            target,
                            request,
                            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                        )
                        .await?;
                        Ok(crate::transport::relay_client::RelayPeerResponseWaiter::ready(
                            response,
                        ))
                    }
                }
            },
        )
        .await;
        let result = match result {
            Ok(waiter) => match waiter.wait().await {
                Ok(RelayPeerResponse::LeasedPromptSubmitted {
                    provider_run_id, ..
                }) => Ok(provider_run_id),
                Ok(other) => Err(DaemonError::LocalTransport {
                    operation: "submit remote prepared prompt",
                    message: format!("{unexpected_response_message}: {other:?}"),
                }),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        RemotePromptSubmissionAttempt {
            result,
            request_transport_started: transport_started.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    remote_prompt_error_should_refresh_binding(error)
}

#[cfg(test)]
fn remote_prompt_dispatch_requires_provider_launch_credential(
    result: &Result<String, DaemonError>,
) -> bool {
    let Err(error) = result else {
        return false;
    };
    let required_code =
        crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE;
    match error {
        DaemonError::LocalTransport { message, .. } => message.contains(required_code),
        DaemonError::RelayTransport { code, message, .. } => {
            code == required_code || message.contains(required_code)
        }
        _ => false,
    }
}

pub(super) fn remote_prompt_error_should_refresh_binding(error: &DaemonError) -> bool {
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
        }
        DaemonError::RelayTransport { code, message, .. } => {
            code == "leased_agent_not_found"
                || code == "execution_lease_not_found"
                || message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
        }
        _ => false,
    }
}

pub(super) fn remote_prompt_error_should_retry_transport(error: &DaemonError) -> bool {
    let (operation, message) = match error {
        DaemonError::RelayTransport {
            operation,
            retryable: true,
            ..
        } => {
            return matches!(
                *operation,
                "read relay peer response" | "read temporary relay peer response"
            )
        }
        DaemonError::RelayTransport { .. } => return false,
        DaemonError::LocalTransport { operation, message } => (*operation, message.as_str()),
        _ => return false,
    };
    if matches!(
        operation,
        "connect temporary relay peer socket"
            | "write temporary relay register"
            | "write temporary relay peer request"
    ) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    let transient_message = [
        "target daemon is not connected to relay",
        "target daemon disconnected from relay",
        "relay is not connected",
        "relay peer request was cancelled",
        "timed out waiting for relay peer response",
        "not currently visible on relay",
        "did not appear on relay",
        "connection reset",
        "connection refused",
        "connection closed",
        "relay closed temporary peer connection",
        "closed without",
        "broken pipe",
        "temporarily unavailable",
        "websocket",
    ]
    .iter()
    .any(|candidate| message.contains(candidate));
    transient_message
        && matches!(
            operation,
            "send relay peer request"
                | "read relay peer response"
                | "read temporary relay peer response"
                | "get_live_kernel"
                | "relay_metadata_query"
                | "connect relay metadata socket"
                | "write relay metadata request"
                | "read relay metadata response"
        )
}

fn remote_git_turn_context(
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        source_attachment_id: Some(dispatch.source_attachment_id.clone()),
        workspace_live_sync_mode: dispatch.workspace_live_sync_mode,
        prompt_origin: Some(dispatch.prompt_origin),
        external_provider: dispatch.external_provider.clone(),
        external_provider_session_id: dispatch.external_provider_session_id.clone(),
        external_provider_turn_id: dispatch.external_provider_turn_id.clone(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::app::{DaemonApp, KernelPreparedPromptSubmission};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
    use chariox_relay::protocol::RelayEnvelope;
    use tokio::sync::Mutex;

    const CANCELLATION_GUARD_RELAY_URL: &str = "ws://127.0.0.1:43179";
    const CANCELLATION_GUARD_WORKER_ID: &str = "worker-cancel-before-submit";
    const CANCELLATION_GUARD_LEASED_AGENT_ID: &str = "leased-agent-cancel-before-submit";

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    async fn receive_fake_worker_request(
        receiver: &mut tokio::sync::mpsc::Receiver<RelayEnvelope>,
        worker_private_key: &str,
    ) -> (String, crate::transport::relay_peer::RelayPeerRequest) {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
            .await
            .expect("fake relay should receive the peer request")
            .expect("fake relay request channel should remain open");
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = envelope
        else {
            panic!("expected a daemon peer request");
        };
        let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            worker_private_key,
            &encrypted_request,
        )
        .expect("fake worker should decrypt the peer request");
        let request = serde_json::from_slice(&decrypted.plaintext)
            .expect("fake worker request should decode");
        (request_id, request)
    }

    #[tokio::test]
    async fn cancelling_accepted_remote_prompt_never_sends_submit_request() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
        config.relay_token = Some("cancel-before-submit-test-token".to_string());
        let worker_config = crate::config::DaemonConfig::for_tests();

        let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "cancel-before-submit-workspace",
                "cancel-before-submit-worktree",
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cancel-before-submit-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("home attachment should be created");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                    worker_machine_id: "worker-machine-cancel-before-submit".to_string(),
                    execution_lease_id: "lease-cancel-before-submit".to_string(),
                    leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to the fake worker");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let mut submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending:cancel-before-submit",
                    &attachment_id,
                    &agent_id,
                    "this accepted prompt is cancelled before dispatch",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should be admitted")
            .expect("remote prompt should produce a dispatch");
        let mut dispatch = submission
            .remote_dispatch
            .take()
            .expect("accepted remote prompt should carry a dispatch");
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &runtime
                        .owned
                        .session_store
                        .get_session(&session_id)
                        .expect("session should remain available"),
                    &agent_id,
                )
                .expect("accepted prompt should remain active")
                .durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
        runtime
            .owned
            .begin_remote_prompt_cancellation(&session_id, &agent_id, &attachment_id)
            .expect("cancellation intent should persist before dispatch begins");

        let relay_state = Arc::clone(&runtime.owned.relay_state);
        let (outgoing_tx, mut peer_requests, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(8);
        {
            let mut relay = relay_state.write().await;
            relay.test_set_connected_sender(outgoing_tx, CANCELLATION_GUARD_RELAY_URL);
            relay.remember_peer_public_key(
                CANCELLATION_GUARD_WORKER_ID,
                worker_config.relay_public_key.clone(),
            );
        }
        let worker_private_key = worker_config.relay_private_key.clone();
        let prompt = dispatch.prompt.clone();
        let submission = submit_remote_prompt_to_worker_with_binding_refresh(
            &runtime,
            &mut dispatch,
            prompt,
            Vec::new(),
        );
        tokio::pin!(submission);
        tokio::select! {
            result = &mut submission => {
                let error = result.expect_err("cancelled Accepted prompt must not be submitted");
                assert!(error.to_string().contains("cannot start remote dispatch"));
                assert!(
                    peer_requests.try_recv().is_err(),
                    "cancelled Accepted prompt must send no relay request"
                );
            }
            envelope = peer_requests.recv() => {
                let envelope = envelope.expect("fake relay should receive any unexpected request");
                let RelayEnvelope::DaemonPeerRequest {
                    encrypted_request,
                    ..
                } = envelope else {
                    panic!("unexpected fake relay envelope: {envelope:?}");
                };
                let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                    &worker_private_key,
                    &encrypted_request,
                )
                .expect("fake worker should decrypt an unexpected request");
                let request: crate::transport::relay_peer::RelayPeerRequest =
                    serde_json::from_slice(&decrypted.plaintext)
                        .expect("fake worker request should decode");
                assert!(
                    matches!(
                        request,
                        crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt { .. }
                    ),
                    "unexpected relay request before SubmitLeasedPrompt: {request:?}"
                );
                panic!(
                    "cancelled Accepted prompt sent SubmitLeasedPrompt before cancellation settled"
                );
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                panic!("dispatch neither rejected cancellation nor produced a test relay request");
            }
        }
    }

    #[tokio::test]
    async fn connected_held_prompt_response_does_not_block_remote_manifest_sync() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
        config.relay_token = Some("manifest-lane-test-token".to_string());
        let home_public_key = config.relay_public_key.clone();
        let worker_config = crate::config::DaemonConfig::for_tests();

        let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "manifest-lane-workspace",
                "manifest-lane-worktree",
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "manifest-lane-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("home attachment should be created");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                    worker_machine_id: "worker-machine-manifest-lane".to_string(),
                    execution_lease_id: "lease-manifest-lane".to_string(),
                    leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to the fake worker");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let mut submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending:manifest-lane",
                    &attachment_id,
                    &agent_id,
                    "hold the worker response while manifest sync runs",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should be admitted")
            .expect("remote prompt should produce a dispatch");
        let mut dispatch = submission
            .remote_dispatch
            .take()
            .expect("accepted remote prompt should carry a dispatch");
        let submitted_prompt = PromptQueueItem::new(
            format!("worker-{}", dispatch.prompt_id),
            "worker-attachment",
            &dispatch.leased_agent_id,
            dispatch.prompt.clone(),
            PromptStatus::Running,
        );

        let relay_state = Arc::clone(&runtime.owned.relay_state);
        let (outgoing_tx, mut peer_requests, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(8);
        {
            let mut relay = relay_state.write().await;
            relay.test_set_connected_sender(outgoing_tx, CANCELLATION_GUARD_RELAY_URL);
            relay.remember_peer_public_key(
                CANCELLATION_GUARD_WORKER_ID,
                worker_config.relay_public_key.clone(),
            );
        }

        let submit_runtime = runtime.clone();
        let submit_prompt = dispatch.prompt.clone();
        let mut submit_task = tokio::spawn(async move {
            submit_remote_prompt_to_worker_with_binding_refresh(
                &submit_runtime,
                &mut dispatch,
                submit_prompt,
                Vec::new(),
            )
            .await
        });

        let (submit_request_id, request) = receive_fake_worker_request(
            &mut peer_requests,
            &worker_config.relay_private_key,
        )
        .await;
        assert!(
            matches!(
                &request,
                crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt { .. }
            ),
            "first fake relay request should submit the held prompt: {request:?}"
        );

        let sync_runtime = runtime.clone();
        let sync_agent = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .expect("home agent should remain available");
        let sync_task = tokio::spawn(async move {
            sync_runtime
                .sync_remote_extension_manifest_for_agent(&sync_agent, None, None)
                .await
        });

        let (manifest_request_id, request) = receive_fake_worker_request(
            &mut peer_requests,
            &worker_config.relay_private_key,
        )
        .await;
        assert_ne!(manifest_request_id, submit_request_id);
        let crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id,
            ..
        } = request
        else {
            panic!("manifest sync should proceed while prompt response is held: {request:?}");
        };
        assert_eq!(leased_agent_id, CANCELLATION_GUARD_LEASED_AGENT_ID);

        let manifest_response = crate::transport::relay_peer::RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
            leased_agent_id: leased_agent_id.clone(),
        };
        let encrypted_manifest_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker_config.relay_private_key,
            &home_public_key,
            &serde_json::to_vec(&manifest_response).expect("manifest response should encode"),
        )
        .expect("fake worker should encrypt the manifest response");
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &relay_state,
            manifest_request_id,
            CANCELLATION_GUARD_WORKER_ID.to_string(),
            encrypted_manifest_response,
        )
        .await;
        sync_task
            .await
            .expect("manifest sync task should not panic")
            .expect("manifest sync should succeed before prompt response is released");
        assert!(
            !submit_task.is_finished(),
            "the fake worker must still be holding the prompt response"
        );

        let submit_response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "provider-run-manifest-lane".to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started {
                prompt: submitted_prompt,
            },
        };
        let encrypted_submit_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker_config.relay_private_key,
            &home_public_key,
            &serde_json::to_vec(&submit_response).expect("submit response should encode"),
        )
        .expect("fake worker should encrypt the submit response");
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &relay_state,
            submit_request_id,
            CANCELLATION_GUARD_WORKER_ID.to_string(),
            encrypted_submit_response,
        )
        .await;
        assert_eq!(
            submit_task
                .await
                .expect("prompt submission task should not panic")
                .expect("prompt submission should succeed"),
            "provider-run-manifest-lane"
        );
    }

    const WORKFLOW_CREDENTIAL_CANARY: &str = "workflow-submit-token-canary";

    #[derive(Clone, Copy)]
    enum WorkflowCredentialState {
        MissingVault,
        MissingCredential,
        Locked,
        Unlocked,
        ActiveWorkerRun,
    }

    struct WorkflowSubmissionFixture {
        runtime: KernelRuntimeState,
        dispatch: crate::app::KernelRemotePromptDispatch,
        root: std::path::PathBuf,
        previous_home: Option<std::ffi::OsString>,
    }

    impl WorkflowSubmissionFixture {
        fn new(credential_state: WorkflowCredentialState) -> Self {
            use crate::app::KernelSessionService;
            use crate::attachment::{AttachRequest, ClientCapabilityLevel};
            use crate::config::{CredentialVaultBackend, DaemonConfig};
            use crate::session::{CreateSessionRequest, DEFAULT_LOCAL_USER_ID};
            use std::sync::Arc;

            static NEXT_FIXTURE_ID: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "chariox-remote-workflow-submit-credential-{}-{}-{fixture_id}",
                std::process::id(),
                crate::session::unix_epoch_ms()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let previous_home = std::env::var_os("CHARIOX_HOME");
            std::env::set_var("CHARIOX_HOME", &root);
            crate::secret::clear_vault_secret_process_cache().unwrap();

            let mut config =
                DaemonConfig::for_tests().with_session_history_root(root.join("history"));
            config.user_config.state.path = Some(root.join("state.db").display().to_string());
            config.user_config.history.operational.path =
                Some(root.join("operations.db").display().to_string());
            config.user_config.artifacts.operational.root =
                Some(root.join("artifacts").display().to_string());
            config.user_config.artifacts.operational.index_path =
                Some(root.join("artifacts.db").display().to_string());
            config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
            config.user_config.credential_vault.path =
                root.join("credentials.vault").display().to_string();
            // A missing relay makes reaching transport distinguishable from failing credential
            // admission, without sending a SubmitLeasedPrompt to any worker.
            config.relay_url = None;
            let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
            let (session, _) = KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                ))
                .unwrap();
            let profile = app
                .provider_account_profile_registry()
                .create_managed(DEFAULT_LOCAL_USER_ID, "claude", "Workflow submit fixture")
                .unwrap();
            crate::test_support::authenticate_provider_account(
                &app.provider_account_profile_registry(),
                DEFAULT_LOCAL_USER_ID,
                "claude",
                &profile.profile_id,
            )
            .expect("workflow fixture account should be authenticated");
            let agent = KernelSessionService::new(&mut app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(session.id(), "claude")
                        .with_account_profile(profile.profile_id.clone()),
                )
                .unwrap();
            let attachment = KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "workflow-submit-credential-test",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap();
            let vault_path = root.join("credentials.vault");
            if matches!(credential_state, WorkflowCredentialState::MissingCredential) {
                crate::secret::unlock_chariox_encrypted_vault(
                    &vault_path,
                    "fixture passphrase",
                    crate::secret::VaultUnlockLease::KernelShutdown,
                )
                .unwrap();
            } else if matches!(
                credential_state,
                WorkflowCredentialState::Locked | WorkflowCredentialState::Unlocked
            ) {
                crate::secret::unlock_chariox_encrypted_vault(
                    &vault_path,
                    "fixture passphrase",
                    crate::secret::VaultUnlockLease::KernelShutdown,
                )
                .unwrap();
                crate::provider::store_provider_account_credential(
                    app.config(),
                    DEFAULT_LOCAL_USER_ID,
                    "claude",
                    &profile.profile_id,
                    WORKFLOW_CREDENTIAL_CANARY,
                    false,
                )
                .unwrap();
                if matches!(credential_state, WorkflowCredentialState::Locked) {
                    crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
                    crate::secret::clear_vault_secret_process_cache().unwrap();
                }
            }
            app.agents()
                .bind_remote_execution(
                    agent.id(),
                    crate::agent::RemoteAgentBinding {
                        worker_kernel_id: "workflow-worker".into(),
                        worker_machine_id: "workflow-worker-machine".into(),
                        execution_lease_id: "workflow-lease".into(),
                        leased_agent_id: "workflow-leased-agent".into(),
                        active_worker_provider_run_id: matches!(
                            credential_state,
                            WorkflowCredentialState::ActiveWorkerRun
                        )
                        .then(|| "already-active-worker-run".into()),
                        relay_url: None,
                        relay_token: None,
                        relay_peer_protocol_version: Some(
                            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                        ),
                    },
                )
                .unwrap();

            let app = Arc::new(tokio::sync::Mutex::new(app));
            let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
                app,
                crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
            );
            let runtime = router.runtime_state();
            let session_id = session.id().to_string();
            let agent_id = agent.id().to_string();
            let attachment_id = attachment.id().to_string();
            let submission = runtime
                .owned
                .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt: crate::session::PromptQueueItem::new(
                        "pending:workflow-submit-credential",
                        &attachment_id,
                        &agent_id,
                        "workflow prompt credential admission",
                        crate::session::PromptStatus::Queued,
                    ),
                    force_queue: false,
                    refresh_projection: true,
                })
                .unwrap()
                .unwrap();
            let mut dispatch = submission.remote_dispatch.unwrap();
            dispatch.workflow_context = Some(crate::execution_lease::RemoteWorkflowTurnContext {
                home_kernel_id: "home-kernel".into(),
                home_session_id: session_id,
                home_agent_id: agent_id,
                workflow_run_id: "workflow-run".into(),
                workflow_node_run_id: "workflow-node-run".into(),
                delivery_token: "workflow-delivery-token".into(),
                event_reply_enabled: false,
                event_context_enabled: false,
                event_actions_enabled: false,
            });
            Self {
                runtime,
                dispatch,
                root,
                previous_home,
            }
        }

        async fn submit(&mut self) -> Result<String, DaemonError> {
            let prompt = self.dispatch.prompt.clone();
            submit_remote_prompt_to_worker_with_binding_refresh(
                &self.runtime,
                &mut self.dispatch,
                prompt,
                Vec::new(),
            )
            .await
        }

        async fn submit_after_rejecting_vault_unlock(&self) -> Result<String, DaemonError> {
            let runtime = self.runtime.clone();
            let mut dispatch = self.dispatch.clone();
            let session_id = dispatch.session_id.clone();
            let agent_id = dispatch.agent_id.clone();
            let prompt = dispatch.prompt.clone();
            let submission = submit_remote_prompt_to_worker_with_binding_refresh(
                &runtime,
                &mut dispatch,
                prompt,
                Vec::new(),
            );
            tokio::pin!(submission);
            let mut completed_submission = None;
            let interaction = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let session = runtime
                        .owned
                        .session_store
                        .get_session(&session_id)
                        .expect("session should remain available");
                    if let Some(interaction) = session.active_interaction_for_agent(&agent_id) {
                        break Some(interaction.clone());
                    }
                    tokio::select! {
                        result = &mut submission => {
                            completed_submission = Some(result);
                            return None;
                        }
                        _ = tokio::task::yield_now() => {}
                    }
                }
            })
            .await
            .expect("locked vault should request a passphrase before transport");
            if let Some(result) = completed_submission {
                return result;
            }
            let interaction = interaction.expect("vault interaction should be observed");
            assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
            runtime
                .resolve_runtime_interaction(&session_id, interaction.id(), "cancel", None)
                .await
                .expect("vault cancellation should resolve");
            submission.await
        }
    }

    impl Drop for WorkflowSubmissionFixture {
        fn drop(&mut self) {
            let _ =
                crate::secret::lock_chariox_encrypted_vault(&self.root.join("credentials.vault"));
            let _ = crate::secret::clear_vault_secret_process_cache();
            match self.previous_home.take() {
                Some(value) => std::env::set_var("CHARIOX_HOME", value),
                None => std::env::remove_var("CHARIOX_HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn classify_error(
        error: DaemonError,
        request_transport_started: bool,
    ) -> RemotePromptSubmissionOutcome {
        classify_remote_prompt_submission_outcome(Err(error), request_transport_started)
    }

    #[test]
    fn remote_prompt_submission_classifies_pre_send_rejection_and_indeterminate_transport() {
        let definitely_pre_send = classify_error(
            DaemonError::LocalTransport {
                operation: "send relay peer request",
                message: "relay is not connected".to_string(),
            },
            true,
        );
        assert!(matches!(
            &definitely_pre_send,
            RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
                if remote_prompt_error_should_retry_transport(error)
        ));

        let known_rejection = classify_error(
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "leased_agent_not_found".to_string(),
                message: "worker rejected the stale lease before admission".to_string(),
                retryable: false,
            },
            true,
        );
        assert!(matches!(
            known_rejection,
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(_)
        ));

        let relay_admission_rejection = classify_error(
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "target_not_connected".to_string(),
                message: "target daemon is not connected to relay".to_string(),
                retryable: true,
            },
            true,
        );
        assert!(matches!(
            &relay_admission_rejection,
            RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
                if remote_prompt_error_should_retry_transport(error)
        ));

        let response_timeout = classify_error(
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "target_disconnected".to_string(),
                message: "worker disconnected while awaiting response".to_string(),
                retryable: true,
            },
            true,
        );
        assert!(matches!(
            response_timeout,
            RemotePromptSubmissionOutcome::Indeterminate(_)
        ));

        let partial_write = classify_error(
            DaemonError::LocalTransport {
                operation: "write temporary relay peer request",
                message: "connection closed during websocket write".to_string(),
            },
            true,
        );
        assert!(matches!(
            partial_write,
            RemotePromptSubmissionOutcome::Indeterminate(_)
        ));

        let setup_failure = classify_error(
            DaemonError::LocalTransport {
                operation: "prepare remote prompt manifest",
                message: "local validation failed before network submission".to_string(),
            },
            false,
        );
        assert!(matches!(
            setup_failure,
            RemotePromptSubmissionOutcome::DefinitelyPreSend(_)
        ));
    }

    #[test]
    fn remote_prompt_dispatch_does_not_refresh_binding_after_worker_timeout() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "remote prompt dispatch timed out waiting for worker response".to_string(),
        });

        assert!(!remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_refreshes_binding_for_missing_lease_errors() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "leased_agent_not_found".to_string(),
        });

        assert!(remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_refreshes_binding_for_structured_missing_lease_errors() {
        let result = Err(DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "leased_agent_not_found".to_string(),
            message: "the leased agent was not found".to_string(),
            retryable: false,
        });

        assert!(remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_retries_with_a_credential_only_when_worker_requests_it() {
        let required = Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: format!(
                "transport error: {}: worker run was lost",
                crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE,
            ),
        });
        assert!(remote_prompt_dispatch_requires_provider_launch_credential(
            &required
        ));

        let unrelated = Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "worker run was lost".to_string(),
        });
        assert!(!remote_prompt_dispatch_requires_provider_launch_credential(
            &unrelated
        ));

        let structured = Err(DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
                .to_string(),
            message: "worker requires a launch credential".to_string(),
            retryable: false,
        });
        assert!(remote_prompt_dispatch_requires_provider_launch_credential(
            &structured
        ));
    }

    #[test]
    fn remote_prompt_dispatch_retries_disconnected_relay_targets() {
        let error = DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "target daemon is not connected to relay".to_string(),
        };

        assert!(remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_dispatch_retries_relay_reported_disconnected_targets() {
        let error = DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "target daemon disconnected from relay".to_string(),
        };

        assert!(remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_dispatch_retries_temporary_relay_responses() {
        let error = DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message: "target daemon disconnected from relay".to_string(),
        };

        assert!(remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_dispatch_retries_structured_temporary_relay_responses() {
        let error = DaemonError::RelayTransport {
            operation: "read temporary relay peer response",
            code: "target_disconnected".to_string(),
            message: "target daemon disconnected from relay".to_string(),
            retryable: true,
        };

        assert!(remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_dispatch_does_not_retry_relay_authorization_failures() {
        let error = DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "invalid relay token".to_string(),
        };

        assert!(!remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_dispatch_does_not_retry_structured_relay_authorization_failures() {
        let error = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "invalid_relay_token".to_string(),
            message: "invalid relay token".to_string(),
            retryable: false,
        };

        assert!(!remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_transport_retry_window_is_bounded() {
        assert!(!remote_prompt_transport_retry_window_expired(
            REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW - std::time::Duration::from_millis(1),
        ));
        assert!(remote_prompt_transport_retry_window_expired(
            REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW,
        ));
    }

    #[test]
    fn remote_prompt_dispatch_does_not_retry_stopped_slice_forever() {
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Stopped,
        ));
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Stopping,
        ));
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Unhealthy,
        ));
        assert!(remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Starting,
        ));
        assert!(remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Running,
        ));
    }

    #[test]
    fn leased_prompt_submit_timeout_covers_codex_mcp_retry_window() {
        assert!(
            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT
                > std::time::Duration::from_secs(180)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn remote_workflow_submit_rejects_missing_vault_credential_before_transport() {
        let _env = crate::env_lock::lock();
        let mut fixture =
            WorkflowSubmissionFixture::new(WorkflowCredentialState::MissingCredential);
        assert!(fixture.dispatch.workflow_context.is_some());
        let error = fixture
            .submit()
            .await
            .expect_err("workflow submit must reject a missing credential before transport");
        assert!(error.to_string().contains("remote Claude launch requires"));
        assert!(!error.to_string().contains("relay_url is not configured"));
        assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn remote_workflow_submit_rejects_missing_or_locked_vault_before_transport() {
        let _env = crate::env_lock::lock();
        for credential_state in [
            WorkflowCredentialState::MissingVault,
            WorkflowCredentialState::Locked,
        ] {
            let fixture = WorkflowSubmissionFixture::new(credential_state);
            assert!(fixture.dispatch.workflow_context.is_some());
            let error = fixture
                .submit_after_rejecting_vault_unlock()
                .await
                .expect_err("rejected vault unlock must stop before relay transport");
            assert!(
                crate::secret::is_chariox_vault_locked_error(&error)
                    || error.to_string().contains("vault unlock was cancelled")
                    || error.to_string().contains("remote Claude launch requires"),
                "missing or locked vault should reject before sending: {error}"
            );
            assert!(!error.to_string().contains("relay_url is not configured"));
            assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn remote_workflow_submit_admits_vaulted_token_or_active_worker_run_without_exposing_it()
    {
        let _env = crate::env_lock::lock();
        for credential_state in [
            WorkflowCredentialState::Unlocked,
            WorkflowCredentialState::ActiveWorkerRun,
        ] {
            let mut fixture = WorkflowSubmissionFixture::new(credential_state);
            assert!(fixture.dispatch.workflow_context.is_some());
            let error = fixture
                .submit()
                .await
                .expect_err("fixture relay is absent after credential admission");
            assert!(
                error.to_string().contains("relay_url is not configured"),
                "credential admission should reach the transport boundary: {error}"
            );
            assert!(!error.to_string().contains(WORKFLOW_CREDENTIAL_CANARY));
            assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
        }
    }
}
