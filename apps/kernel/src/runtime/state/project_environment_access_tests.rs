use super::*;
use crate::runtime::command::KernelCommand;
use crate::runtime::router::CommandRouter;
use crate::runtime::state::kernel_access::test_support::worker_spy::WorkerSpy;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
enum Wait {
    Ordering,
    App,
    Discovery,
}

macro_rules! regression {
    ($name:ident, $wait:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            revoked_setup_cancel(Wait::$wait).await;
        }
    };
}
regression!(kernel_access_setup_cancel_rechecks_ordering_wait, Ordering);
regression!(kernel_access_setup_cancel_rechecks_app_wait, App);
regression!(
    kernel_access_setup_cancel_rechecks_discovery_wait,
    Discovery
);

async fn revoked_setup_cancel(wait: Wait) {
    let worktree = crate::test_support::TestWorktree::new("access-setup-cancel");
    let worker = WorkerSpy::new(matches!(wait, Wait::Discovery));
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("setup-cancel-fixture".into());
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    app.agents_mut()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker.id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let mut execution = super::tests::execution();
    execution.owner_user_id = session.owner_user_id().into();
    execution.session_id = session.id().into();
    execution.agent_id = agent.id().into();
    execution.project_id = session.project_id().into();
    execution.execution_session_id = session.id().into();
    execution.execution_agent_id = agent.id().into();
    execution.workspace_id = worktree.path().display().to_string();
    execution.target_worker_id = "fixture-machine".into();
    execution.remote_leased_agent_id = Some("leased-agent".into());
    let (before, _) = state
        .owned
        .project_environment_setups
        .begin(execution.clone())
        .unwrap();
    *worker.setup_status.lock().unwrap() = Some(before.clone());
    let grant = state.insert_access_grant_for_test(session.id());
    let request =
        LocalDaemonRequest::CancelProjectEnvironmentSetup(CancelProjectEnvironmentSetupRequest {
            operation_id: execution.operation_id.clone(),
            session_id: session.id().into(),
        });
    let gate = state
        .owned
        .project_environment_setups
        .ordering_gate(&execution.operation_id);
    let ordering = if matches!(wait, Wait::Ordering) {
        Some(gate.lock_owned().await)
    } else {
        None
    };
    let guard = if matches!(wait, Wait::App) {
        Some(app.lock().await)
    } else {
        None
    };
    let mut command =
        KernelCommand::from_local_request("external-setup-cancel", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let pending = router.dispatch(command, request.clone());
    tokio::pin!(pending);
    if matches!(wait, Wait::Discovery) {
        tokio::select! { result = &mut pending => panic!("cancel did not reach discovery: {}", result.is_ok()), _ = worker.discovery_started.notified() => {} }
    } else {
        tokio::select! { biased; result = &mut pending => panic!("cancel did not wait: {}", result.is_ok()), _ = tokio::task::yield_now() => {} }
    }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(ordering);
    drop(guard);
    if matches!(wait, Wait::Discovery) {
        worker.release_discovery.notify_one();
    }
    let result = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked setup cancellation reached worker"
    );
    if matches!(wait, Wait::Ordering) {
        assert!(
            !state
                .owned
                .project_environment_setups
                .is_cancelled(&execution.operation_id, before.attempt),
            "revoked setup cancellation changed state"
        );
        assert_eq!(
            state
                .owned
                .project_environment_setups
                .get_entry(&execution.operation_id, session.owner_user_id())
                .unwrap()
                .1,
            before
        );
    }
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    let command = KernelCommand::from_local_request("terminal-setup-cancel", None, None, &request);
    let response = router.dispatch(command, request).await.unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::ProjectEnvironmentSetupCancelled { .. }
    ));
    assert_eq!(worker.requests.load(Ordering::SeqCst), 1);
}
