//! Remote relay inventory projection refresh and liveness probing.

use std::future::Future;

use futures_util::FutureExt;

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

/// The periodic refresh is slow on purpose; a failed one (Cloud unreachable
/// after an outage) retries on the next heartbeat so the waiting room's
/// machine list recovers within seconds.
pub(super) fn remote_inventory_refresh_due(
    heartbeat_tick: u64,
    task: &mut Option<JoinHandle<bool>>,
) -> bool {
    let last_refreshed = match task.as_mut() {
        Some(running) if !running.is_finished() => return false,
        Some(finished) => matches!(finished.now_or_never(), Some(Ok(true))),
        None => true,
    };
    *task = None;
    !last_refreshed || heartbeat_tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS)
}

pub(super) fn abort_inventory_refresh_task(task: &mut Option<JoinHandle<bool>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) fn clear_remote_inventory_projection(router: &CommandRouter) {
    router.clear_remote_relay_inventory_projection();
}

pub(super) fn spawn_remote_inventory_projection_refresh(
    router: Arc<CommandRouter>,
) -> JoinHandle<bool> {
    tokio::spawn(async move {
        match bounded_remote_inventory_refresh(
            router.refresh_remote_relay_inventory_projection(),
            REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT,
        )
        .await
        {
            RemoteInventoryRefreshResult::Refreshed => true,
            RemoteInventoryRefreshResult::Failed(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "remote relay inventory refresh failed",
                    serde_json::json!({
                        "error": error,
                    }),
                );
                false
            }
            RemoteInventoryRefreshResult::TimedOut => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "remote relay inventory refresh timed out",
                    serde_json::json!({
                        "timeout_ms": REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT.as_millis(),
                    }),
                );
                false
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
    let mut runtime_config = config_projection.snapshot();
    if runtime_config.relay_url.is_none() || runtime_config.relay_token.is_none() {
        projection.clear();
        return Ok(());
    }
    runtime_config.relay_request_timeout_ms = runtime_config
        .relay_request_timeout_ms
        .min(REMOTE_INVENTORY_RELAY_TIMEOUT_MS);

    let visible_kernel_ids = if let Some(profile) =
        crate::runtime::cloud_relay_control::active_cloud_profile(&runtime_config)
    {
        relay_discovery::authorization::visible_targets(profile).await?
    } else {
        None
    };
    let live_machines = relay_discovery::list_live_machines(&runtime_config).await?;
    let mut remote_machines = crate::local::provider_requests::remote_machine_records(
        live_machines,
        &runtime_config.host_machine_id,
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
            relay_discovery::list_live_kernels_for_machine(&runtime_config, &machine.machine_id)
                .await?;
        let kernels = kernels
            .into_iter()
            .filter(|k| {
                visible_kernel_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&k.kernel_id))
            })
            .collect();
        remote_kernels
            .extend(validate_live_relay_kernels(&runtime_config, &known_kernel_ids, kernels).await);
    }
    for machine in &mut remote_machines {
        machine.kernel_count = remote_kernels
            .iter()
            .filter(|kernel| kernel.machine_id == machine.machine_id)
            .count();
    }
    if visible_kernel_ids.is_some() {
        remote_machines.retain(|machine| machine.kernel_count > 0);
    }
    projection.update(remote_machines, remote_kernels);
    Ok(())
}

async fn validate_live_relay_kernels(
    config: &crate::config::DaemonConfig,
    known_kernel_ids: &std::collections::BTreeSet<String>,
    kernels: Vec<chariox_relay::protocol::RelayKernelPresence>,
) -> Vec<chariox_relay::protocol::RelayKernelPresence> {
    let mut validated = Vec::new();
    let mut probe_config = config.clone();
    probe_config.relay_request_timeout_ms = probe_config
        .relay_request_timeout_ms
        .min(REMOTE_INVENTORY_KERNEL_PROBE_TIMEOUT_MS);
    for kernel in kernels {
        if kernel.kernel_id == config.daemon_id {
            continue;
        }
        let is_known = known_kernel_ids.contains(&kernel.kernel_id);
        if !requires_peer_probe(is_known, config) {
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

fn requires_peer_probe(is_known: bool, config: &crate::config::DaemonConfig) -> bool {
    // Hosted inventory is already filtered by heartbeat freshness. Avoid a
    // redundant temporary peer admission for the same inventory refresh.
    is_known && !crate::runtime::cloud_relay_control::cloud_relay_transport_selected(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    // MP-08 / MP-10 / MP-11: a private relay's inventory is read with its own
    // token; a retained Cloud profile is not asked which targets are visible.
    #[tokio::test]
    async fn private_relay_inventory_refresh_does_not_contact_cloud() {
        let cloud = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        cloud.set_nonblocking(true).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:1".into());
        config.relay_token = Some("synthetic-private-transport".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: format!("http://{}", cloud.local_addr().unwrap()),
            relay_url: "wss://cloud-relay".into(),
            kernel_credential: Some("synthetic-enrollment".into()),
            ..Default::default()
        });
        let refreshed = timeout(
            Duration::from_secs(3),
            refresh_remote_inventory_projection(
                DaemonConfigProjectionStore::new(config),
                RemoteRelayInventoryProjectionStore::default(),
            ),
        )
        .await;
        assert!(
            matches!(refreshed, Ok(Err(_))),
            "the unreachable private relay is the failure: {refreshed:?}"
        );
        assert!(
            cloud.accept().is_err(),
            "private inventory refresh must not reach the Cloud API"
        );
    }

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

    #[test]
    fn hosted_known_kernels_use_fresh_relay_inventory_without_a_peer_probe() {
        let cloud = crate::config::PersistedCloudRelayProfile {
            relay_url: "wss://cloud-relay".into(),
            ..Default::default()
        };
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(cloud);
        config.relay_url = Some("wss://cloud-relay".into());
        assert!(!requires_peer_probe(true, &config));
        assert!(!requires_peer_probe(false, &config));
        // A private relay with a retained Cloud profile has no Cloud freshness filter.
        config.relay_url = Some("ws://operator-relay:49100".into());
        assert!(requires_peer_probe(true, &config));
        assert!(!requires_peer_probe(false, &config));
        config.cloud_relay = None;
        assert!(requires_peer_probe(true, &config));
    }
}
