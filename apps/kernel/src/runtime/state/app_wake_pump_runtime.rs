//! Kernel-owned App wakes: deliver due wakes to live workers and start stopped
//! workers on demand. The App needs no resident process to wait for a due time.
use super::KernelRuntimeState;
use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
use chariox_app_runtime::managed_state::DueWake;
use std::time::Duration;

const PAGE: usize = 8;
/// A wake delivered this long after its due time is reported as overdue.
const OVERDUE_AFTER_MS: u64 = 60_000;
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);

impl KernelRuntimeState {
    /// Coordinator wiring only: one bounded pass per reservation.
    pub(crate) fn schedule_app_wake_pump(&self) {
        let now_ms = crate::session::unix_epoch_ms();
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(pass) = self.app_control().wake_pump().try_begin(now_ms) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let _pass = pass;
            runtime.app_wake_pass(now_ms).await;
        });
    }

    async fn app_wake_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let due = tokio::task::spawn_blocking(move || {
            store.app_wakes(AppWakeOperation::Due {
                now_ms,
                limit: PAGE,
            })
        })
        .await;
        let Ok(Ok(AppWakeOutcome::Due(due))) = due else {
            return;
        };
        for wake in due {
            let outcome = self.deliver_app_wake(&wake, now_ms).await;
            let Some(operation) = outcome else {
                continue;
            };
            let store = self.owned.durable_state_store.clone();
            let _ = tokio::task::spawn_blocking(move || store.app_wakes(operation)).await;
        }
    }

    /// `None` means delivery is pending an on-demand start; the wake stays due
    /// without consuming a delivery attempt.
    async fn deliver_app_wake(&self, wake: &DueWake, now_ms: u64) -> Option<AppWakeOperation> {
        let control = self.app_control();
        match control.active_app_lease(&wake.owner_id, &wake.installation_id) {
            Some(lease) => {
                let overdue = now_ms.saturating_sub(wake.wake.due_at_ms) > OVERDUE_AFTER_MS;
                Some(
                    match lease.deliver_wake(&wake.wake, overdue, DELIVERY_TIMEOUT).await {
                        Ok(()) => AppWakeOperation::Delivered(wake.clone()),
                        Err(_) => AppWakeOperation::Failed {
                            wake: wake.clone(),
                            now_ms,
                        },
                    },
                )
            }
            None => {
                let lifecycle = control.lifecycle().clone();
                let (owner, installation) = (wake.owner_id.clone(), wake.installation_id.clone());
                let handle = tokio::runtime::Handle::current();
                let started = tokio::task::spawn_blocking(move || {
                    lifecycle.start_active_blocking(&owner, &installation, handle)
                })
                .await;
                match started {
                    Ok(Ok(_)) => None,
                    // Busy admission retries on a later pass; other failures
                    // (inactive, revoked, stopped) consume a bounded attempt.
                    Ok(Err(crate::runtime::app_lifecycle::LifecycleError::Busy)) => None,
                    _ => Some(AppWakeOperation::Failed {
                        wake: wake.clone(),
                        now_ms,
                    }),
                }
            }
        }
    }
}
