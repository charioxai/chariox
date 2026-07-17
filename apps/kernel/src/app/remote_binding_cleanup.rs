use std::collections::BTreeMap;

use arroba_relay::protocol::ClientTarget;
use serde::{Deserialize, Serialize};

use crate::agent::RemoteAgentBinding;
use crate::config::DaemonConfig;
use crate::durable_state::DurableStateEvent;
use crate::error::DaemonError;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::DaemonApp;

const CLEANUP_REQUESTED_EVENT_KIND: &str = "agent.remote_binding_cleanup_requested";
const CLEANUP_ATTEMPTED_EVENT_KIND: &str = "agent.remote_binding_cleanup_attempted";
const CLEANUP_COMPLETED_EVENT_KIND: &str = "agent.remote_binding_cleanup_completed";
pub(crate) const REMOTE_BINDING_REFRESHED_EVENT_KIND: &str = "agent.remote_binding_refreshed";
const CLEANUP_RETRY_BASE_MS: u64 = 1_000;
const CLEANUP_RETRY_MAX_MS: u64 = 30_000;
const CLEANUP_RELAY_REQUEST_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteBindingCleanupIntent {
    pub(crate) intent_id: String,
    pub(crate) agent_id: String,
    pub(crate) worker_kernel_id: String,
    pub(crate) worker_machine_id: String,
    pub(crate) execution_lease_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) leased_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_token: Option<String>,
    requested_at_ms: u64,
}

impl RemoteBindingCleanupIntent {
    fn new(
        agent_id: &str,
        worker_kernel_id: &str,
        worker_machine_id: &str,
        execution_lease_id: &str,
        leased_agent_id: Option<&str>,
        relay_config: &DaemonConfig,
    ) -> Self {
        Self {
            intent_id: cleanup_intent_id(worker_kernel_id, execution_lease_id),
            agent_id: agent_id.to_string(),
            worker_kernel_id: worker_kernel_id.to_string(),
            worker_machine_id: worker_machine_id.to_string(),
            execution_lease_id: execution_lease_id.to_string(),
            leased_agent_id: leased_agent_id.map(str::to_string),
            relay_url: relay_config.relay_url.clone(),
            relay_token: relay_config.relay_token.clone(),
            requested_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    fn relay_config(&self, home_config: &DaemonConfig) -> DaemonConfig {
        let mut config = home_config.clone();
        if let (Some(relay_url), Some(relay_token)) =
            (self.relay_url.clone(), self.relay_token.clone())
        {
            config.apply_remote_relay_override(relay_url, relay_token);
        }
        config.relay_request_timeout_ms = config
            .relay_request_timeout_ms
            .min(CLEANUP_RELAY_REQUEST_TIMEOUT_MS);
        config
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRemoteBindingCleanup {
    intent: RemoteBindingCleanupIntent,
    attempt_count: u32,
    next_attempt_at_ms: u64,
    last_error: Option<String>,
}

impl PendingRemoteBindingCleanup {
    fn new(intent: RemoteBindingCleanupIntent) -> Self {
        let next_attempt_at_ms = intent.requested_at_ms;
        Self {
            intent,
            attempt_count: 0,
            next_attempt_at_ms,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteBindingCleanupDisposition {
    Completed,
    Pending,
}

#[derive(Debug, Default)]
pub(crate) struct RemoteBindingCleanupQueue {
    pending: BTreeMap<String, PendingRemoteBindingCleanup>,
}

impl RemoteBindingCleanupQueue {
    fn restore(&mut self, events: Vec<DurableStateEvent>) {
        self.pending.clear();
        for event in events {
            match event.kind.as_str() {
                CLEANUP_REQUESTED_EVENT_KIND => {
                    let Ok(intent) = serde_json::from_value::<RemoteBindingCleanupIntent>(
                        event.payload.get("intent").cloned().unwrap_or_default(),
                    ) else {
                        crate::logging::warn_with_fields(
                            "remote_agent_binding.cleanup",
                            "ignored malformed durable cleanup intent",
                            serde_json::json!({
                                "event_id": event.event_id,
                                "sequence": event.sequence,
                            }),
                        );
                        continue;
                    };
                    self.pending.insert(
                        intent.intent_id.clone(),
                        PendingRemoteBindingCleanup::new(intent),
                    );
                }
                REMOTE_BINDING_REFRESHED_EVENT_KIND => {
                    let Ok(intent) = serde_json::from_value::<RemoteBindingCleanupIntent>(
                        event
                            .payload
                            .get("cleanup_intent")
                            .cloned()
                            .unwrap_or_default(),
                    ) else {
                        continue;
                    };
                    self.pending.insert(
                        intent.intent_id.clone(),
                        PendingRemoteBindingCleanup::new(intent),
                    );
                }
                CLEANUP_ATTEMPTED_EVENT_KIND => {
                    let Some(intent_id) = event.payload.get("intent_id").and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    let Some(pending) = self.pending.get_mut(intent_id) else {
                        continue;
                    };
                    pending.attempt_count = event
                        .payload
                        .get("attempt_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or_default()
                        .min(u32::MAX as u64) as u32;
                    pending.next_attempt_at_ms = event
                        .payload
                        .get("next_attempt_at_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(pending.intent.requested_at_ms);
                    pending.last_error = event
                        .payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                }
                CLEANUP_COMPLETED_EVENT_KIND => {
                    if let Some(intent_id) = event.payload.get("intent_id").and_then(|v| v.as_str())
                    {
                        self.pending.remove(intent_id);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert(&mut self, intent: RemoteBindingCleanupIntent) {
        self.pending
            .entry(intent.intent_id.clone())
            .or_insert_with(|| PendingRemoteBindingCleanup::new(intent));
    }

    fn due_intent_id(&self, now_ms: u64) -> Option<String> {
        self.pending
            .values()
            .filter(|pending| pending.next_attempt_at_ms <= now_ms)
            .min_by_key(|pending| (pending.next_attempt_at_ms, &pending.intent.intent_id))
            .map(|pending| pending.intent.intent_id.clone())
    }

    fn get(&self, intent_id: &str) -> Option<&PendingRemoteBindingCleanup> {
        self.pending.get(intent_id)
    }

    fn record_failure(
        &mut self,
        intent_id: &str,
        now_ms: u64,
        error: String,
    ) -> Option<(u32, u64)> {
        let pending = self.pending.get_mut(intent_id)?;
        pending.attempt_count = pending.attempt_count.saturating_add(1);
        pending.next_attempt_at_ms =
            now_ms.saturating_add(cleanup_retry_delay_ms(pending.attempt_count));
        pending.last_error = Some(error);
        Some((pending.attempt_count, pending.next_attempt_at_ms))
    }

    fn remove(&mut self, intent_id: &str) {
        self.pending.remove(intent_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }
}

impl DaemonApp {
    pub(crate) fn remote_binding_cleanup_intent(
        &self,
        agent_id: &str,
        binding: &RemoteAgentBinding,
    ) -> RemoteBindingCleanupIntent {
        let relay_config = self.relay_config_for_remote_execution(binding);
        RemoteBindingCleanupIntent::new(
            agent_id,
            &binding.worker_kernel_id,
            &binding.worker_machine_id,
            &binding.execution_lease_id,
            Some(&binding.leased_agent_id),
            &relay_config,
        )
    }

    pub(crate) fn retire_remote_binding(
        &mut self,
        agent_id: &str,
        binding: &RemoteAgentBinding,
    ) -> Result<RemoteBindingCleanupDisposition, DaemonError> {
        let intent = self.remote_binding_cleanup_intent(agent_id, binding);
        self.enqueue_and_attempt_remote_binding_cleanup(intent)
    }

    pub(crate) fn enqueue_persisted_remote_binding_cleanup(
        &mut self,
        intent: RemoteBindingCleanupIntent,
    ) -> Result<RemoteBindingCleanupDisposition, DaemonError> {
        let intent_id = intent.intent_id.clone();
        self.remote_binding_cleanups.insert(intent);
        self.attempt_remote_binding_cleanup(&intent_id, crate::session::unix_epoch_ms())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn retire_remote_binding_candidate(
        &mut self,
        agent_id: &str,
        worker_kernel_id: &str,
        worker_machine_id: &str,
        execution_lease_id: &str,
        leased_agent_id: Option<&str>,
        relay_config: &DaemonConfig,
    ) -> Result<RemoteBindingCleanupDisposition, DaemonError> {
        let intent = RemoteBindingCleanupIntent::new(
            agent_id,
            worker_kernel_id,
            worker_machine_id,
            execution_lease_id,
            leased_agent_id,
            relay_config,
        );
        self.enqueue_and_attempt_remote_binding_cleanup(intent)
    }

    fn enqueue_and_attempt_remote_binding_cleanup(
        &mut self,
        intent: RemoteBindingCleanupIntent,
    ) -> Result<RemoteBindingCleanupDisposition, DaemonError> {
        let intent_id = intent.intent_id.clone();
        if self.remote_binding_cleanups.get(&intent_id).is_none() {
            self.durable_state.append_event(
                CLEANUP_REQUESTED_EVENT_KIND,
                Some(intent.agent_id.clone()),
                serde_json::json!({ "intent": &intent }),
            )?;
            self.remote_binding_cleanups.insert(intent);
        }
        self.attempt_remote_binding_cleanup(&intent_id, crate::session::unix_epoch_ms())
    }

    pub(crate) fn retry_due_remote_binding_cleanup(
        &mut self,
        now_ms: u64,
    ) -> Result<Option<RemoteBindingCleanupDisposition>, DaemonError> {
        let Some(intent_id) = self.remote_binding_cleanups.due_intent_id(now_ms) else {
            return Ok(None);
        };
        self.attempt_remote_binding_cleanup(&intent_id, now_ms)
            .map(Some)
    }

    pub(super) fn restore_remote_binding_cleanup_intents(&mut self) -> Result<(), DaemonError> {
        // Cleanup events are scanned from the beginning independently of the latest snapshot.
        // They intentionally remain a small durable job log instead of becoming agent state, so
        // a retired binding can never overwrite the current agent snapshot during recovery.
        let mut events = Vec::new();
        for kind in [
            CLEANUP_REQUESTED_EVENT_KIND,
            CLEANUP_ATTEMPTED_EVENT_KIND,
            CLEANUP_COMPLETED_EVENT_KIND,
            REMOTE_BINDING_REFRESHED_EVENT_KIND,
        ] {
            events.extend(self.durable_state.load_events_by_kind(kind)?);
        }
        events.sort_by_key(|event| event.sequence);
        self.remote_binding_cleanups.restore(events);
        Ok(())
    }

    fn attempt_remote_binding_cleanup(
        &mut self,
        intent_id: &str,
        now_ms: u64,
    ) -> Result<RemoteBindingCleanupDisposition, DaemonError> {
        let Some(pending) = self.remote_binding_cleanups.get(intent_id).cloned() else {
            return Ok(RemoteBindingCleanupDisposition::Completed);
        };
        let intent = pending.intent;
        let relay_config = intent.relay_config(&self.config);
        let target = ClientTarget {
            daemon_id: Some(intent.worker_kernel_id.clone()),
            daemon_alias: None,
        };

        let lease_result = self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &relay_config,
            target,
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: intent.execution_lease_id.clone(),
            },
        ));
        match lease_result {
            Ok(RelayPeerResponse::ExecutionLeaseDestroyed { lease_id })
                if lease_id == intent.execution_lease_id =>
            {
                // The Worker acknowledges this response both for a newly destroyed lease and for
                // a previously destroyed lease whose first response was lost. Persist completion
                // before removing the in-memory job so restart recovery remains idempotent.
                if let Err(error) = self.durable_state.append_event(
                    CLEANUP_COMPLETED_EVENT_KIND,
                    Some(intent.agent_id.clone()),
                    serde_json::json!({
                        "intent_id": intent.intent_id,
                        "execution_lease_id": intent.execution_lease_id,
                    }),
                ) {
                    self.record_remote_binding_cleanup_failure(
                        &intent,
                        now_ms,
                        format!("persist cleanup completion failed: {error}"),
                    );
                    return Ok(RemoteBindingCleanupDisposition::Pending);
                }
                self.remote_binding_cleanups.remove(&intent.intent_id);
                crate::logging::info_with_fields(
                    "remote_agent_binding.cleanup",
                    "retired remote execution lease cleanup completed",
                    serde_json::json!({
                        "agent_id": intent.agent_id,
                        "worker_kernel_id": intent.worker_kernel_id,
                        "execution_lease_id": intent.execution_lease_id,
                    }),
                );
                Ok(RemoteBindingCleanupDisposition::Completed)
            }
            Ok(other) => {
                self.record_remote_binding_cleanup_failure(
                    &intent,
                    now_ms,
                    format!("unexpected destroy execution lease response: {other:?}"),
                );
                Ok(RemoteBindingCleanupDisposition::Pending)
            }
            Err(error) => {
                self.record_remote_binding_cleanup_failure(
                    &intent,
                    now_ms,
                    format!("destroy execution lease failed: {error}"),
                );
                Ok(RemoteBindingCleanupDisposition::Pending)
            }
        }
    }

    fn record_remote_binding_cleanup_failure(
        &mut self,
        intent: &RemoteBindingCleanupIntent,
        now_ms: u64,
        error: String,
    ) {
        let Some((attempt_count, next_attempt_at_ms)) = self
            .remote_binding_cleanups
            .record_failure(&intent.intent_id, now_ms, error.clone())
        else {
            return;
        };
        if let Err(persist_error) = self.durable_state.append_event(
            CLEANUP_ATTEMPTED_EVENT_KIND,
            Some(intent.agent_id.clone()),
            serde_json::json!({
                "intent_id": intent.intent_id,
                "attempt_count": attempt_count,
                "next_attempt_at_ms": next_attempt_at_ms,
                "error": error,
            }),
        ) {
            crate::logging::warn_with_fields(
                "remote_agent_binding.cleanup",
                "could not persist remote binding cleanup retry state",
                serde_json::json!({
                    "agent_id": intent.agent_id,
                    "worker_kernel_id": intent.worker_kernel_id,
                    "execution_lease_id": intent.execution_lease_id,
                    "error": persist_error.to_string(),
                }),
            );
        }
        crate::logging::warn_with_fields(
            "remote_agent_binding.cleanup",
            "retired remote execution lease cleanup is pending retry",
            serde_json::json!({
                "agent_id": intent.agent_id,
                "worker_kernel_id": intent.worker_kernel_id,
                "execution_lease_id": intent.execution_lease_id,
                "attempt_count": attempt_count,
                "next_attempt_at_ms": next_attempt_at_ms,
                "error": error,
            }),
        );
    }

    #[cfg(test)]
    pub(crate) fn pending_remote_binding_cleanup_count_for_test(&self) -> usize {
        self.remote_binding_cleanups.len()
    }
}

fn cleanup_intent_id(worker_kernel_id: &str, execution_lease_id: &str) -> String {
    format!("{worker_kernel_id}:{execution_lease_id}")
}

fn cleanup_retry_delay_ms(attempt_count: u32) -> u64 {
    let shift = attempt_count.saturating_sub(1).min(5);
    CLEANUP_RETRY_BASE_MS
        .saturating_mul(1_u64 << shift)
        .min(CLEANUP_RETRY_MAX_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::DurableStateEvent;

    fn intent() -> RemoteBindingCleanupIntent {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://relay.example.test".to_string());
        config.relay_token = Some("relay-secret".to_string());
        RemoteBindingCleanupIntent::new(
            "agent-1",
            "worker-1",
            "machine-1",
            "lease-1",
            Some("leased-agent-1"),
            &config,
        )
    }

    fn event(sequence: u64, kind: &str, payload: serde_json::Value) -> DurableStateEvent {
        DurableStateEvent {
            sequence,
            event_id: format!("event-{sequence}"),
            kind: kind.to_string(),
            subject_id: Some("agent-1".to_string()),
            timestamp_ms: sequence,
            payload,
        }
    }

    #[test]
    fn durable_cleanup_log_restores_pending_attempt_and_completion() {
        let intent = intent();
        let mut queue = RemoteBindingCleanupQueue::default();
        queue.restore(vec![
            event(
                1,
                CLEANUP_REQUESTED_EVENT_KIND,
                serde_json::json!({ "intent": &intent }),
            ),
            event(
                2,
                CLEANUP_ATTEMPTED_EVENT_KIND,
                serde_json::json!({
                    "intent_id": intent.intent_id,
                    "attempt_count": 2,
                    "next_attempt_at_ms": 9_000,
                    "error": "relay offline",
                }),
            ),
        ]);

        assert_eq!(queue.len(), 1);
        let pending = queue
            .get(&intent.intent_id)
            .expect("cleanup should restore");
        assert_eq!(pending.attempt_count, 2);
        assert_eq!(pending.next_attempt_at_ms, 9_000);
        assert_eq!(pending.last_error.as_deref(), Some("relay offline"));

        queue.restore(vec![
            event(
                1,
                CLEANUP_REQUESTED_EVENT_KIND,
                serde_json::json!({ "intent": &intent }),
            ),
            event(
                3,
                CLEANUP_COMPLETED_EVENT_KIND,
                serde_json::json!({ "intent_id": intent.intent_id }),
            ),
        ]);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn atomic_binding_refresh_event_restores_retired_binding_cleanup() {
        let intent = intent();
        let mut queue = RemoteBindingCleanupQueue::default();
        queue.restore(vec![event(
            1,
            REMOTE_BINDING_REFRESHED_EVENT_KIND,
            serde_json::json!({
                "agent": { "id": "agent-1" },
                "cleanup_intent": &intent,
            }),
        )]);

        assert!(queue.get(&intent.intent_id).is_some());
    }

    #[test]
    fn cleanup_retry_backoff_is_bounded() {
        assert_eq!(cleanup_retry_delay_ms(1), 1_000);
        assert_eq!(cleanup_retry_delay_ms(2), 2_000);
        assert_eq!(cleanup_retry_delay_ms(6), 30_000);
        assert_eq!(cleanup_retry_delay_ms(u32::MAX), 30_000);
    }

    #[test]
    fn pending_cleanup_intent_survives_daemon_restart() {
        let config = DaemonConfig::for_tests();
        let durable_state =
            crate::durable_state::DurableKernelStateStore::open(config.durable_state_path())
                .expect("durable state should open");
        let intent = intent();
        durable_state
            .append_event(
                CLEANUP_REQUESTED_EVENT_KIND,
                Some(intent.agent_id.clone()),
                serde_json::json!({ "intent": &intent }),
            )
            .expect("cleanup request should persist");
        drop(durable_state);

        let app = DaemonApp::bootstrap(config).expect("daemon should restore");
        assert_eq!(app.pending_remote_binding_cleanup_count_for_test(), 1);
    }

    #[test]
    fn indeterminate_candidate_cleanup_survives_restart_for_retry() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:9".to_string());
        config.relay_token = Some("offline-relay".to_string());
        config.relay_request_timeout_ms = 50;
        let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon should boot");

        let disposition = app
            .retire_remote_binding_candidate(
                "agent-offline",
                "worker-offline",
                "machine-offline",
                "lease-offline",
                Some("leased-agent-offline"),
                &config,
            )
            .expect("failed transport should leave a durable cleanup job");
        assert_eq!(disposition, RemoteBindingCleanupDisposition::Pending);
        assert_eq!(app.pending_remote_binding_cleanup_count_for_test(), 1);
        drop(app);

        let restored = DaemonApp::bootstrap(config).expect("daemon should restore cleanup job");
        assert_eq!(restored.pending_remote_binding_cleanup_count_for_test(), 1);
    }
}
