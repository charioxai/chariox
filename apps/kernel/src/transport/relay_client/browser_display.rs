//! MD-DISPLAY-04: caller-bound registration on the existing relay event route.
use super::subscriptions::{RelaySubscriptionTask, RelaySubscriptionTasks};
use super::*;
use chariox_relay::protocol::RelayCallerIdentity;
// MD-DISPLAY-04: delivery registration only; policy stays in the host service.
pub(super) async fn handle_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    tasks: &RelaySubscriptionTasks,
    request_id: String,
    relay_id: String,
    display_id: String,
    generation: String,
    identity: Option<RelayCallerIdentity>,
    public_key: String,
    resume: Option<u64>,
) -> Result<(), DaemonError> {
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
    }
    guard.insert(
        key,
        RelaySubscriptionTask {
            display_id: Some(display_id),
            relay_subscription_id: relay_id,
            client_public_key: public_key.clone(),
            handle: tokio::spawn(sleep(Duration::from_secs(60))),
        },
    );
    drop(guard);
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
