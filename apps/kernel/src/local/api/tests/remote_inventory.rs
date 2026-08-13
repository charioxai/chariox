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
                    provider_accounts: vec![chariox_relay::protocol::RelayProviderAccountSummary {
                        provider: "codex".to_string(),
                        state: "configured".to_string(),
                        auth_type: Some("chatgpt".to_string()),
                        account_id: Some("acct-remote-1".to_string()),
                        email: None,
                        organization_id: None,
                        organization_name: None,
                        subscription_type: None,
                        alias: Some("remote-codex".to_string()),
                    }],
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
                provider_accounts: vec![chariox_relay::protocol::RelayProviderAccountSummary {
                    provider: "codex".to_string(),
                    state: "configured".to_string(),
                    auth_type: Some("chatgpt".to_string()),
                    account_id: Some("acct-remote-1".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    alias: Some("remote-codex".to_string()),
                }],
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
    assert_eq!(
        machine.provider_accounts[0].alias.as_deref(),
        Some("remote-codex")
    );

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
    assert_eq!(
        kernels[0].provider_accounts[0].account_id.as_deref(),
        Some("acct-remote-1")
    );
    assert!(kernels[0].accepting_remote_leases);
}

#[test]
fn local_request_api_resolves_self_hosted_kernel_client_connection_from_inventory() {
    let mut config = DaemonConfig::for_tests();
    let host_machine_id = config.host_machine_id.clone();
    config.relay_url = Some("ws://relay.local".to_string());
    config.relay_token = Some("shared-token".to_string());
    let harness = LocalRouterTestHarness::with_config(config);
    harness.with_app_mut(|app| {
        app.remote_relay_inventory_projection_store().update(
            crate::local::provider_requests::remote_machine_records(
                vec![RelayMachinePresence {
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("workstation".to_string()),
                    kernel_count: 1,
                    available_providers: vec!["opencode".to_string()],
                    provider_accounts: vec![],
                }],
                &host_machine_id,
            ),
            vec![RelayKernelPresence {
                kernel_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                relay_alias: Some("builder-kernel".to_string()),
                kernel_alias: Some("default".to_string()),
                available_providers: vec!["opencode".to_string()],
                provider_accounts: vec![],
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: "public-key".to_string(),
            }],
        );
    });

    let connection = match harness
        .dispatch(LocalDaemonRequest::ResolveKernelClientConnection(
            ResolveKernelClientConnectionRequest {
                kernel_ref: "daemon-1".to_string(),
                machine_ref: Some("machine-1".to_string()),
                client_id: Some("cli-1".to_string()),
                session_id: None,
            },
        ))
        .expect("kernel client connection resolve should succeed")
    {
        LocalDaemonResponse::KernelClientConnectionResolved { connection } => connection,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(connection.relay_url, "ws://relay.local");
    assert_eq!(connection.relay_token, "shared-token");
    assert_eq!(connection.target_daemon_id.as_deref(), Some("daemon-1"));
    assert_eq!(
        connection.target_daemon_alias.as_deref(),
        Some("builder-kernel")
    );
    assert_eq!(connection.machine_id.as_deref(), Some("machine-1"));
    assert_eq!(connection.kernel_id.as_deref(), Some("daemon-1"));
    assert_eq!(connection.token_expires_at, None);
}

#[test]
fn local_request_api_lists_same_machine_sibling_kernels_as_trusted() {
    let config = DaemonConfig::for_tests();
    let host_machine_id = config.host_machine_id.clone();
    let harness = LocalRouterTestHarness::with_config(config);
    harness.with_app_mut(|app| {
        app.remote_relay_inventory_projection_store().update(
            crate::local::provider_requests::remote_machine_records(
                vec![RelayMachinePresence {
                    machine_id: host_machine_id.clone(),
                    machine_alias: Some("laptop".to_string()),
                    kernel_count: 2,
                    available_providers: Vec::new(),
                    provider_accounts: Vec::new(),
                }],
                &host_machine_id,
            ),
            vec![RelayKernelPresence {
                kernel_id: "sibling-kernel".to_string(),
                machine_id: host_machine_id.clone(),
                machine_alias: Some("laptop".to_string()),
                relay_alias: None,
                kernel_alias: Some("experiments".to_string()),
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 1,
                public_key: "sibling-public-key".to_string(),
            }],
        );
    });

    let machines = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachines(
            ListRemoteMachinesRequest,
        ))
        .expect("same-machine inventory should succeed")
    {
        LocalDaemonResponse::RemoteMachinesListed { machines } => machines,
        other => panic!("unexpected response: {other:?}"),
    };
    let machine = machines
        .iter()
        .find(|machine| machine.machine_id == host_machine_id)
        .expect("local machine with sibling kernels should be listed");
    assert_eq!(machine.trust_status, RemoteMachineTrustStatus::Approved);
    assert!(!machine.pending);
    assert_eq!(machine.kernel_count, 2);
}
