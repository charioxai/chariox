//! MP-08 / MP-10: provider readiness is independent of broad maintenance.
use super::*;
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::collections::BTreeSet;

const MAX_PROVIDER_OUTPUT_PUMPS: usize = 64;

impl KernelRuntimeState {
    pub(super) fn take_ready_provider_output_run_ids(&self, now_ms: u64) -> BTreeSet<String> {
        let mut ready = self.owned.pty_output_signal.take_ready_provider_run_ids();
        ready.extend(
            self.owned
                .provider_store
                .run_actor_completion_signal()
                .take_ready_provider_run_ids(),
        );
        ready.extend(
            self.owned
                .structured_output_records
                .take_due_provider_run_ids(now_ms),
        );
        ready.extend(
            self.owned
                .provider_output_deadlines
                .take_due_provider_run_ids(now_ms),
        );
        for run_id in self
            .owned
            .provider_store
            .output_ready_signal()
            .take_ready_provider_run_ids()
        {
            self.owned
                .structured_output_records
                .mark_output_ready(run_id.clone());
            ready.insert(run_id);
        }
        ready
    }

    pub(super) async fn pump_ready_provider_output_run(&self, run_id: &str) {
        let Ok(run) = self.owned.provider_store.get_run(run_id) else {
            self.owned.structured_output_records.clear(run_id);
            return;
        };
        let session_id = run.session_id().to_string();
        // All client and background callers retain the same per-run owner.
        let _permit = self.provider_runtime_lanes.acquire(run_id).await;
        let recipients = self
            .owned
            .attachment_store
            .list_session_attachment_ids(&session_id);
        if let Err(error) = self
            .pump_owned_provider_output(&session_id, run_id, recipients, false)
            .await
        {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "ready provider output pump failed",
                serde_json::json!({"session_id": session_id, "provider_run_id": run_id, "error": error.to_string()}),
            );
        }
        self.observe_git_after_provider_activity_if_pending(run_id)
            .await;
    }

    pub(crate) async fn run_provider_output_pump(&self) {
        self.run_provider_output_pump_until(std::future::pending())
            .await;
    }

    /// Runs until `stop`, then finishes the polls in flight and applies their
    /// results: output a poll took from a provider reaches history before the
    /// kernel stops.
    pub(crate) async fn run_provider_output_pump_until(
        &self,
        stop: impl std::future::Future<Output = ()>,
    ) {
        tokio::pin!(stop);
        let mut jobs = FuturesUnordered::new();
        let mut in_flight = BTreeSet::new();
        let mut pending = BTreeSet::new();
        loop {
            // Capture before taking readiness, including events arriving while
            // another run settles a turn or waits for a process operation.
            let ready_sequence = self.provider_output_ready_sequence();
            let actor_sequence = self.provider_run_actor_completion_sequence();
            let pty_sequence = self.pty_output_change_sequence();
            let change_sequence = self.transport_runtime_pump_change_sequence();
            if self.owned.publication_activation.is_active() {
                self.reap_structured_prompt_jobs_and_dispatch();
                pending.extend(
                    self.take_ready_provider_output_run_ids(crate::session::unix_epoch_ms()),
                );
            }
            while self.owned.publication_activation.is_active()
                && jobs.len() < MAX_PROVIDER_OUTPUT_PUMPS
            {
                let Some(run_id) = pending.iter().find(|id| !in_flight.contains(*id)).cloned()
                else {
                    break;
                };
                pending.remove(&run_id);
                in_flight.insert(run_id.clone());
                let runtime = self.clone();
                jobs.push(async move {
                    runtime.pump_ready_provider_output_run(&run_id).await;
                    run_id
                });
            }
            let now = crate::session::unix_epoch_ms();
            let delay = if self.owned.publication_activation.is_active() {
                [
                    self.owned.structured_output_records.next_poll_due_at_ms(),
                    self.owned.provider_output_deadlines.next_due_at_ms(),
                    self.owned
                        .provider_store
                        .output_ready_signal()
                        .next_due_at_ms(),
                ]
                .into_iter()
                .flatten()
                .min()
                .map(|due| due.saturating_sub(now).min(500))
                .unwrap_or(500)
            } else {
                500
            };
            let has_jobs = !jobs.is_empty();
            tokio::select! {
                () = &mut stop => {
                    while jobs.next().await.is_some() {}
                    // Apply the polls those passes started.
                    for run in self.owned.provider_store.list_runs() {
                        self.pump_ready_provider_output_run(run.id()).await;
                    }
                    return;
                }
                Some(run_id) = jobs.next(), if has_jobs => { in_flight.remove(&run_id); }
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {}
                _ = self.wait_for_provider_output_ready_after(ready_sequence) => {}
                _ = self.wait_for_provider_run_actor_completion_after(actor_sequence) => {}
                _ = self.wait_for_pty_output_change_after(pty_sequence) => {}
                _ = self.wait_for_transport_runtime_pump_change_after(change_sequence) => {}
            }
        }
    }
}
