//! Relay subscription task lifecycle and event polling loops.

use super::request_errors::map_relay_error;
use super::*;
use crate::runtime::projection::SessionSnapshotProjection;
use arroba_relay::protocol::RelayCallerIdentity;

pub(super) type RelaySubscriptionTasks = Arc<Mutex<BTreeMap<String, RelaySubscriptionTask>>>;

pub(super) struct RelaySubscriptionTask {
    pub(super) relay_subscription_id: String,
    pub(super) session_id: String,
    pub(super) attachment_id: String,
    pub(super) subscription_scope: Option<String>,
    pub(super) handle: JoinHandle<()>,
}

pub(super) async fn abort_subscription_tasks(
    router: &Arc<CommandRouter>,
    subscription_tasks: &RelaySubscriptionTasks,
) {
    let mut guard = subscription_tasks.lock().await;
    let tasks = guard
        .values()
        .map(RelaySubscriptionTask::snapshot)
        .collect::<Vec<_>>();
    for task in guard.values() {
        task.handle.abort();
    }
    guard.clear();
    drop(guard);
    for task in tasks {
        cleanup_relay_subscription_attachment(router, &task).await;
    }
}

pub(super) async fn remove_relay_subscription_task_by_relay_id(
    subscription_tasks: &RelaySubscriptionTasks,
    relay_subscription_id: &str,
) -> Option<RelaySubscriptionTask> {
    let mut guard = subscription_tasks.lock().await;
    let task_key = guard.iter().find_map(|(key, task)| {
        (task.relay_subscription_id == relay_subscription_id).then(|| key.clone())
    })?;
    guard.remove(&task_key)
}

pub(super) fn relay_subscription_task_key(
    session_id: &str,
    attachment_id: &str,
    subscription_scope: Option<&str>,
) -> String {
    let scope = subscription_scope.unwrap_or("session");
    format!("{scope}\u{1f}{session_id}\u{1f}{attachment_id}")
}

#[derive(Debug, Clone)]
struct RelaySubscriptionTaskSnapshot {
    session_id: String,
    attachment_id: String,
    subscription_scope: Option<String>,
}

impl RelaySubscriptionTask {
    fn snapshot(&self) -> RelaySubscriptionTaskSnapshot {
        RelaySubscriptionTaskSnapshot {
            session_id: self.session_id.clone(),
            attachment_id: self.attachment_id.clone(),
            subscription_scope: self.subscription_scope.clone(),
        }
    }
}

async fn cleanup_relay_subscription_attachment(
    router: &Arc<CommandRouter>,
    task: &RelaySubscriptionTaskSnapshot,
) {
    if task.subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE)
        || task.session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
        || task.attachment_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
    {
        return;
    }
    match router
        .detach_relay_subscription_attachment(&task.attachment_id)
        .await
    {
        Ok(()) => {}
        Err(DaemonError::AttachmentNotFound { .. })
        | Err(DaemonError::AttachmentNotInSession { .. }) => {}
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to clean up relay subscription attachment",
                serde_json::json!({
                    "session_id": task.session_id,
                    "attachment_id": task.attachment_id,
                    "subscription_scope": task.subscription_scope,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

pub(super) async fn handle_relay_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &RelaySubscriptionTasks,
    event_runtime: &Arc<RelayEventRuntime>,
    relay_request_id: String,
    relay_subscription_id: String,
    session_id: String,
    attachment_id: String,
    caller_identity: Option<RelayCallerIdentity>,
    client_public_key: String,
    subscription_scope: Option<String>,
    resume_from_event_id: Option<u64>,
) -> Result<(), DaemonError> {
    let is_inventory_subscription =
        subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE);
    if resume_from_event_id.is_none() {
        crate::logging::info_with_fields(
            "daemon.relay_client",
            "relay subscription request received",
            serde_json::json!({
                "relay_request_id": relay_request_id,
                "relay_subscription_id": relay_subscription_id,
                "session_id": session_id,
                "attachment_id": attachment_id,
                "subscription_scope": subscription_scope,
                "resume_from_event_id": resume_from_event_id,
                "is_waiting_room_inventory_subscription": is_inventory_subscription,
            }),
        );
    } else {
        crate::logging::debug_with_fields(
            "daemon.relay_client",
            "relay subscription resume request received",
            serde_json::json!({
                "relay_request_id": relay_request_id,
                "relay_subscription_id": relay_subscription_id,
                "session_id": session_id,
                "attachment_id": attachment_id,
                "subscription_scope": subscription_scope,
                "resume_from_event_id": resume_from_event_id,
                "is_waiting_room_inventory_subscription": is_inventory_subscription,
            }),
        );
    }
    if !is_inventory_subscription
        && (session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
            || attachment_id == WAITING_ROOM_INVENTORY_SENTINEL_ID)
    {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "waiting-room inventory sentinel arrived without subscription scope",
            serde_json::json!({
                "relay_request_id": relay_request_id,
                "relay_subscription_id": relay_subscription_id,
                "session_id": session_id,
                "attachment_id": attachment_id,
                "subscription_scope": subscription_scope,
                "diagnosis": "relay or client likely dropped subscription_scope=waiting_room_inventory",
            }),
        );
    }
    if !is_inventory_subscription {
        let validation = if let Some(user_id) = caller_identity
            .as_ref()
            .and_then(|identity| identity.user_id.as_deref())
        {
            router
                .ensure_relay_subscription_attachment_for_user(&session_id, &attachment_id, user_id)
                .await
        } else {
            router
                .ensure_relay_subscription_attachment(&session_id, &attachment_id)
                .await
        };
        if let Err(error) = validation {
            if resume_from_event_id.is_none() {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay subscription attachment validation failed",
                    serde_json::json!({
                        "relay_request_id": relay_request_id,
                        "relay_subscription_id": relay_subscription_id,
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "subscription_scope": subscription_scope,
                        "error": error.to_string(),
                    }),
                );
            } else {
                crate::logging::debug_with_fields(
                    "daemon.relay_client",
                    "relay subscription resume attachment validation failed",
                    serde_json::json!({
                        "relay_request_id": relay_request_id,
                        "relay_subscription_id": relay_subscription_id,
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "subscription_scope": subscription_scope,
                        "error": error.to_string(),
                    }),
                );
            }
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                },
            )?;
            return Ok(());
        }
    }
    let task_key =
        relay_subscription_task_key(&session_id, &attachment_id, subscription_scope.as_deref());
    let ack = match encrypt_json_response(
        router,
        &client_public_key,
        serde_json::json!({
            "ok": true,
            "resumed_from_event_id": resume_from_event_id,
        }),
    )
    .await
    {
        Ok(ack) => ack,
        Err(error) => {
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                },
            )?;
            return Ok(());
        }
    };
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: Some(ack),
            error: None,
        },
    )?;
    if subscription_tasks
        .lock()
        .await
        .get(&task_key)
        .is_some_and(|existing| {
            existing.relay_subscription_id == relay_subscription_id
                && !existing.handle.is_finished()
        })
    {
        return Ok(());
    }
    if let Some(existing) = subscription_tasks.lock().await.remove(&task_key) {
        existing.handle.abort();
        cleanup_relay_subscription_attachment(router, &existing.snapshot()).await;
    }
    if !is_inventory_subscription {
        if let Err(error) = replay_recent_relay_events(
            event_runtime,
            router,
            outgoing_tx,
            &relay_subscription_id,
            &client_public_key,
            &session_id,
            &attachment_id,
            resume_from_event_id,
        )
        .await
        {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to replay relay subscription events",
                serde_json::json!({
                    "relay_subscription_id": relay_subscription_id,
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
    let task = tokio::spawn(run_relay_subscription_loop(
        Arc::clone(router),
        outgoing_tx.clone(),
        relay_subscription_id.clone(),
        client_public_key,
        session_id.clone(),
        attachment_id.clone(),
        subscription_scope.clone(),
        Arc::clone(event_runtime),
        resume_from_event_id.is_some(),
    ));
    subscription_tasks.lock().await.insert(
        task_key,
        RelaySubscriptionTask {
            relay_subscription_id,
            session_id,
            attachment_id,
            subscription_scope,
            handle: task,
        },
    );
    Ok(())
}

pub(super) async fn handle_relay_unsubscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &RelaySubscriptionTasks,
    relay_request_id: String,
    relay_subscription_id: String,
    client_public_key: String,
) -> Result<(), DaemonError> {
    let existing =
        remove_relay_subscription_task_by_relay_id(subscription_tasks, &relay_subscription_id)
            .await;
    if let Some(task) = existing {
        task.handle.abort();
        cleanup_relay_subscription_attachment(router, &task.snapshot()).await;
    }
    let ack = match encrypt_json_response(
        router,
        &client_public_key,
        serde_json::json!({ "ok": true }),
    )
    .await
    {
        Ok(ack) => ack,
        Err(error) => {
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                },
            )?;
            return Ok(());
        }
    };
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: Some(ack),
            error: None,
        },
    )
}

pub(super) async fn run_relay_subscription_loop(
    router: Arc<CommandRouter>,
    outgoing_tx: RelayOutgoingSender,
    subscription_id: String,
    client_public_key: String,
    session_id: String,
    attachment_id: String,
    subscription_scope: Option<String>,
    event_runtime: Arc<RelayEventRuntime>,
    resumed: bool,
) {
    if subscription_scope.as_deref() == Some("waiting_room_inventory") {
        run_relay_waiting_room_inventory_subscription_loop(
            router,
            outgoing_tx,
            subscription_id,
            client_public_key,
            event_runtime,
            resumed,
        )
        .await;
        return;
    }
    let mut previous_snapshot: Option<SessionSnapshotProjection> = None;
    let mut last_workflow_design_sequence = 0_u64;
    let mut last_snapshot_projection_sequence: Option<u64> = None;
    let mut last_snapshot_check_tick = 0_u64;
    let mut tick: u64 = 0;
    let event_stream_id = subscription_event_stream_id(&session_id, &attachment_id);

    loop {
        let terminal_change_sequence = router.terminal_stream_change_sequence();
        let session_projection_change_sequence = router.session_projection_change_sequence();
        let should_check_snapshot = previous_snapshot.is_none()
            || last_snapshot_projection_sequence != Some(session_projection_change_sequence)
            || tick.wrapping_sub(last_snapshot_check_tick)
                >= SESSION_SNAPSHOT_RECONCILIATION_INTERVAL_TICKS;
        let previous_snapshot_for_watch = if should_check_snapshot {
            previous_snapshot.clone()
        } else {
            None
        };
        let watch_result = router
            .relay_watch_subscription_state(
                &session_id,
                &attachment_id,
                should_check_snapshot,
                previous_snapshot_for_watch,
                last_workflow_design_sequence,
            )
            .await;
        if should_check_snapshot {
            last_snapshot_projection_sequence = Some(session_projection_change_sequence);
            last_snapshot_check_tick = tick;
        }

        match watch_result {
            WatchResult::Ok {
                records,
                notices,
                completions,
                workflow_design_events,
                snapshot,
            } => {
                if !records.is_empty() {
                    let mut terminal_output_failed = false;
                    for records in terminal_output_event_batches(records) {
                        if emit_relay_event(
                            &router,
                            &outgoing_tx,
                            &subscription_id,
                            &client_public_key,
                            &event_runtime,
                            &event_stream_id,
                            KernelEvent::TerminalOutput { records },
                        )
                        .await
                        .is_err()
                        {
                            terminal_output_failed = true;
                            break;
                        }
                    }
                    if terminal_output_failed {
                        break;
                    }
                }
                if !notices.is_empty()
                    && emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        &event_stream_id,
                        KernelEvent::RuntimeNotices { notices },
                    )
                    .await
                    .is_err()
                {
                    break;
                }
                for completion in completions {
                    if emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        &event_stream_id,
                        KernelEvent::AssistantMessageCompleted {
                            session_id: completion.session_id,
                            provider_run_id: completion.provider_run_id,
                            agent_id: completion.agent_id,
                            recipient_attachment_ids: completion.recipient_attachment_ids,
                            message_id: completion.message_id,
                            completed_at_ms: completion.completed_at_ms,
                        },
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
                for workflow_event in workflow_design_events {
                    last_workflow_design_sequence =
                        last_workflow_design_sequence.max(workflow_event.kernel_sequence);
                    if emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        &event_stream_id,
                        KernelEvent::WorkflowDesignOp {
                            design_op: workflow_event,
                        },
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
                if let Some(snapshot) = *snapshot {
                    let previous_snapshot_ref = previous_snapshot.as_ref();
                    let mut emitted_projection_delta = false;
                    let mut emit_failed = false;
                    for event in [
                        agent_activity_changed_event(&snapshot, previous_snapshot_ref),
                        provider_run_changed_event(&snapshot, previous_snapshot_ref),
                        session_metadata_changed_event(&snapshot, previous_snapshot_ref),
                        runtime_interactions_changed_event(&snapshot, previous_snapshot_ref),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        emitted_projection_delta = true;
                        if emit_relay_event(
                            &router,
                            &outgoing_tx,
                            &subscription_id,
                            &client_public_key,
                            &event_runtime,
                            &event_stream_id,
                            event,
                        )
                        .await
                        .is_err()
                        {
                            emit_failed = true;
                            break;
                        }
                    }
                    if emit_failed {
                        break;
                    }
                    let workflow_run_events =
                        workflow_run_updated_events(&snapshot, previous_snapshot_ref);
                    let workflow_run_only =
                        workflow_run_only_changed(&snapshot, previous_snapshot_ref)
                            && !workflow_run_events.is_empty();
                    for event in workflow_run_events {
                        emitted_projection_delta = true;
                        if emit_relay_event(
                            &router,
                            &outgoing_tx,
                            &subscription_id,
                            &client_public_key,
                            &event_runtime,
                            &event_stream_id,
                            event,
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                    previous_snapshot = Some(snapshot.clone());
                    if emitted_projection_delta || workflow_run_only {
                        continue;
                    }
                    if emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        &event_stream_id,
                        KernelEvent::SessionSnapshot {
                            session: Box::new(snapshot.session),
                            provider_run: Box::new(snapshot.provider_run),
                            agent_activity: Box::new(snapshot.agent_activity),
                            agent_activity_revision: snapshot.metadata.last_event_id,
                        },
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
                if tick.is_multiple_of(RELAY_HEARTBEAT_INTERVAL_TICKS)
                    && emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        &event_stream_id,
                        KernelEvent::Heartbeat {
                            session_id: session_id.clone(),
                        },
                    )
                    .await
                    .is_err()
                {
                    break;
                }
            }
            WatchResult::Unavailable(message) => {
                let _ = emit_relay_event(
                    &router,
                    &outgoing_tx,
                    &subscription_id,
                    &client_public_key,
                    &event_runtime,
                    &event_stream_id,
                    KernelEvent::SessionUnavailable {
                        session_id: session_id.clone(),
                        message,
                    },
                )
                .await;
                break;
            }
        }

        let wait_ms = router.transport_runtime_pump_interval_ms(
            WATCH_INTERVAL_MS,
            IDLE_SUBSCRIPTION_WAIT_INTERVAL_MS,
            crate::session::unix_epoch_ms(),
        );
        let wait_started = Instant::now();
        let _ = timeout(
            Duration::from_millis(wait_ms),
            async {
                tokio::select! {
                    _ = router.wait_for_terminal_stream_change_after(terminal_change_sequence) => {}
                    _ = router.wait_for_session_projection_change_after(session_projection_change_sequence) => {}
                }
            },
        )
        .await;
        let elapsed_ticks =
            ((wait_started.elapsed().as_millis() as u64) / WATCH_INTERVAL_MS).max(1);
        tick = tick.wrapping_add(elapsed_ticks);
    }
}

async fn run_relay_waiting_room_inventory_subscription_loop(
    router: Arc<CommandRouter>,
    outgoing_tx: RelayOutgoingSender,
    subscription_id: String,
    client_public_key: String,
    event_runtime: Arc<RelayEventRuntime>,
    resumed: bool,
) {
    let mut previous_waiting_room_snapshot = None;
    let mut previous_relay_status = if resumed {
        Some(router.transport_relay_status_snapshot().await)
    } else {
        None
    };
    let mut previous_remote_machines = resumed.then(|| router.transport_remote_machines_snapshot());
    let mut previous_provider_catalog = resumed
        .then(|| router.transport_provider_catalog_snapshot())
        .flatten();
    let mut previous_slices = resumed.then(|| router.transport_slices_snapshot());
    let mut inventory_dirty = !resumed;
    let mut tick: u64 = 0;
    loop {
        let waiting_room_change_sequence = router.waiting_room_change_sequence();
        if inventory_dirty
            || (!resumed && tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS))
        {
            match router.waiting_room_public_snapshot().await {
                Ok(snapshot) => {
                    inventory_dirty = false;
                    if let Some(event) = waiting_room_rows_changed_event(
                        snapshot.clone(),
                        previous_waiting_room_snapshot.as_ref(),
                    ) {
                        previous_waiting_room_snapshot = Some(snapshot);
                        if emit_relay_event(
                            &router,
                            &outgoing_tx,
                            &subscription_id,
                            &client_public_key,
                            &event_runtime,
                            WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                            event,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "relay waiting-room inventory subscription failed to build snapshot",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                }
            }
        }
        if tick.is_multiple_of(RELAY_HEARTBEAT_INTERVAL_TICKS) {
            let status = router.transport_relay_status_snapshot().await;
            if previous_relay_status.as_ref() != Some(&status) {
                previous_relay_status = Some(status.clone());
                if emit_relay_event(
                    &router,
                    &outgoing_tx,
                    &subscription_id,
                    &client_public_key,
                    &event_runtime,
                    WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                    KernelEvent::RelayStatusChanged { status },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        }
        if tick.is_multiple_of(RELAY_REMOTE_MACHINE_DISCOVERY_INTERVAL_TICKS) {
            let machines = router.transport_remote_machines_snapshot();
            if previous_remote_machines.as_ref() != Some(&machines) {
                previous_remote_machines = Some(machines.clone());
                if emit_relay_event(
                    &router,
                    &outgoing_tx,
                    &subscription_id,
                    &client_public_key,
                    &event_runtime,
                    WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                    KernelEvent::RemoteMachinesChanged { machines },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        }
        if let Some(catalog) = router.transport_provider_catalog_snapshot() {
            if previous_provider_catalog.as_ref() != Some(&catalog) {
                previous_provider_catalog = Some(catalog.clone());
                if emit_relay_event(
                    &router,
                    &outgoing_tx,
                    &subscription_id,
                    &client_public_key,
                    &event_runtime,
                    WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                    KernelEvent::ProviderCatalogChanged {
                        generated_at_ms: crate::session::unix_epoch_ms(),
                        catalog,
                    },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        }
        let slices = router.transport_slices_snapshot();
        if previous_slices.as_ref() != Some(&slices) {
            previous_slices = Some(slices.clone());
            if emit_relay_event(
                &router,
                &outgoing_tx,
                &subscription_id,
                &client_public_key,
                &event_runtime,
                WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                KernelEvent::SlicesChanged {
                    generated_at_ms: crate::session::unix_epoch_ms(),
                    slices,
                },
            )
            .await
            .is_err()
            {
                break;
            }
        }
        let wait_started = Instant::now();
        let wait_result = timeout(
            Duration::from_millis(WATCH_INTERVAL_MS * RELAY_HEARTBEAT_INTERVAL_TICKS),
            router.wait_for_waiting_room_change_after(waiting_room_change_sequence),
        )
        .await;
        if wait_result.is_ok() {
            sleep(Duration::from_millis(WAITING_ROOM_ROW_COALESCE_MS)).await;
            inventory_dirty = true;
        }
        let elapsed_ticks =
            ((wait_started.elapsed().as_millis() as u64) / WATCH_INTERVAL_MS).max(1);
        tick = tick.wrapping_add(elapsed_ticks);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        relay_subscription_task_key, remove_relay_subscription_task_by_relay_id,
        RelaySubscriptionTask, RelaySubscriptionTasks,
    };

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn relay_subscription_tasks_are_owned_by_logical_attachment_subscription() {
        let tasks: RelaySubscriptionTasks = Arc::new(Mutex::new(BTreeMap::new()));
        let first_key = relay_subscription_task_key("session-1", "attachment-1", None);
        let second_key = relay_subscription_task_key("session-1", "attachment-1", None);
        assert_eq!(first_key, second_key);

        let first_handle = tokio::spawn(async {
            sleep(Duration::from_secs(60)).await;
        });
        tasks.lock().await.insert(
            first_key.clone(),
            RelaySubscriptionTask {
                relay_subscription_id: "relay-subscription-1".to_string(),
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                subscription_scope: None,
                handle: first_handle,
            },
        );

        let second_handle = tokio::spawn(async {
            sleep(Duration::from_secs(60)).await;
        });
        if let Some(existing) = tasks.lock().await.remove(&second_key) {
            existing.handle.abort();
        }
        tasks.lock().await.insert(
            second_key,
            RelaySubscriptionTask {
                relay_subscription_id: "relay-subscription-2".to_string(),
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                subscription_scope: None,
                handle: second_handle,
            },
        );
        assert_eq!(tasks.lock().await.len(), 1);

        let removed =
            remove_relay_subscription_task_by_relay_id(&tasks, "relay-subscription-2").await;
        assert!(removed.is_some());
        assert!(tasks.lock().await.is_empty());
    }
}
