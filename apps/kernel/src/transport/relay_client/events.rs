//! Relay kernel event emission, retention, replay, and replay-gap recovery.

use super::*;
use crate::runtime::event_log::SerializedEvent;

const RELAY_EVENT_LOG_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct RelayEventRuntime {
    pub(super) event_log: EventLog<KernelEvent>,
    serialized_snapshot: tokio::sync::Mutex<Option<Arc<SerializedEvent<KernelEvent>>>>,
}

impl RelayEventRuntime {
    pub(super) fn new(
        event_counter_path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, DaemonError> {
        let event_counter_path = event_counter_path.into();
        Ok(Self {
            serialized_snapshot: Default::default(),
            event_log: EventLog::new_with_persistent_event_store_and_retention(
                EventRetentionPolicy {
                    max_stream_events: RECENT_EVENT_LIMIT,
                    max_total_bytes: Some(RELAY_EVENT_LOG_MAX_BYTES),
                    max_age_ms: Some(
                        crate::runtime::event_log::DEFAULT_PERSISTENT_EVENT_MAX_AGE_MS,
                    ),
                },
                event_counter_path.clone(),
                event_counter_path.with_file_name("relay-events.jsonl"),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "reserve relay kernel event ids",
                message: error.to_string(),
            })?,
        })
    }

    async fn serialized_snapshot(
        &self,
        event: KernelEvent,
    ) -> Result<Arc<SerializedEvent<KernelEvent>>, DaemonError> {
        let mut cached = self.serialized_snapshot.lock().await;
        if let Some(encoded) = cached.as_ref().filter(|encoded| encoded.event() == &event) {
            if crate::runtime::perf_timing::enabled() {
                crate::logging::info_with_fields(
                    "daemon.event_encoding",
                    "snapshot encoding reused",
                    serde_json::json!({"cache_hit":true,"bytes":encoded.json().get().len()}),
                );
            }
            return Ok(Arc::clone(encoded));
        }
        // One bounded cache also admits only one bulk encoder at a time. Free
        // the runtime workers to publish controls while serialization runs.
        let encoded = tokio::task::spawn_blocking(move || SerializedEvent::new(event))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "serialize relay snapshot",
                message: error.to_string(),
            })?
            .map_err(|error| DaemonError::LocalTransport {
                operation: "serialize relay snapshot",
                message: error.to_string(),
            })?;
        let encoded = Arc::new(encoded);
        if crate::runtime::perf_timing::enabled() {
            crate::logging::info_with_fields(
                "daemon.event_encoding",
                "snapshot encoded",
                serde_json::json!({"cache_hit":false,"bytes":encoded.json().get().len()}),
            );
        }
        *cached = Some(Arc::clone(&encoded));
        Ok(encoded)
    }

    #[cfg(test)]
    pub(super) fn for_tests(retention_limit: usize) -> Self {
        Self {
            serialized_snapshot: Default::default(),
            event_log: EventLog::new(retention_limit),
        }
    }
}

pub(super) async fn emit_relay_event(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    client_public_key: &str,
    event_runtime: &Arc<RelayEventRuntime>,
    event_stream_id: &str,
    mut event: KernelEvent,
) -> Result<(), DaemonError> {
    let prepared_at_ms = crate::session::unix_epoch_ms();
    router.admit_relay_stream(client_public_key)?;
    let daemon_private_key = router.relay_private_key();
    if matches!(&event, KernelEvent::SessionSnapshot { .. }) {
        let encoded = event_runtime.serialized_snapshot(event).await?;
        let encrypted_event = relay_crypto::encrypt_payload_for_peer(
            &daemon_private_key,
            client_public_key,
            encoded.json().get().as_bytes(),
        )?;
        let append_started = std::time::Instant::now();
        let event_id = event_runtime
            .event_log
            .append_serialized(event_stream_id.to_string(), &encoded)
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "reserve relay event id",
                message: error.to_string(),
            })?
            .event_id;
        crate::runtime::perf_timing::record_relay_event(
            subscription_id,
            event_id,
            encoded.event(),
            prepared_at_ms,
            append_started.elapsed().as_micros(),
        );
        return send_relay_event_frame(outgoing_tx, subscription_id, event_id, encrypted_event)
            .await;
    }
    let plaintext =
        crate::transport::terminal_output_wire::encode(&mut event).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "serialize relay event",
                message: error.to_string(),
            }
        })?;
    let encrypted_event =
        relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)?;
    let append_started = std::time::Instant::now();
    let event_id = event_runtime
        .event_log
        .append(event_stream_id.to_string(), event.clone())
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "reserve relay event id",
            message: error.to_string(),
        })?
        .event_id;
    crate::runtime::perf_timing::record_relay_event(
        subscription_id,
        event_id,
        &event,
        prepared_at_ms,
        append_started.elapsed().as_micros(),
    );
    send_relay_event_frame(outgoing_tx, subscription_id, event_id, encrypted_event).await
}

#[allow(
    clippy::too_many_arguments,
    reason = "Preserve the existing replay_recent_relay_events operation signature and explicit context arguments"
)]
pub(super) async fn replay_recent_relay_events(
    event_runtime: &Arc<RelayEventRuntime>,
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    client_public_key: &str,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
    terminal_output: bool,
) -> Result<(), DaemonError> {
    let Some(cursor) = resume_from_event_id else {
        return Ok(());
    };
    router.admit_relay_stream(client_public_key)?;
    let event_stream_id = subscription_event_stream_id(session_id, attachment_id);
    let events = match event_runtime
        .event_log
        .replay_after(&event_stream_id, cursor)
        .await
    {
        ReplayOutcome::Replayed(events) => events,
        ReplayOutcome::Gap(gap) => {
            emit_relay_event(
                router,
                outgoing_tx,
                subscription_id,
                client_public_key,
                event_runtime,
                &event_stream_id,
                KernelEvent::ReplayGap {
                    session_id: session_id.to_string(),
                    requested_from_event_id: cursor,
                    first_retained_event_id: gap.first_retained_event_id,
                    latest_event_id: gap.latest_event_id,
                    message: "Replay cursor is outside the retained relay event window; refresh the session projection.".to_string(),
                },
            )
            .await?;
            emit_relay_replay_gap_snapshot(
                router,
                outgoing_tx,
                subscription_id,
                client_public_key,
                event_runtime,
                session_id,
                attachment_id,
            )
            .await?;
            emit_relay_event(
                router,
                outgoing_tx,
                subscription_id,
                client_public_key,
                event_runtime,
                &event_stream_id,
                KernelEvent::TransportResumed {
                    session_id: session_id.to_string(),
                    resumed_from_event_id: Some(cursor),
                },
            )
            .await?;
            return Ok(());
        }
    };
    for mut persisted in events {
        if !terminal_output && matches!(&persisted.event, KernelEvent::TerminalOutput { .. }) {
            continue;
        }
        if persisted.event_id <= cursor {
            continue;
        }
        if matches!(&persisted.event, KernelEvent::TransportResumed { .. }) {
            continue;
        }
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        router.admit_relay_stream(client_public_key)?;
        let daemon_private_key = router.relay_private_key();
        let plaintext = crate::transport::terminal_output_wire::encode(&mut persisted.event)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "serialize relay event",
                message: error.to_string(),
            })?;
        let encrypted_event = relay_crypto::encrypt_payload_for_peer(
            &daemon_private_key,
            client_public_key,
            &plaintext,
        )?;
        send_relay_event_frame(
            outgoing_tx,
            subscription_id,
            persisted.event_id,
            encrypted_event,
        )
        .await?;
    }
    emit_relay_event(
        router,
        outgoing_tx,
        subscription_id,
        client_public_key,
        event_runtime,
        &event_stream_id,
        KernelEvent::TransportResumed {
            session_id: session_id.to_string(),
            resumed_from_event_id: Some(cursor),
        },
    )
    .await
}

async fn emit_relay_replay_gap_snapshot(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    client_public_key: &str,
    event_runtime: &Arc<RelayEventRuntime>,
    session_id: &str,
    attachment_id: &str,
) -> Result<(), DaemonError> {
    router.admit_relay_stream(client_public_key)?;
    let event_stream_id = subscription_event_stream_id(session_id, attachment_id);
    let snapshot = router.session_snapshot_projection_for_attachment(
        session_id,
        attachment_id,
        router.session_projection_change_sequence(),
    );
    match snapshot {
        Ok(projection) => {
            emit_relay_event(
                router,
                outgoing_tx,
                subscription_id,
                client_public_key,
                event_runtime,
                &event_stream_id,
                KernelEvent::SessionSnapshot {
                    room_workflows: projection.room_workflows,
                    session: Box::new(projection.session),
                    provider_run: Box::new(
                        projection
                            .provider_run
                            .map(crate::provider::PublicProviderRun::from),
                    ),
                    agent_activity: Box::new(projection.agent_activity),
                    agent_activity_revision: projection.metadata.last_event_id,
                },
            )
            .await?;
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to build replay gap session snapshot",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
    Ok(())
}

async fn send_relay_event_frame(
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    event_id: u64,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    let result = send_outgoing_event_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonEvent {
            subscription_id: subscription_id.to_string(),
            event_id,
            encrypted_event,
        },
    )
    .await;
    if let Err(error) = &result {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "relay subscription event delivery ended",
            serde_json::json!({
                "relay_subscription_id": subscription_id,
                "event_id": event_id,
                "error": error.to_string(),
            }),
        );
    }
    result
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[tokio::test]
    async fn identical_subscriber_snapshots_share_only_their_plaintext_encoding() {
        let runtime = RelayEventRuntime::for_tests(8);
        let snapshot = KernelEvent::SessionSnapshot {
            room_workflows: crate::runtime::projection::RoomWorkflowInventory {
                session_id: "room".into(),
                home_kernel_id: "kernel".into(),
                revision: "1".into(),
                workflow_count: 0,
                workflows: vec![],
            },
            session: Box::new(crate::session::RuntimeSession::new(
                "room",
                None,
                "workspace",
                "worktree",
                "machine",
                "kernel",
            )),
            provider_run: Box::new(None),
            agent_activity: Box::default(),
            agent_activity_revision: 1,
        };
        let first = runtime.serialized_snapshot(snapshot.clone()).await.unwrap();
        let second = runtime.serialized_snapshot(snapshot.clone()).await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let mut next = snapshot;
        if let KernelEvent::SessionSnapshot {
            agent_activity_revision,
            ..
        } = &mut next
        {
            *agent_activity_revision = 2;
        }
        let next = runtime.serialized_snapshot(next).await.unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert_eq!(
            serde_json::from_str::<KernelEvent>(next.json().get()).unwrap(),
            *next.event()
        );
    }
}
