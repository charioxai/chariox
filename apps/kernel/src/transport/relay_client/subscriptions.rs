//! Relay subscription task lifecycle and event polling loops.

use super::*;

pub(super) type RelaySubscriptionTasks = Arc<Mutex<BTreeMap<String, RelaySubscriptionTask>>>;

pub(super) struct RelaySubscriptionTask {
    pub(super) relay_subscription_id: String,
    pub(super) handle: JoinHandle<()>,
}

pub(super) async fn abort_subscription_tasks(subscription_tasks: &RelaySubscriptionTasks) {
    let mut guard = subscription_tasks.lock().await;
    for (_, task) in guard.iter() {
        task.handle.abort();
    }
    guard.clear();
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

pub(super) async fn run_relay_subscription_loop(
    router: Arc<CommandRouter>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: String,
    client_public_key: String,
    session_id: String,
    attachment_id: String,
    subscription_scope: Option<String>,
    event_runtime: Arc<RelayEventRuntime>,
) {
    if subscription_scope.as_deref() == Some("waiting_room_inventory") {
        run_relay_waiting_room_inventory_subscription_loop(
            router,
            outgoing_tx,
            subscription_id,
            client_public_key,
            event_runtime,
        )
        .await;
        return;
    }
    let mut previous_snapshot: Option<SessionSnapshotProjection> = None;
    let mut previous_inventory_version = None;
    let mut last_workflow_design_sequence = 0_u64;
    let mut tick: u64 = 0;
    let event_stream_id = subscription_event_stream_id(&session_id, &attachment_id);

    loop {
        let watch_result = router
            .relay_watch_subscription_state(
                &session_id,
                &attachment_id,
                tick,
                previous_snapshot.clone(),
                last_workflow_design_sequence,
            )
            .await;

        match watch_result {
            WatchResult::Ok {
                records,
                notices,
                completions,
                workflow_design_events,
                snapshot,
            } => {
                if !records.is_empty()
                    && emit_relay_event(
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
                    break;
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
                    previous_snapshot = Some(snapshot.clone());
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
                if tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS) {
                    match router.waiting_room_inventory_version().await {
                        Ok(inventory_version) => {
                            if previous_inventory_version.as_ref() != Some(&inventory_version) {
                                previous_inventory_version = Some(inventory_version.clone());
                                if emit_relay_event(
                                    &router,
                                    &outgoing_tx,
                                    &subscription_id,
                                    &client_public_key,
                                    &event_runtime,
                                    WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                                    KernelEvent::WaitingRoomInventoryChanged { inventory_version },
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
                                "relay event loop failed to build waiting-room inventory version",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "attachment_id": attachment_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
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

        tick = tick.wrapping_add(1);
        sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
    }
}

async fn run_relay_waiting_room_inventory_subscription_loop(
    router: Arc<CommandRouter>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: String,
    client_public_key: String,
    event_runtime: Arc<RelayEventRuntime>,
) {
    let mut previous_inventory_version = None;
    loop {
        match router.waiting_room_inventory_version().await {
            Ok(inventory_version) => {
                if previous_inventory_version.as_ref() != Some(&inventory_version) {
                    previous_inventory_version = Some(inventory_version.clone());
                    if emit_relay_event(
                        &router,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_runtime,
                        WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
                        KernelEvent::WaitingRoomInventoryChanged { inventory_version },
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
                    "relay waiting-room inventory subscription failed to build version",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
        sleep(Duration::from_millis(
            WATCH_INTERVAL_MS * RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
        ))
        .await;
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
