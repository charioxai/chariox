//! Remote relay inventory projection refresh and liveness probing.

use super::*;

pub(super) fn abort_inventory_refresh_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) async fn clear_remote_inventory_projection(app: &Arc<Mutex<DaemonApp>>) {
    let projection = {
        let app = app.lock().await;
        app.remote_relay_inventory_projection_store()
    };
    projection.clear();
}

pub(super) fn spawn_remote_inventory_projection_refresh(
    app: Arc<Mutex<DaemonApp>>,
    _state: Arc<RwLock<RelayClientState>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = refresh_remote_inventory_projection_for_app_with_relay_state(&app).await
        {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "remote relay inventory refresh failed",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    })
}

pub(crate) async fn refresh_remote_inventory_projection_for_app_with_relay_state(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<(), DaemonError> {
    let (mut config, projection) = {
        let app = app.lock().await;
        (
            app.config().clone(),
            app.remote_relay_inventory_projection_store(),
        )
    };
    if config.relay_url.is_none() || config.relay_token.is_none() {
        projection.clear();
        return Ok(());
    }
    config.relay_request_timeout_ms = config
        .relay_request_timeout_ms
        .min(REMOTE_INVENTORY_RELAY_TIMEOUT_MS);

    let live_machines = relay_discovery::list_live_machines(&config).await?;
    let mut remote_machines = crate::local::provider_requests::remote_machine_records(
        live_machines,
        &config.host_machine_id,
    );
    let (_, previous_kernels) = projection.snapshot();
    let known_kernel_ids = previous_kernels
        .into_iter()
        .map(|kernel| kernel.kernel_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut remote_kernels = Vec::new();
    for machine in remote_machines
        .iter()
        .filter(|machine| machine.online && machine.kernel_count > 0)
    {
        let kernels =
            relay_discovery::list_live_kernels_for_machine(&config, &machine.machine_id).await?;
        remote_kernels
            .extend(validate_live_relay_kernels(&config, &known_kernel_ids, kernels).await);
    }
    for machine in &mut remote_machines {
        machine.kernel_count = remote_kernels
            .iter()
            .filter(|kernel| kernel.machine_id == machine.machine_id)
            .count();
    }
    projection.update(remote_machines, remote_kernels);
    Ok(())
}

async fn validate_live_relay_kernels(
    config: &crate::config::DaemonConfig,
    known_kernel_ids: &std::collections::BTreeSet<String>,
    kernels: Vec<arroba_relay::protocol::RelayKernelPresence>,
) -> Vec<arroba_relay::protocol::RelayKernelPresence> {
    let mut validated = Vec::new();
    let mut probe_config = config.clone();
    probe_config.relay_request_timeout_ms = probe_config
        .relay_request_timeout_ms
        .min(REMOTE_INVENTORY_KERNEL_PROBE_TIMEOUT_MS);
    for kernel in kernels {
        if !known_kernel_ids.contains(&kernel.kernel_id) {
            validated.push(kernel);
            continue;
        }
        let target = ClientTarget {
            daemon_id: Some(kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        let reachable = matches!(
            send_peer_request_via_temporary_connection(
                &probe_config,
                target,
                RelayPeerRequest::Ping {
                    value: "inventory-probe".to_string(),
                },
            )
            .await,
            Ok(RelayPeerResponse::Pong { .. })
        );
        if reachable {
            validated.push(kernel);
        }
    }
    validated
}
