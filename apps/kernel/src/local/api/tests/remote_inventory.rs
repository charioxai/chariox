use super::*;

#[test]
fn local_request_api_lists_live_remote_machines_and_kernels() {
    let config = DaemonConfig::for_tests();
    let host_machine_id = config.host_machine_id.clone();
    let harness = LocalRouterTestHarness::with_config(config);
    harness.with_app_mut(|app| {
        app.remote_relay_inventory_projection_store().update(
            crate::local::provider_requests::remote_machine_records(
                vec![RelayMachinePresence {
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("workstation".to_string()),
                    kernel_count: 1,
                    available_providers: vec!["codex".to_string(), "opencode".to_string()],
                }],
                &host_machine_id,
            ),
            vec![RelayKernelPresence {
                kernel_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                relay_alias: Some("mbp".to_string()),
                kernel_alias: Some("default".to_string()),
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 2,
                local_session_count: 3,
                public_key: "public-key".to_string(),
            }],
        );
    });

    let machines = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachines(
            ListRemoteMachinesRequest,
        ))
        .expect("remote machines request should succeed")
    {
        LocalDaemonResponse::RemoteMachinesListed { machines } => machines,
        other => panic!("unexpected response: {other:?}"),
    };
    let machine = machines
        .iter()
        .find(|machine| machine.machine_id == "machine-1")
        .expect("registered machine should be listed");
    assert_eq!(machine.machine_alias.as_deref(), Some("workstation"));
    assert_eq!(machine.display_name, "workstation");
    assert_eq!(machine.available_providers, vec!["codex", "opencode"]);

    let kernels = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachineKernels(
            ListRemoteMachineKernelsRequest {
                machine_ref: "workstation".to_string(),
            },
        ))
        .expect("remote machine kernels request should succeed")
    {
        LocalDaemonResponse::RemoteMachineKernelsListed { kernels, .. } => kernels,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(kernels.len(), 1);
    assert_eq!(kernels[0].kernel_id, "daemon-1");
    assert_eq!(kernels[0].available_providers, vec!["codex", "opencode"]);
    assert!(kernels[0].accepting_remote_leases);
}
