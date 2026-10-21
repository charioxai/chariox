//! MP-08/MP-09/MP-10/MP-11: local focus after a worker-owned run settles.
use super::*;

async fn focus_local_after_leased_run(projected: bool) {
    let worktree = crate::test_support::TestWorktree::new("leased-focus-recovery");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, remote) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let local = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("local-after-lease"),
        )
        .unwrap();
    let binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        execution_lease_id: "lease-focus".into(),
        leased_agent_id: "leased-agent-focus".into(),
        active_worker_provider_run_id: Some("worker-run-focus".into()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    app.agents
        .bind_remote_execution(remote.id(), binding.clone())
        .unwrap();
    let id = "leased:leased-agent-focus:worker-run-focus";
    if projected {
        let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
            "worker-run-focus",
            "worker-session".into(),
            Some("leased-agent-focus".into()),
            "dev-stub".into(),
        );
        run.mark_running();
        app.update_provider_run_projection(run.projected_for_home_agent_with_id(
            id,
            session.id(),
            remote.id(),
        ));
    }
    app.sessions
        .set_active_provider_run(session.id(), Some(id.into()))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .focus_agent(
            session.id(),
            local.id(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("a local focus must not resolve or park a worker-owned run in the home store");
    let focused = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    assert_eq!(focused.focused_agent_id(), Some(local.id()));
    assert_eq!(focused.active_provider_run_id(), None);
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(remote.id())
            .unwrap()
            .remote_execution(),
        Some(&binding)
    );
    if projected {
        assert_eq!(
            runtime
                .owned
                .provider_run_projection
                .get(id)
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Running,
            "focus must leave the worker-owned projection unchanged"
        );
    }
}

#[tokio::test]
async fn mp10_a10_local_focus_after_leased_projection_is_lost() {
    focus_local_after_leased_run(false).await;
}

#[tokio::test]
async fn mp10_a10_local_focus_does_not_park_worker_owned_run() {
    focus_local_after_leased_run(true).await;
}
