//! MD-DISPLAY-04: caller-bound registration on the existing relay event route.
use super::subscriptions::{RelaySubscriptionTask, RelaySubscriptionTasks};
use super::*;
use chariox_relay::protocol::RelayCallerIdentity;
// MD-DISPLAY-04: delivery registration only; policy stays in the host service.
pub(super) struct DisplaySubscription {
    pub(super) request_id: String,
    pub(super) relay_id: String,
    pub(super) display_id: String,
    pub(super) generation: String,
    pub(super) identity: Option<RelayCallerIdentity>,
    pub(super) public_key: String,
    pub(super) resume: Option<u64>,
}

pub(super) async fn handle_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    tasks: &RelaySubscriptionTasks,
    subscription: DisplaySubscription,
) -> Result<(), DaemonError> {
    let DisplaySubscription {
        request_id,
        relay_id,
        display_id,
        generation,
        identity,
        public_key,
        resume,
    } = subscription;
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let request =
        crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::State,
        });
    let caller = KernelCommand::from_local_request_with_caller(
        request_id.clone(),
        KernelCommandSource::RelayClient,
        identity
            .map(KernelCaller::from_relay_identity)
            .unwrap_or_else(|| KernelCaller::for_source(&KernelCommandSource::RelayClient)),
        None,
        None,
        &request,
    );
    let admission = match generation.parse::<u64>() {
        Ok(generation) if resume.is_none() => {
            router
                .runtime_state()
                .kernel_browser_display_attach(&caller, display_id.clone(), generation)
                .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "kernel_browser",
            message: "MD-DISPLAY: fresh generation-bound subscription required".into(),
        }),
    };
    if let Err(error) = admission {
        return send_outgoing_envelope(
            outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: request_id,
                encrypted_response: None,
                error: Some(super::request_errors::map_relay_error(&error)),
            },
        );
    }
    let mut guard = tasks.lock().await;
    guard.retain(|_, task| task.display_id.is_none() || !task.handle.is_finished());
    let key = format!("display:{display_id}:{public_key}");
    if let Some(old) = guard.remove(&key) {
        old.handle.abort();
        if let Some(pump) = &old.display_pump {
            pump.stop();
        }
    }
    // MP-08/MP-10: protocol 475 push pump, dormant until the first ack.
    let generation = generation.parse::<u64>().unwrap_or_default();
    let pump = super::display_pump::DisplayPump::new(generation);
    guard.insert(
        key.clone(),
        RelaySubscriptionTask {
            display_id: Some(display_id.clone()),
            display_pump: Some(pump.clone()),
            relay_subscription_id: relay_id.clone(),
            client_public_key: public_key.clone(),
            handle: tokio::spawn(sleep(Duration::from_secs(60))),
        },
    );
    drop(guard);
    let request =
        crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::DisplayNext {
                subscription_id: display_id.clone(),
                generation,
                after_sequence: 0,
            },
        });
    let pump_caller = KernelCommand::from_local_request_with_caller(
        format!("{request_id}-push"),
        KernelCommandSource::RelayClient,
        caller.caller.clone(),
        None,
        None,
        &request,
    );
    tokio::spawn(super::display_pump::run(
        super::display_pump::PumpRoute {
            router: router.clone(),
            outgoing_tx: outgoing_tx.clone(),
            tasks: tasks.clone(),
            key,
            caller: pump_caller,
            display_id,
            relay_id,
            public_key: public_key.clone(),
        },
        pump,
    ));
    let encrypted = encrypt_json_response(
        router,
        &public_key,
        serde_json::json!({"ok":true,"display_attached":true}),
    )
    .await?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonResponse {
            relay_request_id: request_id,
            encrypted_response: Some(encrypted),
            error: None,
        },
    )
}

pub(super) async fn browser_display_delivery_id(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
) -> Option<String> {
    let mut guard = tasks.lock().await;
    let task = guard.values_mut().find(|task| {
        task.display_id.as_deref() == Some(display_id)
            && task.client_public_key == public_key
            && !task.handle.is_finished()
    })?;
    task.handle.abort();
    task.handle = tokio::spawn(sleep(Duration::from_secs(60)));
    Some(task.relay_subscription_id.clone())
}

/// MP-08/MP-10: protocol 475 acknowledgement from the subscription's own
/// sender key; renews its delivery lease and reports the pump state.
pub(super) async fn acknowledge(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
    generation: u64,
    sequence: u64,
    lost: bool,
) -> Option<&'static str> {
    let pump = {
        let guard = tasks.lock().await;
        guard
            .values()
            .find(|task| {
                task.display_id.as_deref() == Some(display_id)
                    && task.client_public_key == public_key
                    && !task.handle.is_finished()
            })?
            .display_pump
            .clone()?
    };
    let status = pump.acknowledge(generation, sequence, lost)?;
    browser_display_delivery_id(tasks, display_id, public_key).await?;
    Some(status)
}

pub(super) async fn refresh_admitted_display_poll(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
) -> bool {
    browser_display_delivery_id(tasks, display_id, public_key)
        .await
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn md_display_static_polling_renews_delivery_beyond_sixty_seconds() {
        let tasks = Arc::new(Mutex::new(std::collections::BTreeMap::from([(
            "display".into(),
            RelaySubscriptionTask {
                display_id: Some("d".into()),
                display_pump: None,
                relay_subscription_id: "relay".into(),
                client_public_key: "key".into(),
                handle: tokio::spawn(sleep(Duration::from_secs(60))),
            },
        )])));
        tokio::task::yield_now().await;
        for _ in 0..4 {
            tokio::time::advance(Duration::from_secs(20)).await;
            assert!(refresh_admitted_display_poll(&tasks, "d", "key").await);
            tokio::task::yield_now().await;
        }
        assert_eq!(
            browser_display_delivery_id(&tasks, "d", "key")
                .await
                .as_deref(),
            Some("relay")
        );
        assert!(browser_display_delivery_id(&tasks, "d", "foreign")
            .await
            .is_none());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(browser_display_delivery_id(&tasks, "d", "key")
            .await
            .is_none());
    }
}
