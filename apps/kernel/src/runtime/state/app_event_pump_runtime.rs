//! App delivery uses the same workflow queue, durable writer and provider dispatch
//! as every terminal. No foreground view or second workflow scheduler is needed.
use super::{KernelRuntimeOwnedState, KernelRuntimeState, WorkflowPromptDispatches};
use crate::durable_state::{
    app_automations::AppAutomationError,
    app_event_delivery::AppEventDeliveryError,
    app_event_maintenance::{
        AppEventClassification, AppEventMaintenanceOperation, AppEventMaintenanceOutcome,
    },
};
use crate::runtime::{
    app_event_pump::{AppCursor, AppEventPass},
    app_operation_budget::AppOperationBudget,
    app_worker::AppWorkerLease,
};
use chariox_app_runtime::app_outbox::{OutboxError, Receipt};
use std::{collections::BTreeSet, sync::Arc};
const PAGE: usize = 8; // at most eight new queue admissions + eight queued recovery sessions

struct PassOutput {
    delivery: Option<AppCursor>,
    maintenance: Option<AppCursor>,
    dispatch: Option<String>,
    more: bool,
    dispatches: WorkflowPromptDispatches,
}
impl KernelRuntimeState {
    /// Executes the actual bounded pass without dispatching a provider process.
    /// A fresh timer reservation keeps fixture retries deterministic; active
    /// worker discovery and AppControl admission use their production paths.
    #[cfg(test)]
    pub(super) fn fixture_app_event_pass(&self) -> WorkflowPromptDispatches {
        let pump = crate::runtime::app_event_pump::AppEventPump::new();
        let pass = pump.try_begin().unwrap();
        let _permit = self.app_control().try_admit().unwrap();
        let leases = self.app_control().active_app_leases(None, PAGE);
        self.owned.pump_app_event_pass(&pass, leases).dispatches
    }

    /// Coordinator wiring only: admission and the one-pass reservation move into
    /// blocking ownership. Cancelling an awaiting task cannot release them early.
    pub(crate) fn schedule_app_event_pump(&self) {
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(pass) = self.app_control().event_pump().try_begin() else {
            return;
        };
        let Ok(permit) = self.app_control().try_admit() else {
            return;
        };
        let leases = self
            .app_control()
            .active_app_leases(pass.delivery_after(), PAGE);
        let runtime = self.clone();
        tokio::spawn(async move {
            let owned = runtime.owned.clone();
            let result = tokio::task::spawn_blocking(move || {
                let output = owned.pump_app_event_pass(&pass, leases);
                (pass, permit, output)
            })
            .await;
            if let Ok((pass, _permit, output)) = result {
                runtime.spawn_workflow_prompt_dispatches(output.dispatches);
                pass.finish(
                    output.delivery,
                    output.maintenance,
                    output.dispatch,
                    output.more,
                );
            }
        });
    }
}
impl KernelRuntimeOwnedState {
    fn pump_app_event_pass(&self, pass: &AppEventPass, leases: Vec<AppWorkerLease>) -> PassOutput {
        // Budgets may outlive their caller in the writer queue. Never capture
        // the store in a request for its own writer; that would retain its join
        // owner on the writer thread. Health is checked here and at admission.
        let budget = AppOperationBudget::from_supervisor(|| false);
        let mut output = PassOutput {
            delivery: None,
            maintenance: None,
            dispatch: pass.dispatch_after().map(str::to_owned),
            more: false,
            dispatches: WorkflowPromptDispatches::default(),
        };
        let mut sessions = BTreeSet::new();
        // Housekeeping remains available after publisher revoke/uninstall. It
        // cannot accept new events or grant an automation; only actual already
        // queued workflow records can produce recovery dispatch candidates.
        if let Ok(page) = self
            .durable_state_store
            .app_event_installations_page(pass.maintenance_after(), PAGE)
        {
            let full = page.len() == PAGE;
            let mut completed = 0;
            for (owner, installation) in &page {
                if budget.check().is_err()
                    || self.durable_state_store.require_writer_healthy().is_err()
                {
                    break;
                }
                let operation = AppEventMaintenanceOperation::Sweep {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    durable_owner: self.config_projection.snapshot().daemon_id,
                };
                if let Ok(AppEventMaintenanceOutcome::Swept {
                    maintenance,
                    queued_sessions,
                }) = self
                    .durable_state_store
                    .maintain_app_events(operation, budget.fork(|| false))
                {
                    sessions.extend(queued_sessions);
                    output.more |= maintenance.pruned == 256
                        || maintenance.expired + maintenance.failed == 256;
                }
                output.maintenance = Some((owner.clone(), installation.clone()));
                completed += 1;
            }
            if !full && completed == page.len() {
                output.maintenance = None;
            } else {
                output.more = true;
            }
        }
        if self.publication_activation.is_active() {
            let owner = self.config_projection.snapshot().daemon_id;
            if let Ok(mut pending) = self.durable_state_store.pending_workflow_dispatch_sessions(
                &owner,
                pass.dispatch_after(),
                PAGE,
            ) {
                if pending.len() < PAGE && pass.dispatch_after().is_some() {
                    if let Ok(wrapped) = self
                        .durable_state_store
                        .pending_workflow_dispatch_sessions(&owner, None, PAGE - pending.len())
                    {
                        pending.extend(wrapped);
                    }
                }
                sessions.extend(pending);
            }
            let full = leases.len() == PAGE;
            let expected = leases.len();
            let mut completed = 0;
            for lease in leases {
                if budget.check().is_err()
                    || self.durable_state_store.require_writer_healthy().is_err()
                {
                    break;
                }
                completed += 1;
                let lease = Arc::new(lease);
                output.delivery = Some((
                    lease.owner().into(),
                    lease.catalog().installation_id().into(),
                ));
                if lease.is_stopped() {
                    continue;
                }
                let now = crate::session::unix_epoch_ms();
                let Ok(mut pending) = self.durable_state_store.pending_app_events(
                    lease.owner(),
                    lease.catalog(),
                    now,
                    2,
                ) else {
                    continue;
                };
                output.more |= pending.len() > 1;
                if pending.is_empty() {
                    continue;
                }
                let receipt = pending.remove(0);
                if now >= receipt.expires_at_ms {
                    self.classify_app_event(
                        &budget,
                        &lease,
                        receipt,
                        AppEventClassification::Expired,
                    );
                    continue;
                }
                let observe = lease.clone();
                match self.queue_app_event(
                    lease.owner(),
                    lease.catalog().clone(),
                    &receipt.receipt_id,
                    budget.fork(move || observe.is_stopped()),
                ) {
                    Ok(queued) => {
                        if let Some(session) = queued.queued_session_id {
                            sessions.insert(session);
                        }
                    }
                    Err(AppEventDeliveryError::CommitUnknown) => {
                        // The sole writer has stopped. Do not construct or
                        // dispatch runs from any un-reconciled in-memory queue.
                        output.more = false;
                        return output;
                    }
                    Err(error) => {
                        if let Some(classification) = classify_failure(&error) {
                            self.classify_app_event(&budget, &lease, receipt, classification);
                        }
                    }
                }
            }
            if !full && completed == expected {
                output.delivery = None;
            } else {
                output.more = true;
            }
            let dispatches = dispatch_order(sessions, pass.dispatch_after());
            output.more |= dispatches.len() > PAGE;
            for session in dispatches.into_iter().take(PAGE) {
                if budget.check().is_err()
                    || self.durable_state_store.require_writer_healthy().is_err()
                {
                    break;
                }
                output.dispatch = Some(session.clone());
                // Common workflow admission persists queue -> Ready + entry
                // intent before scheduling, and its prompt path owns recovery.
                match self.workflow_start_next_queued_prompt_for_response(&session) {
                    Ok((_, dispatches)) => output.dispatches.extend(dispatches),
                    Err(_) => crate::logging::warn_with_fields(
                        "daemon.app_events",
                        "durable App event remains queued for normal workflow recovery",
                        serde_json::json!({"session_id":session}),
                    ),
                }
            }
        }
        output
    }
    fn classify_app_event(
        &self,
        budget: &AppOperationBudget,
        lease: &Arc<AppWorkerLease>,
        receipt: Receipt,
        classification: AppEventClassification,
    ) {
        let observe = lease.clone();
        let _ = self.durable_state_store.maintain_app_events(
            AppEventMaintenanceOperation::Classify {
                owner: lease.owner().into(),
                catalog: lease.catalog().clone(),
                receipt,
                classification,
            },
            budget.fork(move || observe.is_stopped()),
        );
    }
}
fn dispatch_order(sessions: BTreeSet<String>, after: Option<&str>) -> Vec<String> {
    let after = after.unwrap_or("");
    sessions
        .iter()
        .filter(|value| value.as_str() > after)
        .chain(sessions.iter().filter(|value| value.as_str() <= after))
        .cloned()
        .collect()
}
fn classify_failure(error: &AppEventDeliveryError) -> Option<AppEventClassification> {
    use AppEventClassification::{Failed, Retryable};
    match error {
        AppEventDeliveryError::Limit => Some(Retryable),
        AppEventDeliveryError::AutomationChanged => Some(Failed),
        AppEventDeliveryError::Outbox(OutboxError::Database(rusqlite::Error::SqliteFailure(
            code,
            _,
        ))) if matches!(
            code.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ) =>
        {
            Some(Retryable)
        }
        AppEventDeliveryError::Outbox(
            OutboxError::Invalid | OutboxError::Schema | OutboxError::Corrupt | OutboxError::TooOld,
        ) => Some(Failed),
        AppEventDeliveryError::Target(
            AppAutomationError::InvalidTarget
            | AppAutomationError::NotOwner
            | AppAutomationError::TargetChanged,
        ) => Some(Failed),
        // Lifecycle cancellation, current-generation/signer fencing, paused
        // bindings, concurrent revision changes and storage uncertainty do not
        // spend retry attempts or assert permanent domain failure.
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_cursor_rotates_past_slow_or_previously_budgeted_sessions() {
        let sessions = BTreeSet::from(["a".into(), "b".into(), "c".into()]);
        assert_eq!(
            dispatch_order(sessions.clone(), Some("a")),
            vec!["b", "c", "a"]
        );
        assert_eq!(dispatch_order(sessions, Some("c")), vec!["a", "b", "c"]);
    }
    #[test]
    fn only_explicit_transient_failures_consume_retry_attempts() {
        assert!(matches!(
            classify_failure(&AppEventDeliveryError::Limit),
            Some(AppEventClassification::Retryable)
        ));
        assert!(matches!(
            classify_failure(&AppEventDeliveryError::AutomationChanged),
            Some(AppEventClassification::Failed)
        ));
        assert!(classify_failure(&AppEventDeliveryError::CommitUnknown).is_none());
        assert!(classify_failure(&AppEventDeliveryError::Conflict).is_none());
        assert!(classify_failure(&AppEventDeliveryError::Outbox(OutboxError::Inactive)).is_none());
    }
}
