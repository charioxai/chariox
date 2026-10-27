use std::future::Future;
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;
use tokio::time::timeout;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{ListRemoteMachineKernelsRequest, LocalDaemonResponse};
use crate::runtime::projection::DaemonConfigProjectionStore;

const FRESH_REMOTE_RELAY_QUERY_TIMEOUT: Duration = Duration::from_secs(8);

/// Queries the relay registration directly. This response is a current scoped
/// observation only; it does not establish historical heartbeat-ID absence or
/// full MP-10 acceptance.
pub(crate) async fn execute_fresh_remote_machine_kernels_request(
    config_projection: DaemonConfigProjectionStore,
    request: ListRemoteMachineKernelsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    execute_fresh_remote_machine_kernels_with(
        config_projection.snapshot(),
        request,
        FRESH_REMOTE_RELAY_QUERY_TIMEOUT,
        |config, machine_ref| async move {
            crate::transport::relay_discovery::list_live_kernels_for_machine(&config, &machine_ref)
                .await
        },
    )
    .await
}

async fn execute_fresh_remote_machine_kernels_with<Q, F>(
    config: DaemonConfig,
    request: ListRemoteMachineKernelsRequest,
    query_timeout: Duration,
    query: Q,
) -> Result<LocalDaemonResponse, DaemonError>
where
    Q: FnOnce(DaemonConfig, String) -> F,
    F: Future<Output = Result<Vec<RelayKernelPresence>, DaemonError>>,
{
    let machine_ref = crate::local::provider_requests::resolve_registered_or_raw_machine_ref(
        &request.machine_ref,
    );
    if machine_ref.is_empty() {
        return Err(safe_query_error("machine_ref is required"));
    }

    let query_started_at_ms = crate::session::unix_epoch_ms();
    let query_result = timeout(query_timeout, query(config, machine_ref.clone())).await;
    let query_completed_at_ms = crate::session::unix_epoch_ms();
    let kernels = match query_result {
        Ok(Ok(kernels)) => kernels,
        Ok(Err(_)) => return Err(safe_query_error("fresh relay kernel observation failed")),
        Err(_) => return Err(safe_query_error("fresh relay kernel observation timed out")),
    };
    if kernels.iter().any(|kernel| {
        kernel.machine_id.as_str() != machine_ref.as_str()
            && kernel.machine_alias.as_deref() != Some(machine_ref.as_str())
            && kernel.relay_alias.as_deref() != Some(machine_ref.as_str())
    }) {
        return Err(safe_query_error(
            "fresh relay kernel observation was not machine-bound",
        ));
    }

    Ok(LocalDaemonResponse::FreshRemoteMachineKernelsObserved {
        machine_ref,
        query_started_at_ms,
        query_completed_at_ms,
        kernels,
    })
}

fn safe_query_error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "query fresh remote machine kernels",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_kernel() -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: "kernel-live-1".to_string(),
            machine_id: "mp10-fixture-machine".to_string(),
            machine_alias: Some("builder".to_string()),
            relay_alias: None,
            kernel_alias: Some("worker".to_string()),
            available_providers: vec!["codex".to_string()],
            provider_accounts: Vec::new(),
            capabilities: vec!["kernel_ws".to_string()],
            accepting_remote_leases: true,
            leased_agent_count: 1,
            local_session_count: 2,
            public_key: "fixture-public-key".to_string(),
        }
    }

    #[tokio::test]
    async fn empty_machine_binding_is_rejected_before_querying_the_relay() {
        let result = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: " \t ".to_string(),
            },
            Duration::from_secs(1),
            |_config, _machine_ref| async {
                panic!("an empty binding must not query the relay");
                #[allow(unreachable_code)]
                Ok(Vec::new())
            },
        )
        .await;
        let Err(DaemonError::LocalTransport { message, .. }) = result else {
            panic!("an empty binding must fail");
        };
        assert_eq!(message, "machine_ref is required");
    }

    #[tokio::test]
    async fn fresh_empty_registration_is_returned_without_a_cached_fallback() {
        let response = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: "mp10-fixture-machine".to_string(),
            },
            Duration::from_secs(1),
            |_config, _machine_ref| async { Ok(Vec::new()) },
        )
        .await
        .expect("a successful empty current registration is an observation");
        let LocalDaemonResponse::FreshRemoteMachineKernelsObserved { kernels, .. } = response
        else {
            panic!("an empty observation must retain the fresh response variant");
        };
        assert!(kernels.is_empty());
    }

    #[tokio::test]
    async fn fresh_query_accepts_machine_and_relay_alias_bindings() {
        for machine_ref in ["builder", "relay-worker"] {
            let response = execute_fresh_remote_machine_kernels_with(
                DaemonConfig::for_tests(),
                ListRemoteMachineKernelsRequest {
                    machine_ref: machine_ref.to_string(),
                },
                Duration::from_secs(1),
                |_config, _machine_ref| async {
                    let mut kernel = live_kernel();
                    kernel.relay_alias = Some("relay-worker".to_string());
                    Ok(vec![kernel])
                },
            )
            .await
            .expect("a matching registered alias must be accepted");
            let LocalDaemonResponse::FreshRemoteMachineKernelsObserved {
                machine_ref: observed,
                kernels,
                ..
            } = response
            else {
                panic!("a matching alias must retain the fresh response variant");
            };
            assert_eq!(observed, machine_ref);
            assert_eq!(kernels.len(), 1);
        }
    }

    #[tokio::test]
    async fn injected_relay_query_returns_fresh_scoped_kernels_and_timestamps() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("fixture-relay-token".to_string());
        config.relay_request_timeout_ms = 321;
        let expected_kernel = live_kernel();
        let response = execute_fresh_remote_machine_kernels_with(
            config,
            ListRemoteMachineKernelsRequest {
                machine_ref: "  mp10-fixture-machine  ".to_string(),
            },
            Duration::from_secs(1),
            move |config, machine_ref| async move {
                assert_eq!(machine_ref, "mp10-fixture-machine");
                assert_eq!(
                    config.relay_url.as_deref(),
                    Some("wss://relay.example.test")
                );
                assert_eq!(config.relay_token.as_deref(), Some("fixture-relay-token"));
                assert_eq!(config.relay_request_timeout_ms, 321);
                assert_eq!(machine_ref, expected_kernel.machine_id);
                Ok(vec![expected_kernel])
            },
        )
        .await
        .expect("fresh relay query should succeed");
        let encoded_response =
            serde_json::to_string(&response).expect("fresh response should encode");
        assert!(!encoded_response.contains("fixture-relay-token"));

        let LocalDaemonResponse::FreshRemoteMachineKernelsObserved {
            machine_ref,
            query_started_at_ms,
            query_completed_at_ms,
            kernels,
        } = response
        else {
            panic!("fresh query should use its distinct response variant");
        };
        assert_eq!(machine_ref, "mp10-fixture-machine");
        assert!(query_started_at_ms <= query_completed_at_ms);
        assert_eq!(kernels, vec![live_kernel()]);
    }

    #[tokio::test]
    async fn wrong_request_id_error_is_sanitized_and_never_returns_cached_data() {
        let result = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: "mp10-fixture-machine".to_string(),
            },
            Duration::from_secs(1),
            |_config, _machine_ref| async {
                Err(DaemonError::LocalTransport {
                    operation: "decode relay metadata response",
                    message: "response request_id mismatch: opaque-relay-payload".to_string(),
                })
            },
        )
        .await;

        let Err(DaemonError::LocalTransport { message, .. }) = result else {
            panic!("wrong request id must fail the fresh observation");
        };
        assert_eq!(message, "fresh relay kernel observation failed");
        assert!(!message.contains("opaque-relay-payload"));
    }

    #[tokio::test]
    async fn relay_error_payload_is_sanitized_without_projection_fallback() {
        let result = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: "mp10-fixture-machine".to_string(),
            },
            Duration::from_secs(1),
            |_config, _machine_ref| async {
                Err(DaemonError::LocalTransport {
                    operation: "list_remote_machine_kernels",
                    message: "opaque relay error and credential-like payload".to_string(),
                })
            },
        )
        .await;

        let Err(DaemonError::LocalTransport { message, .. }) = result else {
            panic!("relay errors must fail the fresh observation");
        };
        assert_eq!(message, "fresh relay kernel observation failed");
        assert!(!message.contains("opaque relay error"));
        assert!(!message.contains("credential-like payload"));
    }

    #[tokio::test]
    async fn relay_query_timeout_fails_without_returning_projected_data() {
        let result = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: "mp10-fixture-machine".to_string(),
            },
            Duration::from_millis(1),
            |_config, _machine_ref| async {
                std::future::pending::<Result<Vec<RelayKernelPresence>, DaemonError>>().await
            },
        )
        .await;

        let Err(DaemonError::LocalTransport { message, .. }) = result else {
            panic!("a stalled relay query must time out");
        };
        assert_eq!(message, "fresh relay kernel observation timed out");
    }

    #[tokio::test]
    async fn fresh_query_rejects_kernels_outside_the_resolved_machine_binding() {
        let result = execute_fresh_remote_machine_kernels_with(
            DaemonConfig::for_tests(),
            ListRemoteMachineKernelsRequest {
                machine_ref: "mp10-fixture-machine".to_string(),
            },
            Duration::from_secs(1),
            |_config, _machine_ref| async {
                let mut unrelated = live_kernel();
                unrelated.machine_id = "another-machine".to_string();
                unrelated.machine_alias = None;
                unrelated.relay_alias = None;
                Ok(vec![unrelated])
            },
        )
        .await;

        let Err(DaemonError::LocalTransport { message, .. }) = result else {
            panic!("kernels outside the requested machine must be rejected");
        };
        assert_eq!(
            message,
            "fresh relay kernel observation was not machine-bound"
        );
    }
}
