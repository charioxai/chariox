//! MP-08/MP-11: HOME admission stays bound to the explicit grant and live lease.
use super::*;
use crate::runtime::router::CommandRouter;

#[tokio::test]
async fn mp11_leased_computer_home_fences_context_epoch_and_ordinary_focus() {
    let workspace = crate::test_support::TestWorktree::new("leased-computer-home");
    let config = crate::config::DaemonConfig::for_tests();
    let home = config.daemon_id.clone();
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    app.agents()
        .bind_remote_execution(
            agent.id(),
            serde_json::from_value(serde_json::json!({
                "worker_kernel_id":"desktop", "worker_machine_id":"machine",
                "execution_lease_id":"lease", "leased_agent_id":"leased",
                "active_worker_provider_run_id":"run", "relay_peer_protocol_version":109
            }))
            .unwrap(),
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4)
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            "desktop",
        ));
    let state = router.runtime_state();
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: home.clone(),
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        leased_agent_id: "leased".into(),
        worker_provider_run_id: "run".into(),
        worker_kernel_id: Some("desktop".into()),
        worker_machine_id: Some("machine".into()),
    };
    let grant = crate::transport::relay_peer::LeasedComputerGrant {
        epoch: "epoch".into(),
        home_kernel_id: home,
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        leased_agent_id: "leased".into(),
        desktop_kernel_id: "desktop".into(),
        target: crate::local::KernelDesktopTarget {
            surface_id: "surface".into(),
            generation: "generation".into(),
        },
    };
    let user = state.provider_account_authority_owner_user_id(agent.owner_user_id());
    assert!(state
        .check_leased_computer_grant(&context, "epoch")
        .is_err());
    state
        .owned
        .leased_computer_grants
        .lock()
        .unwrap()
        .insert(agent.id().into(), grant.clone());
    state
        .owned
        .kernel_browser_host
        .set_focus(&user, Some(agent.id()));
    assert!(
        state
            .check_leased_computer_grant(&context, "epoch")
            .is_err(),
        "MP-11: ordinary focus cannot manufacture explicit Computer authority"
    );
    state
        .owned
        .kernel_browser_host
        .grant_desktop(&user, agent.id(), "surface");
    assert_eq!(
        state
            .check_leased_computer_grant(&context, "epoch")
            .unwrap(),
        grant
    );
    assert!(state
        .check_leased_computer_grant(&context, "old-epoch")
        .is_err());
    for field in [
        "home_kernel_id",
        "home_session_id",
        "home_agent_id",
        "leased_agent_id",
        "worker_provider_run_id",
        "worker_kernel_id",
        "worker_machine_id",
    ] {
        let mut value = serde_json::to_value(&context).unwrap();
        value[field] = "foreign".into();
        assert!(
            state
                .check_leased_computer_grant(&serde_json::from_value(value).unwrap(), "epoch")
                .is_err(),
            "MP-11: {field}"
        );
    }
    let mut newer = grant.clone();
    newer.epoch = "new-epoch".into();
    state
        .owned
        .leased_computer_grants
        .lock()
        .unwrap()
        .insert(agent.id().into(), newer.clone());
    assert!(
        state
            .retire_leased_computer_grant(&user, agent.id(), Some("epoch"))
            .is_none(),
        "MP-11: a late failure/revoke cannot retire a newer grant"
    );
    assert_eq!(
        state
            .check_leased_computer_grant(&context, "new-epoch")
            .unwrap(),
        newer
    );
    assert_eq!(
        state.retire_leased_computer_grant(&user, agent.id(), Some("new-epoch")),
        Some(newer)
    );
    state.owned.kernel_browser_host.revoke_agent(agent.id());
    state
        .owned
        .kernel_browser_host
        .set_focus(&user, Some(agent.id()));
    assert!(
        state
            .check_leased_computer_grant(&context, "epoch")
            .is_err(),
        "MP-11: focus cannot revive revoked or expired authority"
    );
    state.shutdown_cleanup().await.unwrap();
}
