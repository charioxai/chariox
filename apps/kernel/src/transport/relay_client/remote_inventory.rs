//! Remote relay inventory projection refresh and liveness probing.

use std::future::Future;

use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::*;

const REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteInventoryRefreshResult {
    Refreshed,
    Failed(String),
    TimedOut,
}

pub(super) fn abort_inventory_refresh_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) fn clear_remote_inventory_projection(router: &CommandRouter) {
    router.clear_remote_relay_inventory_projection();
}

pub(super) fn spawn_remote_inventory_projection_refresh(
    router: Arc<CommandRouter>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        match bounded_remote_inventory_refresh(
            router.refresh_remote_relay_inventory_projection(),
            REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT,
        )
        .await
        {
            RemoteInventoryRefreshResult::Refreshed => {}
            RemoteInventoryRefreshResult::Failed(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "remote relay inventory refresh failed",
                    serde_json::json!({
                        "error": error,
                    }),
                );
            }
            RemoteInventoryRefreshResult::TimedOut => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "remote relay inventory refresh timed out",
                    serde_json::json!({
                        "timeout_ms": REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT.as_millis(),
                    }),
                );
            }
        }
    })
}

async fn bounded_remote_inventory_refresh<F, E>(
    refresh: F,
    refresh_timeout: Duration,
) -> RemoteInventoryRefreshResult
where
    F: Future<Output = Result<(), E>>,
    E: ToString,
{
    match timeout(refresh_timeout, refresh).await {
        Ok(Ok(())) => RemoteInventoryRefreshResult::Refreshed,
        Ok(Err(error)) => RemoteInventoryRefreshResult::Failed(error.to_string()),
        Err(_) => RemoteInventoryRefreshResult::TimedOut,
    }
}

pub(crate) async fn refresh_remote_inventory_projection(
    config_projection: DaemonConfigProjectionStore,
    projection: RemoteRelayInventoryProjectionStore,
) -> Result<(), DaemonError> {
    let mut config = config_projection.snapshot();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_remote_inventory_refresh_reports_success() {
        let result = bounded_remote_inventory_refresh(
            async { Ok::<(), &'static str>(()) },
            Duration::from_secs(1),
        )
        .await;

        assert_eq!(result, RemoteInventoryRefreshResult::Refreshed);
    }

    #[tokio::test]
    async fn bounded_remote_inventory_refresh_reports_failure() {
        let result = bounded_remote_inventory_refresh(
            async { Err::<(), &'static str>("relay unavailable") },
            Duration::from_secs(1),
        )
        .await;

        assert_eq!(
            result,
            RemoteInventoryRefreshResult::Failed("relay unavailable".to_string())
        );
    }

    #[tokio::test]
    async fn bounded_remote_inventory_refresh_times_out() {
        let result = bounded_remote_inventory_refresh(
            std::future::pending::<Result<(), &'static str>>(),
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, RemoteInventoryRefreshResult::TimedOut);
    }
}
