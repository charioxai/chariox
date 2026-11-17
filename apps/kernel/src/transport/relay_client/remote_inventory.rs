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

/// The machine list is kept warm for the clients that subscribed to it; with none attached, a
/// request for it refreshes it on demand, so an idle kernel does not ask Cloud and the relay for it
/// every 500 s. The first client to subscribe finds the refresh already started.
pub(super) fn inventory_refresh_wanted(
    attached: bool,
    was_attached: bool,
    due: bool,
    no_refresh_running: bool,
) -> bool {
    attached && (due || (!was_attached && no_refresh_running))
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
        run_remote_inventory_refresh(router.refresh_remote_relay_inventory_projection()).await
    })
}

async fn run_remote_inventory_refresh<F, E>(refresh: F) -> bool
where
    F: Future<Output = Result<(), E>>,
    E: ToString,
{
    match bounded_remote_inventory_refresh(refresh, REMOTE_INVENTORY_PROJECTION_REFRESH_TIMEOUT)
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
            // Cloud refused with "retry in N seconds": the next heartbeat would only be
            // refused again, so this refresh keeps its slot until then.
            if let Some(wait) = cloud_retry_hint(&error) {
                tokio::time::sleep(wait).await;
            }
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

/// Renews the kernel list over the relay connection `relay_state` holds; with none (first
/// start, relay down) it asks over a socket of its own.
pub(crate) async fn refresh_remote_inventory_projection(
    config_projection: DaemonConfigProjectionStore,
    projection: RemoteRelayInventoryProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
) -> Result<(), DaemonError> {
    let main = relay_state.map(MainConnection::new);
    let main = main.as_ref();
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
    let live_machines = relay_discovery::list_live_machines_over(&runtime_config, main).await?;
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
        let kernels = relay_discovery::list_live_kernels_for_machine_over(
            &runtime_config,
            main,
            &machine.machine_id,
        )
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
    let remote_machines = listed_remote_machines(
        remote_machines,
        &runtime_config.host_machine_id,
        visible_kernel_ids.is_some(),
    );
    projection.update(remote_machines, remote_kernels);
    Ok(())
}

/// A machine is listed for the kernels a client can reach on it. This kernel is "local" to its
/// clients, so its own machine is listed only while another kernel runs there; with Cloud
/// visibility, a machine without a visible kernel is not listed at all.
fn listed_remote_machines(
    mut machines: Vec<crate::local::RemoteMachineRecord>,
    host_machine_id: &str,
    cloud_visibility: bool,
) -> Vec<crate::local::RemoteMachineRecord> {
    machines.retain(|machine| {
        machine.kernel_count > 0 || !(cloud_visibility || machine.machine_id == host_machine_id)
    });
    machines
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

    #[test]
    fn an_unattended_kernel_does_not_refresh_the_machine_list_on_a_timer() {
        // No client: neither the 500 s tick nor the retry after a failure asks for it.
        assert!(!inventory_refresh_wanted(false, false, true, true));
        assert!(!inventory_refresh_wanted(false, true, true, true));
        // The first subscriber starts it at once; later ones follow the timer.
        assert!(inventory_refresh_wanted(true, false, false, true));
        assert!(!inventory_refresh_wanted(true, false, false, false));
        assert!(!inventory_refresh_wanted(true, true, false, true));
        assert!(inventory_refresh_wanted(true, true, true, true));
    }

    // MP-08 / MP-11: the home's own machine is "local" in every client; without another kernel on
    // it, it must not also appear as a remote machine with no kernels, whether or not Cloud is used.
    #[test]
    fn the_own_machine_is_listed_only_while_another_kernel_runs_on_it() {
        let machine = |machine_id: &str, kernel_count| crate::local::RemoteMachineRecord {
            machine_id: machine_id.into(),
            machine_alias: None,
            display_name: machine_id.into(),
            trust_status: crate::local::RemoteMachineTrustStatus::Approved,
            online: true,
            pending: false,
            kernel_count,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
        };
        let ids = |machines: Vec<crate::local::RemoteMachineRecord>| {
            machines
                .into_iter()
                .map(|machine| machine.machine_id)
                .collect::<Vec<_>>()
        };
        let listed = |cloud_visibility| {
            ids(listed_remote_machines(
                vec![machine("home", 0), machine("other", 0), machine("busy", 2)],
                "home",
                cloud_visibility,
            ))
        };
        assert_eq!(listed(false), ["other", "busy"]);
        assert_eq!(listed(true), ["busy"]);
        assert_eq!(
            ids(listed_remote_machines(
                vec![machine("home", 1)],
                "home",
                false
            )),
            ["home"],
            "a sibling kernel keeps the own machine reachable"
        );
    }

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
                None,
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

    // Cloud refused the inventory read with "retry in 20 seconds": the refresh holds its
    // slot until then, so the 5-second heartbeat does not ask again inside that window.
    #[tokio::test(start_paused = true)]
    async fn a_rate_limited_inventory_refresh_waits_out_the_cloud_retry_hint() {
        let mut task = Some(tokio::spawn(run_remote_inventory_refresh(async {
            Err::<(), _>("cloud relay request failed with 429: cloud_api_code=rate_limited: Rate limit exceeded, retry in 20 seconds")
        })));
        tokio::time::sleep(Duration::from_secs(19)).await;
        assert!(
            !remote_inventory_refresh_due(1, &mut task),
            "the next heartbeat asked Cloud again inside its retry window"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert!(remote_inventory_refresh_due(1, &mut task));
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
