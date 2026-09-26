//! Exact worker-receipt validation, reconciliation, and durable settlement decisions.

use super::remote_prompt_projection_drain_runtime::same_remote_prompt_worker_binding;
use super::remote_prompt_worker_submission_runtime::{
    persist_remote_prompt_reconciliation_pending, query_remote_prompt_worker_receipt,
    remote_prompt_reconciliation_pending_error,
};
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemotePromptReceiptAction {
    AssociateActiveRun(String),
    DrainCompletedProjection(String),
    RejectUnadmitted,
}

pub(super) fn remote_prompt_receipt_action(
    expected_prompt_id: &str,
    expected_execution_lease_id: &str,
    receipt: Option<crate::transport::relay_peer::LeasedPromptReceipt>,
) -> Result<RemotePromptReceiptAction, &'static str> {
    let Some(receipt) = receipt else {
        return Err("worker returned no receipt for the exact home prompt");
    };
    if receipt.home_prompt_id != expected_prompt_id {
        return Err("worker receipt names a different home prompt");
    }
    if matches!(
        receipt.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
            | crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
    ) && (expected_execution_lease_id.trim().is_empty()
        || receipt.execution_lease_id.as_deref() != Some(expected_execution_lease_id))
    {
        return Err("worker receipt has a different execution lease");
    }
    if matches!(
        receipt.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
            | crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
    ) && receipt.target_home_prompt_id.is_some()
    {
        return Err("worker receipt names a queued steer while reconciling an active prompt");
    }
    match receipt.phase {
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
        | crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
            if receipt.worker_provider_run_id.trim().is_empty() =>
        {
            Err("worker receipt has no provider-run identity")
        }
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active => Ok(
            RemotePromptReceiptAction::AssociateActiveRun(receipt.worker_provider_run_id),
        ),
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed => Ok(
            RemotePromptReceiptAction::DrainCompletedProjection(receipt.worker_provider_run_id),
        ),
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected
            if !expected_execution_lease_id.trim().is_empty()
                && receipt.target_home_prompt_id.is_none()
                && receipt.worker_provider_run_id.is_empty()
                && receipt.execution_lease_id.as_deref() == Some(expected_execution_lease_id) =>
        {
            Ok(RemotePromptReceiptAction::RejectUnadmitted)
        }
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerDispatching
        | crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted
        | crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected => {
            Err("worker returned a queued-steer receipt while reconciling an active prompt")
        }
    }
}

pub(super) fn remote_prompt_receipt_action_requires_projection(
    action: &RemotePromptReceiptAction,
) -> bool {
    matches!(
        action,
        RemotePromptReceiptAction::DrainCompletedProjection(_)
    )
}

pub(super) fn remote_prompt_receipt_is_rejected(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport {
            operation: "remote prompt worker rejected admission",
            ..
        }
    )
}

impl KernelRuntimeState {
    pub(super) async fn reconcile_remote_prompt_worker_receipt(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<String, DaemonError> {
        match self.remote_prompt_receipt_prompt_is_current(dispatch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(DaemonError::NoActivePrompt {
                    session_id: dispatch.session_id.clone(),
                });
            }
            Err(error) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(
                    dispatch,
                    format!("could not verify the active home prompt before querying: {error}"),
                ));
            }
        }
        if let Err(error) = persist_remote_prompt_reconciliation_pending(self, dispatch, true) {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                format!("could not persist reconciliation marker before querying worker: {error}"),
            ));
        }
        let binding_before = match self.remote_prompt_receipt_binding(dispatch) {
            Ok(binding) => binding,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        let receipt = match query_remote_prompt_worker_receipt(self, dispatch).await {
            Ok(receipt) => receipt,
            Err(error) => {
                let detail = format!("read-only worker receipt query failed: {error}");
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail));
            }
        };
        match self.remote_prompt_receipt_prompt_is_current(dispatch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(DaemonError::NoActivePrompt {
                    session_id: dispatch.session_id.clone(),
                });
            }
            Err(error) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(
                    dispatch,
                    format!("could not verify the active home prompt after querying: {error}"),
                ));
            }
        }
        let action = match remote_prompt_receipt_action(
            &dispatch.prompt_id,
            &binding_before.execution_lease_id,
            receipt,
        ) {
            Ok(action) => action,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        let binding_after = match self.remote_prompt_receipt_binding(dispatch) {
            Ok(binding) => binding,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        if !same_remote_prompt_worker_binding(&binding_before, &binding_after)
            || binding_before.active_worker_provider_run_id
                != binding_after.active_worker_provider_run_id
        {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                "remote worker binding changed while its receipt was being queried",
            ));
        }
        if action == RemotePromptReceiptAction::RejectUnadmitted {
            return Err(DaemonError::LocalTransport {
                operation: "remote prompt worker rejected admission",
                message: format!(
                    "worker durably rejected exact prompt {} under execution lease {}",
                    dispatch.prompt_id, binding_before.execution_lease_id
                ),
            });
        }
        let completed = remote_prompt_receipt_action_requires_projection(&action);
        let worker_provider_run_id = match action {
            RemotePromptReceiptAction::AssociateActiveRun(provider_run_id) => provider_run_id,
            RemotePromptReceiptAction::DrainCompletedProjection(provider_run_id) => provider_run_id,
            RemotePromptReceiptAction::RejectUnadmitted => unreachable!(),
        };
        if let Err(error) = self
            .owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                &dispatch.agent_id,
                Some(worker_provider_run_id.clone()),
            )
        {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                format!("could not associate the verified worker provider run: {error}"),
            ));
        }
        if let Err(error) = self.owned.session_snapshot(&dispatch.session_id) {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                format!("could not persist the verified worker provider run: {error}"),
            ));
        }

        if completed {
            if let Err(detail) = self
                .drain_completed_remote_prompt_receipt_projection(dispatch, &worker_provider_run_id)
                .await
            {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail));
            }
        }
        Ok(worker_provider_run_id)
    }

    pub(super) async fn resume_delivered_remote_prompt_cancellation_after_restart(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        expected_provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let expected_provider_run_id =
            expected_provider_run_id.ok_or_else(|| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "delivered cancellation has no durable worker provider-run identity"
                    .to_string(),
            })?;
        if !self.remote_prompt_receipt_prompt_is_current(dispatch)? {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let binding_before = self
            .remote_prompt_receipt_binding(dispatch)
            .map_err(|detail| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: detail,
            })?;
        if binding_before.active_worker_provider_run_id.as_deref() != Some(expected_provider_run_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "the current worker binding no longer owns the delivered prompt run"
                    .to_string(),
            });
        }
        let receipt = query_remote_prompt_worker_receipt(self, dispatch).await?;
        if !self.remote_prompt_receipt_prompt_is_current(dispatch)? {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let action = remote_prompt_receipt_action(
            &dispatch.prompt_id,
            &binding_before.execution_lease_id,
            receipt,
        )
        .map_err(|detail| DaemonError::LocalTransport {
            operation: "resume remote prompt cancellation",
            message: detail.to_string(),
        })?;
        let binding_after = self
            .remote_prompt_receipt_binding(dispatch)
            .map_err(|detail| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: detail,
            })?;
        if !same_remote_prompt_worker_binding(&binding_before, &binding_after)
            || binding_after.active_worker_provider_run_id.as_deref()
                != Some(expected_provider_run_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "the verified worker binding changed while its receipt was queried"
                    .to_string(),
            });
        }
        match action {
            RemotePromptReceiptAction::AssociateActiveRun(provider_run_id)
                if provider_run_id.as_str() == expected_provider_run_id =>
            {
                self.cancel_remote_agent_prompt_if_remote(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.source_attachment_id,
                )
                .await?;
                Ok(())
            }
            RemotePromptReceiptAction::DrainCompletedProjection(provider_run_id)
                if provider_run_id.as_str() == expected_provider_run_id =>
            {
                self.drain_completed_remote_prompt_receipt_projection(
                    dispatch,
                    expected_provider_run_id,
                )
                .await
                .map_err(|detail| DaemonError::LocalTransport {
                    operation: "resume remote prompt cancellation",
                    message: detail,
                })?;
                let binding = self
                    .remote_prompt_receipt_binding(dispatch)
                    .map_err(|detail| DaemonError::LocalTransport {
                        operation: "resume remote prompt cancellation",
                        message: detail,
                    })?;
                let session = self.owned.session_store.get_session(&dispatch.session_id)?;
                if let Some(prompt) = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &dispatch.agent_id)
                    .filter(|prompt| {
                        prompt.id() == dispatch.prompt_id
                            && prompt.status() == crate::session::PromptStatus::Cancelling
                            && prompt.durable_delivery_phase()
                                == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                            && prompt.durable_delivery_provider_run_id()
                                == Some(expected_provider_run_id)
                            && binding.active_worker_provider_run_id.as_deref()
                                == Some(expected_provider_run_id)
                    })
                {
                    self.finalize_remote_prompt_cancellation_and_advance(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        prompt.source_attachment_id(),
                    )?;
                }
                Ok(())
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "worker receipt did not prove the exact delivered provider run"
                    .to_string(),
            }),
        }
    }

    pub(super) fn remote_prompt_receipt_prompt_is_current(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .is_some_and(|prompt| prompt.id() == dispatch.prompt_id))
    }

    pub(super) fn remote_prompt_receipt_binding(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<crate::agent::RemoteAgentBinding, String> {
        let agent = self
            .owned
            .agent_store
            .get_agent(&dispatch.agent_id)
            .map_err(|error| format!("could not read current remote worker binding: {error}"))?;
        agent
            .remote_execution()
            .filter(|binding| {
                binding.worker_kernel_id == dispatch.worker_kernel_id
                    && binding.leased_agent_id == dispatch.leased_agent_id
            })
            .cloned()
            .ok_or_else(|| {
                "current remote worker binding conflicts with the uncertain dispatch".to_string()
            })
    }

    fn hold_remote_prompt_receipt_reconciliation(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        detail: impl std::fmt::Display,
    ) -> DaemonError {
        let detail = detail.to_string();
        let detail = match persist_remote_prompt_reconciliation_pending(self, dispatch, true) {
            Ok(()) => detail,
            Err(error) => format!(
                "{detail}; durable reconciliation marker could not be refreshed ({error}); the Dispatching phase continues to block replay"
            ),
        };
        self.report_remote_prompt_reconciliation_pending(dispatch, &detail);
        remote_prompt_reconciliation_pending_error(dispatch, &detail)
    }

    pub(super) fn report_remote_prompt_reconciliation_pending(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        detail: &str,
    ) {
        let message = format!(
            "Remote delivery of prompt `{}` to worker kernel `{}` is indeterminate: {detail}. The same prompt remains active in durable Dispatching state; it was not cancelled, promoted, or replayed. Automatic replay is blocked until the exact worker receipt is reconciled.",
            dispatch.prompt_id, dispatch.worker_kernel_id
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
                "worker_prompt_receipt_required": true,
            }),
        );
        let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
        let merge_key = Some(format!("remote-dispatch-uncertain:{}", dispatch.prompt_id));
        self.owned.fan_out_remote_dispatch_error(
            dispatch,
            &provider_run_id,
            merge_key.clone(),
            &message,
        );
        let workflow_run_id = self
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &dispatch.agent_id)
                    .filter(|prompt| prompt.id() == dispatch.prompt_id)
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
            });
        let workflow_id = workflow_run_id.as_deref().and_then(|run_id| {
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
                workflow_run_id,
                ..Default::default()
            },
        );
    }

    pub(super) async fn settle_verified_remote_prompt_rejection(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        rejection: DaemonError,
    ) -> Result<(), DaemonError> {
        let result = self
            .finish_remote_prompt_dispatch(dispatch.clone(), Err(rejection))
            .await;
        // The ordinary failure finalizer returns the original provider error even after
        // durably settling it. Only retry when this exact prompt remains active.
        if !self.remote_prompt_receipt_prompt_is_current(dispatch)? {
            return Ok(());
        }
        Err(result.err().unwrap_or_else(|| DaemonError::LocalTransport {
            operation: "settle remote prompt worker rejection",
            message: "verified worker rejection left the exact home prompt active".to_string(),
        }))
    }
}
