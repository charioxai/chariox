//! Relay kernel event emission, retention, replay, and replay-gap recovery.

use super::*;

const RELAY_EVENT_LOG_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct RelayEventRuntime {
    pub(super) event_log: EventLog<KernelEvent>,
}

impl RelayEventRuntime {
    pub(super) fn new(
        event_counter_path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, DaemonError> {
        let event_counter_path = event_counter_path.into();
        Ok(Self {
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

    #[cfg(test)]
    pub(super) fn for_tests(retention_limit: usize) -> Self {
        Self {
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
    event: KernelEvent,
) -> Result<(), DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let plaintext = serde_json::to_vec(&event).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay event",
        message: error.to_string(),
    })?;
    let encrypted_event =
        relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)?;
    let event_id = event_runtime
        .event_log
        .append(event_stream_id.to_string(), event.clone())
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "reserve relay event id",
            message: error.to_string(),
        })?
        .event_id;
    send_relay_event_frame(outgoing_tx, subscription_id, event_id, encrypted_event)
}

pub(super) async fn replay_recent_relay_events(
    event_runtime: &Arc<RelayEventRuntime>,
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    client_public_key: &str,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
) -> Result<(), DaemonError> {
    let Some(cursor) = resume_from_event_id else {
        return Ok(());
    };
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
            return Ok(());
        }
    };
    for persisted in events {
        if persisted.event_id <= cursor {
            continue;
        }
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        let daemon_private_key = router.relay_private_key();
        let plaintext =
            serde_json::to_vec(&persisted.event).map_err(|error| DaemonError::LocalTransport {
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
        )?;
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
                    session: Box::new(projection.session),
                    provider_run: Box::new(projection.provider_run),
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

fn send_relay_event_frame(
    outgoing_tx: &RelayOutgoingSender,
    subscription_id: &str,
    event_id: u64,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonEvent {
            subscription_id: subscription_id.to_string(),
            event_id,
            encrypted_event,
        },
    )
}
