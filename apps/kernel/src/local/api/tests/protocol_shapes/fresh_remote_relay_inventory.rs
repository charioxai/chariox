use super::*;

#[test]
fn local_daemon_protocol_fresh_relay_kernel_observation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 346);

    let request =
        LocalDaemonRequest::QueryFreshRemoteMachineKernels(ListRemoteMachineKernelsRequest {
            machine_ref: "machine-1".to_string(),
        });
    assert_eq!(
        serde_json::to_value(request).expect("fresh inventory request should encode"),
        serde_json::json!({
            "QueryFreshRemoteMachineKernels": { "machine_ref": "machine-1" }
        })
    );

    let response = LocalDaemonResponse::FreshRemoteMachineKernelsObserved {
        machine_ref: "machine-1".to_string(),
        query_started_at_ms: 100,
        query_completed_at_ms: 125,
        kernels: vec![RelayKernelPresence {
            kernel_id: "kernel-1".to_string(),
            machine_id: "machine-1".to_string(),
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
        }],
    };
    assert_eq!(
        serde_json::to_value(response).expect("fresh inventory response should encode"),
        serde_json::json!({
            "FreshRemoteMachineKernelsObserved": {
                "machine_ref": "machine-1",
                "query_started_at_ms": 100,
                "query_completed_at_ms": 125,
                "kernels": [{
                    "kernel_id": "kernel-1",
                    "machine_id": "machine-1",
                    "machine_alias": "builder",
                    "relay_alias": null,
                    "kernel_alias": "worker",
                    "available_providers": ["codex"],
                    "provider_accounts": [],
                    "capabilities": ["kernel_ws"],
                    "accepting_remote_leases": true,
                    "leased_agent_count": 1,
                    "local_session_count": 2,
                    "public_key": "fixture-public-key"
                }]
            }
        })
    );
}
