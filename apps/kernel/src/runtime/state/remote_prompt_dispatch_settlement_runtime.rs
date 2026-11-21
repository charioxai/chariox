//! Durable remote prompt dispatch settlement and its user-visible projection.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        self.finish_remote_prompt_dispatch_with_retry_hook(dispatch, result, || {})
            .await
    }

    pub(super) async fn finish_remote_prompt_dispatch_with_retry_hook(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
        mut after_failed_ack: impl FnMut() + Send,
    ) -> Result<(), DaemonError> {
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let delivery_succeeded = result.is_ok();
        use super::remote_prompt_owned_state::RemotePromptDispatchSettlement;
        let mut append_retry = 0_u8;
        let settlement = loop {
            match self.owned.settle_remote_dispatch_if_current(
                &dispatch,
                result.as_ref().ok().map(String::as_str),
            ) {
                Err(error)
                    if result.is_ok()
                        && (matches!(&error, DaemonError::SessionHistoryFailed { .. })
                            || crate::durable_state::is_retryable_durable_write_error(&error))
                        && append_retry < 2 =>
                {
                    append_retry += 1;
                    after_failed_ack();
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "worker accepted prompt but durable acknowledgement failed; retrying acknowledgement only",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": append_retry,
                            "error": error.to_string(),
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        50 * u64::from(append_retry),
                    ))
                    .await;
                }
                outcome => break outcome?,
            }
        };
        match (&settlement, &result) {
            (RemotePromptDispatchSettlement::Settled(_), Ok(run)) => {
                self.bind_leased_agent_task(&dispatch, run);
                self.record_leased_event_receipt(&dispatch, Some(run), "accepted")
            }
            (RemotePromptDispatchSettlement::Settled(_), Err(_)) => {
                self.record_leased_event_receipt(&dispatch, None, "rejected")
            }
            (RemotePromptDispatchSettlement::BindingChanged(_), _) => {
                self.record_leased_event_receipt(&dispatch, None, "uncertain")
            }
            (RemotePromptDispatchSettlement::Superseded, _) => {}
        }
        let settled_prompt = match settlement {
            RemotePromptDispatchSettlement::Settled(prompt) => prompt,
            RemotePromptDispatchSettlement::Superseded => return Ok(()),
            RemotePromptDispatchSettlement::BindingChanged(prompt) => {
                let workflow_id = prompt.workflow_run_id().and_then(|run_id| {
                    self.owned
                        .session_store
                        .get_session(&dispatch.session_id)
                        .ok()
                        .and_then(|session| {
                            session
                                .workflow_run(run_id)
                                .map(|run| run.workflow_id().to_string())
                        })
                });
                let message = format!(
                    "Remote delivery of prompt `{}` is uncertain because its worker binding changed before acknowledgement. The prompt remains pending; it was not replayed or cancelled. Check the previous worker's turn before deciding whether to retry.",
                    dispatch.prompt_id,
                );
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    &message,
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "prompt_id": dispatch.prompt_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "delivery_uncertain": true,
                        "prompt_replayed": false,
                    }),
                );
                let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                let merge_key = Some(format!("remote-dispatch-uncertain:{}", dispatch.prompt_id));
                self.owned.fan_out_remote_dispatch_error(
                    &dispatch,
                    &provider_run_id,
                    merge_key.clone(),
                    &message,
                );
                self.owned.append_operational_history_entry_with_context(
                    &SessionHistoryEntry::provider_output(
                        &dispatch.session_id,
                        &provider_run_id,
                        Some(&dispatch.agent_id),
                        crate::terminal::TerminalOutputKind::ProviderError,
                        merge_key,
                        message,
                    )
                    .with_prompt_origin(dispatch.prompt_origin)
                    .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                    crate::history::HistoryEventTurnContext {
                        session_id: Some(dispatch.session_id.clone()),
                        agent_id: Some(dispatch.agent_id.clone()),
                        provider_run_id: Some(provider_run_id),
                        prompt_id: Some(dispatch.prompt_id.clone()),
                        turn_id: Some(dispatch.prompt_id.clone()),
                        workflow_id,
                        workflow_run_id: prompt.workflow_run_id().map(str::to_string),
                        workflow_node_id: prompt.workflow_node_run_id().map(str::to_string),
                        ..Default::default()
                    },
                );
                return Ok(());
            }
        };
        self.owned.provider_process_projection.invalidate();
        let echo_to_all_attachments = settled_prompt.durable_initially_queued() == Some(true);
        let should_start_projection_drain = {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    let _ = owned.session_snapshot(&dispatch.session_id)?;
                    if echo_to_all_attachments {
                        let projected_provider_run_id =
                            crate::provider::projected_leased_provider_run_id(
                                &dispatch.leased_agent_id,
                                &remote_provider_run_id,
                            );
                        owned.echo_promoted_queued_prompt_to_attachments(
                            &dispatch.session_id,
                            &projected_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    } else {
                        owned.echo_prompt_to_other_attachments(
                            &dispatch.session_id,
                            &remote_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    }
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                        None,
                    );
                    Ok(true)
                }
                Err(error) => {
                    let failure =
                        remote_dispatch_failure_outcome(&error, &dispatch.worker_kernel_id);
                    let message = failure.reason.clone();
                    let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                    let merge_key = Some(format!("remote-dispatch-error:{}", dispatch.prompt_id));
                    owned.fan_out_remote_dispatch_error(
                        &dispatch,
                        &provider_run_id,
                        merge_key.clone(),
                        &message,
                    );
                    let workflow_id = settled_prompt.workflow_run_id().and_then(|run_id| {
                        owned
                            .session_store
                            .get_session(&dispatch.session_id)
                            .ok()
                            .and_then(|session| {
                                session
                                    .workflow_run(run_id)
                                    .map(|run| run.workflow_id().to_string())
                            })
                    });
                    owned.append_operational_history_entry_with_context(
                        &SessionHistoryEntry::provider_output(
                            &dispatch.session_id,
                            &provider_run_id,
                            Some(&dispatch.agent_id),
                            crate::terminal::TerminalOutputKind::ProviderError,
                            merge_key,
                            message.clone(),
                        )
                        .with_prompt_origin(dispatch.prompt_origin)
                        .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                        crate::history::HistoryEventTurnContext {
                            workflow_id,
                            session_id: Some(dispatch.session_id.clone()),
                            agent_id: Some(dispatch.agent_id.clone()),
                            provider_run_id: Some(provider_run_id.clone()),
                            prompt_id: Some(dispatch.prompt_id.clone()),
                            turn_id: Some(dispatch.prompt_id.clone()),
                            workflow_run_id: settled_prompt.workflow_run_id().map(str::to_string),
                            workflow_node_id: settled_prompt
                                .workflow_node_run_id()
                                .map(str::to_string),
                            ..Default::default()
                        },
                    );
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                        Some(message.clone()),
                    );
                    if settled_prompt.workflow_run_id().is_none()
                        && settled_prompt.status() != crate::session::PromptStatus::Cancelling
                    {
                        owned.block_failed_prompt_task(
                            &dispatch.session_id,
                            &dispatch.agent_id,
                            &dispatch.prompt_id,
                            None,
                            &failure.reason,
                        )?;
                        self.schedule_agent_lifecycle_sweep();
                    }
                    owned.record_failed_prompt_settlement(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &settled_prompt,
                        settled_prompt.durable_delivery_provider_run_id(),
                        failure,
                    );
                    if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                        settled_prompt.workflow_run_id(),
                        settled_prompt.workflow_node_run_id(),
                    ) {
                        owned.release_workflow_node_workspace_claim(
                            &dispatch.session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                        );
                        owned.workflow_fail_node_after_dispatch_error(
                            &dispatch.session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                            &error,
                        );
                    }
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    Err(error)
                }
            }
        }?;
        if should_start_projection_drain
            && settled_prompt.status() != crate::session::PromptStatus::Cancelling
        {
            self.spawn_remote_prompt_projection_drain(session_id.clone(), agent_id.clone());
        }
        if delivery_succeeded && settled_prompt.status() == crate::session::PromptStatus::Cancelling
        {
            if let Err(error) = self
                .continue_remote_agent_prompt_cancellation(
                    &session_id,
                    &agent_id,
                    settled_prompt.source_attachment_id(),
                    settled_prompt.id(),
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "durable remote cancellation intent remains pending after worker ACK",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "prompt_id": settled_prompt.id(),
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "provider_run_id": settled_prompt.durable_delivery_provider_run_id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }
}

/// Why a remote prompt could not start, in words for the person looking at the
/// agent: the card and the turn carry this instead of a bare Error.
pub(super) fn remote_dispatch_failure_outcome(
    error: &DaemonError,
    worker_kernel_id: &str,
) -> crate::provider::TurnOutcome {
    use crate::provider::TurnOutcomeKind;
    use crate::transport::relay_peer::{
        LeasedPromptRejection, REMOTE_PROVIDER_ACCOUNT_MISSING_CODE,
        REMOTE_PROVIDER_ACCOUNT_UNUSABLE_CODE,
    };
    let cause = match error {
        DaemonError::RemotePromptRejected { reason } => *reason,
        DaemonError::RelayTransport { code, .. } => match code.as_str() {
            REMOTE_PROVIDER_ACCOUNT_MISSING_CODE | "provider_profile_unavailable" => {
                LeasedPromptRejection::AccountMissing
            }
            REMOTE_PROVIDER_ACCOUNT_UNUSABLE_CODE
            | "provider_profile_unverified"
            | "provider_profile_unauthenticated"
            | "provider_profile_usage_exhausted" => LeasedPromptRejection::AccountUnusable,
            "vault_locked" | "target_vault_locked" => LeasedPromptRejection::VaultLocked,
            "target_not_connected" => {
                return crate::provider::TurnOutcome::new(
                    TurnOutcomeKind::Network,
                    true,
                    &format!("Worker kernel {worker_kernel_id} is not connected. Reconnect it and retry."),
                );
            }
            _ => LeasedPromptRejection::Other,
        },
        error => LeasedPromptRejection::from_error(error),
    };
    let (kind, reason) = match cause {
        LeasedPromptRejection::AccountMissing => (
            TurnOutcomeKind::SignInRequired,
            format!("Worker kernel {worker_kernel_id} does not hold the selected account. Copy it there in Provider Accounts or choose another account."),
        ),
        LeasedPromptRejection::AccountUnusable => (
            TurnOutcomeKind::SignInRequired,
            format!("The selected provider account is unavailable for use on worker kernel {worker_kernel_id}. Check its sign-in in Provider Accounts or choose another account."),
        ),
        LeasedPromptRejection::ExecutableMissing => (
            TurnOutcomeKind::Other,
            format!("The provider program is not installed on worker kernel {worker_kernel_id}. Install it on that machine."),
        ),
        LeasedPromptRejection::VaultLocked => (
            TurnOutcomeKind::Other,
            format!("The provider could not open the Vault for worker kernel {worker_kernel_id}. Unlock the Vault in Chariox and retry."),
        ),
        LeasedPromptRejection::Other => (
            TurnOutcomeKind::Other,
            format!("Worker kernel {worker_kernel_id} could not start the provider. Check the provider setup on that machine and retry."),
        ),
    };
    crate::provider::TurnOutcome::new(kind, false, &reason)
}
