//! Protocol 1006: `kernel_resources` subscriptions over the relay. The
//! subscription task serves the feed; `WatchKernelResources` from the same
//! client key reaches it through the task's watch inbox.
use super::request_errors::{map_relay_error, relay_error};
use super::subscriptions::{relay_subscription_task_key, RelaySubscriptionTask};
use super::*;
use crate::local::{LocalDaemonResponse, WatchKernelResourcesRequest};
use crate::runtime::command::{KernelCaller, KernelCommandSource};
use crate::transport::kernel_resource_feed::{
    run_kernel_resource_feed, submit_watch_set, watch_inbox, KERNEL_RESOURCES_SENTINEL_ID,
    KERNEL_RESOURCES_SUBSCRIPTION_SCOPE,
};

pub(super) struct KernelResourcesSubscription {
    pub(super) relay_request_id: String,
    pub(super) relay_subscription_id: String,
    pub(super) client_public_key: String,
    pub(super) heartbeat_interval: Duration,
}

pub(super) async fn handle_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &super::subscriptions::RelaySubscriptionTasks,
    event_runtime: &Arc<RelayEventRuntime>,
    subscription: KernelResourcesSubscription,
) -> Result<(), DaemonError> {
    let KernelResourcesSubscription {
        relay_request_id,
        relay_subscription_id,
        client_public_key,
        heartbeat_interval,
    } = subscription;
    let ack = match encrypt_json_response(
        router,
        &client_public_key,
        serde_json::json!({ "ok": true, "resumed_from_event_id": null }),
    )
    .await
    {
        Ok(ack) => ack,
        Err(error) => {
            return send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                },
            );
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
    let task_key = relay_subscription_task_key(
        KERNEL_RESOURCES_SENTINEL_ID,
        KERNEL_RESOURCES_SENTINEL_ID,
        Some(KERNEL_RESOURCES_SUBSCRIPTION_SCOPE),
        &relay_subscription_id,
    );
    let (inbox, updates) = watch_inbox();
    let task = tokio::spawn({
        let router = Arc::clone(router);
        let outgoing_tx = outgoing_tx.clone();
        let event_runtime = Arc::clone(event_runtime);
        let subscription_id = relay_subscription_id.clone();
        let client_public_key = client_public_key.clone();
        async move {
            run_kernel_resource_feed(
                Arc::clone(&router),
                KernelCommandSource::RelayClient,
                updates,
                heartbeat_interval,
                |event| {
                    let router = Arc::clone(&router);
                    let outgoing_tx = outgoing_tx.clone();
                    let event_runtime = Arc::clone(&event_runtime);
                    let subscription_id = subscription_id.clone();
                    let client_public_key = client_public_key.clone();
                    async move {
                        super::events::emit_relay_event(
                            &router,
                            &outgoing_tx,
                            &subscription_id,
                            &client_public_key,
                            &event_runtime,
                            KERNEL_RESOURCES_SUBSCRIPTION_SCOPE,
                            event,
                        )
                        .await
                        .is_ok()
                    }
                },
            )
            .await;
        }
    });
    // A re-subscribe with the same id starts over: the client sends its watch set again.
    if let Some(existing) = subscription_tasks.lock().await.insert(
        task_key,
        RelaySubscriptionTask {
            relay_subscription_id,
            client_public_key,
            display_id: None,
            display_pump: None,
            _terminal_lease: None,
            resource_watch: Some(inbox),
            handle: task,
        },
    ) {
        existing.handle.abort();
    }
    Ok(())
}

/// Answers `WatchKernelResources`. A client signs each subscription with its own
/// key and its requests with another, so the subscription is found by its id,
/// which only that client holds; every watch is read with the sender's own authority.
pub(super) async fn watch(
    subscription_tasks: &super::subscriptions::RelaySubscriptionTasks,
    caller: KernelCaller,
    request: WatchKernelResourcesRequest,
) -> Result<LocalDaemonResponse, RelayError> {
    let inbox = subscription_tasks.lock().await.values().find_map(|task| {
        (task.relay_subscription_id == request.subscription_request_id
            && !task.handle.is_finished())
        .then(|| task.resource_watch.clone())
        .flatten()
    });
    let Some(inbox) = inbox else {
        return Err(relay_error(
            "kernel_resources_subscription_required",
            "open a kernel_resources subscription with this id, then watch",
            false,
        ));
    };
    submit_watch_set(&inbox, request.watches, caller)
        .await
        .map(|refused| LocalDaemonResponse::KernelResourcesWatched { refused })
        .map_err(|error| map_relay_error(&error))
}
