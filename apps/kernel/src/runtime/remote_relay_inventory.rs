use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{
    ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest, LocalDaemonRequest,
    LocalDaemonResponse, RelayStatus,
};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::transport::relay_client::{refresh_remote_inventory_projection, RelayClientState};

const REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS: u64 = 1_000;
/// How long an answer on a stale machine list waits for its renewal.
const STALE_LIST_RENEWAL_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) async fn projected_remote_machines_response(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    // A kernel no client follows keeps the list it made when it connected: a client that just
    // made it the source of its machine list would miss every machine that joined since.
    // Connected, a stale list is renewed before the answer; a renewal slower than the budget
    // goes on, and the answer is the last list marked as being renewed.
    let stale_after_ms = config_projection
        .snapshot()
        .relay_heartbeat_ms
        .saturating_mul(2)
        .max(1_000);
    let now_ms = crate::session::unix_epoch_ms();
    let stale = !remote_relay_inventory_projection.is_fresh(now_ms, stale_after_ms);
    let renewing = if stale && relay_state.read().await.connected() {
        let projection = remote_relay_inventory_projection.clone();
        renewed_machine_list(
            &remote_relay_inventory_projection,
            now_ms,
            stale_after_ms,
            move || {
                refresh_remote_inventory_projection(
                    config_projection,
                    projection,
                    Some(relay_state),
                )
            },
            STALE_LIST_RENEWAL_WAIT,
        )
        .await?
    } else {
        request_remote_relay_inventory_projection_refresh(
            relay_state,
            config_projection,
            remote_relay_inventory_projection.clone(),
        )
        .await;
        false
    };
    let crate::local::RemoteMachineInventory {
        machines,
        offline_machines,
    } = remote_relay_inventory_projection.inventory();
    Ok(LocalDaemonResponse::RemoteMachinesListed {
        renewing,
        machines,
        offline_machines,
    })
}

/// Renews the machine list before an answer, waiting at most `budget`: Ok(false) when the renewed
/// list is in the projection, Ok(true) when the answer is the last list because the renewal still
/// runs (it goes on and publishes), and the failure with its reason when the renewal failed. A
/// request takes a renewal that started within its freshness bound (`now_ms - stale_after_ms`);
/// otherwise it starts one (`machine_list_renewal`).
async fn renewed_machine_list<F>(
    projection: &RemoteRelayInventoryProjectionStore,
    now_ms: u64,
    stale_after_ms: u64,
    renew: impl FnOnce() -> F + Send + 'static,
    budget: std::time::Duration,
) -> Result<bool, DaemonError>
where
    F: std::future::Future<Output = Result<(), DaemonError>> + Send + 'static,
{
    let wait = projection.renewal().request(
        now_ms.saturating_sub(stale_after_ms),
        now_ms,
        move || async move { renew().await.map_err(|error| error.to_string()) },
    );
    match crate::runtime::projection::machine_list_renewal::outcome_within(wait, budget).await {
        None => Ok(true),
        Some(Ok(())) => Ok(false),
        Some(Err(reason)) => Err(DaemonError::LocalTransport {
            operation: "renew the machine list",
            message: format!(
                "the machine list could not be renewed ({reason}); the last list is stale, ask again"
            ),
        }),
    }
}

pub(crate) async fn execute_list_remote_machines_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    _request: ListRemoteMachinesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    projected_remote_machines_response(
        relay_state,
        config_projection,
        remote_relay_inventory_projection,
    )
    .await
}

pub(crate) async fn projected_remote_machine_kernels_response(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    machine_ref: String,
) -> Result<LocalDaemonResponse, DaemonError> {
    request_remote_relay_inventory_projection_refresh(
        relay_state,
        config_projection,
        remote_relay_inventory_projection.clone(),
    )
    .await;
    let machine_ref =
        crate::local::provider_requests::resolve_registered_or_raw_machine_ref(&machine_ref);
    let (_, kernels) = remote_relay_inventory_projection.snapshot();
    let kernels = kernels
        .into_iter()
        .filter(|kernel| {
            kernel.machine_id == machine_ref
                || kernel.machine_alias.as_deref() == Some(machine_ref.as_str())
                || kernel.relay_alias.as_deref() == Some(machine_ref.as_str())
                || kernel.kernel_alias.as_deref() == Some(machine_ref.as_str())
        })
        .collect();
    Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
        machine_ref,
        kernels,
    })
}

pub(crate) async fn execute_list_remote_machine_kernels_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: ListRemoteMachineKernelsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    projected_remote_machine_kernels_response(
        relay_state,
        config_projection,
        remote_relay_inventory_projection,
        request.machine_ref,
    )
    .await
}

pub(crate) async fn execute_remote_relay_inventory_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListRemoteMachines(request) => {
            execute_list_remote_machines_request(
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::ListRemoteMachineKernels(request) => {
            execute_list_remote_machine_kernels_request(
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::QueryFreshRemoteMachineKernels(request) => {
            crate::runtime::fresh_remote_relay_inventory::execute_fresh_remote_machine_kernels_request(
                config_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "remote relay inventory request",
            message: "unsupported remote relay inventory request".to_string(),
        }),
    }
}

pub(crate) async fn projected_relay_status(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> RelayStatus {
    let config = config_projection.snapshot();
    // A configuration switch does not move the existing socket to the new relay.
    let connected = {
        let state = relay_state.read().await;
        state.connected()
            && config.relay_url.is_some()
            && state.connected_relay_url() == config.relay_url
    };
    let machine_alias = config.machine_name();
    RelayStatus {
        local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        runtime_process_identity: crate::runtime::kernel_process_identity::current(),
        capabilities: crate::local::RUNTIME_CONTROL_CAPABILITIES
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        configured: config.relay_url.is_some() && config.relay_token.is_some(),
        connected,
        relay_url: config.relay_url,
        relay_token_configured: config.relay_token.is_some(),
        daemon_id: config.daemon_id,
        machine_id: config.host_machine_id,
        machine_alias,
    }
}

async fn request_remote_relay_inventory_projection_refresh(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
) {
    let connected = relay_state.read().await.connected();
    if !connected {
        return;
    }
    let config = config_projection.snapshot();
    let now_ms = crate::session::unix_epoch_ms();
    let stale_after_ms = (config.relay_heartbeat_ms.saturating_mul(2)).max(1_000);
    if !remote_relay_inventory_projection.should_request_refresh(
        now_ms,
        stale_after_ms,
        REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS,
    ) {
        return;
    }
    tokio::spawn(async move {
        if let Err(error) = refresh_remote_inventory_projection(
            config_projection,
            remote_relay_inventory_projection,
            None,
        )
        .await
        {
            crate::logging::warn_with_fields(
                "daemon.router",
                "remote relay inventory refresh on demand failed",
                serde_json::json!({
                    "error": error.to_string(),
                    "stale_after_ms": stale_after_ms,
                    "cooldown_ms": REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS,
                }),
            );
        }
    });
}

#[cfg(test)]
mod renewal_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn machine(machine_id: &str) -> crate::local::RemoteMachineRecord {
        crate::local::RemoteMachineRecord {
            directory: None,
            machine_id: machine_id.into(),
            machine_alias: None,
            display_name: machine_id.into(),
            trust_status: crate::local::RemoteMachineTrustStatus::Approved,
            presence: crate::local::RemoteMachinePresence::Online,
            pending: false,
            kernel_count: 1,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            kernels: Vec::new(),
        }
    }

    // #1474 review 5968553263 P2-2: a pass slower than the answer budget is not cancelled by it.
    // The answer is the last list, marked as being renewed; a retry waits for the same pass and
    // gets its list.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pass_slower_than_the_answer_budget_runs_on_and_a_retry_gets_its_list() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            crate::local::RemoteMachineInventory {
                machines: vec![machine("known")],
                ..Default::default()
            },
            Vec::new(),
        );
        let started = Arc::new(AtomicUsize::new(0));
        let pass = || {
            let projection = projection.clone();
            let started = Arc::clone(&started);
            move || {
                started.fetch_add(1, Ordering::SeqCst);
                async move {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    projection.update(
                        crate::local::RemoteMachineInventory {
                            machines: vec![machine("known"), machine("joined")],
                            ..Default::default()
                        },
                        Vec::new(),
                    );
                    Ok(())
                }
            }
        };
        let renewing =
            renewed_machine_list(&projection, 1_000, 1_000, pass(), Duration::from_millis(50))
                .await
                .expect("answers");
        assert!(renewing, "the answer says the list is being renewed");
        assert_eq!(projection.inventory().machines.len(), 1, "the last list");

        let renewing =
            renewed_machine_list(&projection, 1_100, 1_000, pass(), Duration::from_secs(5))
                .await
                .expect("answers");
        assert!(!renewing);
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "the retry waited for the pass already running"
        );
        assert_eq!(
            projection
                .inventory()
                .machines
                .iter()
                .map(|machine| machine.machine_id.as_str())
                .collect::<Vec<_>>(),
            vec!["known", "joined"],
            "the pass that outlived the first answer finished"
        );
    }
}

#[cfg(test)]
mod renewal_findings {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    // #1474 review 5969448770 P2-2: a renewal that fails within the budget is a failure with its
    // reason, never a successful answer of the stale list.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_renewal_that_fails_answers_with_its_failure() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        let answer = renewed_machine_list(
            &projection,
            1_000,
            1_000,
            || async {
                Err(DaemonError::LocalTransport {
                    operation: "relay metadata",
                    message: "the relay metadata query timed out".into(),
                })
            },
            Duration::from_secs(5),
        )
        .await;
        let error = answer.expect_err("a failed renewal is not a successful list");
        assert!(
            error
                .to_string()
                .contains("the relay metadata query timed out"),
            "{error}"
        );
    }

    // #1474 review 5969448770 P2-1: a pass whose waiter timed out finishes in the background;
    // after another stale interval a new request starts a new pass, not the finished one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_renewal_is_not_reused_once_its_list_went_stale() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        let started = Arc::new(AtomicUsize::new(0));
        let pass = |delay_ms: u64| {
            let started = Arc::clone(&started);
            move || {
                started.fetch_add(1, Ordering::SeqCst);
                async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    Ok(())
                }
            }
        };
        let renewing = renewed_machine_list(
            &projection,
            1_000,
            1_000,
            pass(200),
            Duration::from_millis(20),
        )
        .await
        .expect("answers");
        assert!(renewing, "the waiter timed out");
        tokio::time::sleep(Duration::from_millis(400)).await;
        // Another stale interval later (the first pass started at 1_000; the bound is 4_000).
        let renewing =
            renewed_machine_list(&projection, 5_000, 1_000, pass(0), Duration::from_secs(5))
                .await
                .expect("answers");
        assert!(!renewing);
        assert_eq!(
            started.load(Ordering::SeqCst),
            2,
            "the stale finished pass was not reused: a new one ran"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;

    #[tokio::test]
    async fn relay_status_does_not_admit_the_previous_relay_after_a_switch() {
        let previous_url = "wss://previous-relay.example";
        let selected_url = "wss://selected-relay.example";
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(selected_url.into());
        config.relay_token = Some("selected-relay-token".into());
        let mut state = RelayClientState::default();
        let (outgoing, _priority, _events) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        state.test_set_connected_sender(outgoing.clone(), previous_url);
        let state = Arc::new(RwLock::new(state));
        let projection = DaemonConfigProjectionStore::new(config);

        let pending = projected_relay_status(state.clone(), projection.clone()).await;
        assert_eq!(pending.relay_url.as_deref(), Some(selected_url));
        assert!(pending.configured);
        assert!(
            !pending.connected,
            "the previous relay cannot admit a browser to the selected relay"
        );

        state
            .write()
            .await
            .test_set_connected_sender(outgoing, selected_url);
        assert!(projected_relay_status(state, projection).await.connected);
    }

    #[tokio::test]
    async fn relay_status_projects_config_and_connection_state() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example".to_string());
        config.relay_token = Some("token".to_string());
        config.daemon_id = "daemon-1".to_string();
        config.host_machine_id = "machine-1".to_string();
        config.machine_name_for_tests = Some("laptop".to_string());

        let status = projected_relay_status(
            Arc::new(RwLock::new(RelayClientState::default())),
            DaemonConfigProjectionStore::new(config),
        )
        .await;

        assert!(status.configured);
        assert!(!status.connected);
        assert_eq!(status.relay_url.as_deref(), Some("wss://relay.example"));
        assert!(status.relay_token_configured);
        assert_eq!(status.daemon_id, "daemon-1");
        assert_eq!(status.machine_id, "machine-1");
        assert_eq!(status.machine_alias.as_deref(), Some("laptop"));
    }
}
