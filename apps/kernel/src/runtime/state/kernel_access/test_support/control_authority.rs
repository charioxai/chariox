use super::session_authority::{external_command, runtime};
use super::worker_spy::WorkerSpy;
use super::*;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::session_actor::FocusedAgentProjection;
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

#[derive(Clone, Copy)]
enum Operation {
    MoveLocal,
    MoveRemote,
    Config,
}

macro_rules! regression {
    ($name:ident, $operation:ident, $discovery:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            revoked_control(Operation::$operation, $discovery).await;
        }
    };
}
regression!(kernel_access_move_local_rechecks_app_wait, MoveLocal, false);
regression!(
    kernel_access_move_remote_rechecks_app_wait,
    MoveRemote,
    false
);
regression!(
    kernel_access_move_local_rechecks_discovery_wait,
    MoveLocal,
    true
);
regression!(
    kernel_access_move_remote_rechecks_discovery_wait,
    MoveRemote,
    true
);
regression!(
    kernel_access_remote_config_rechecks_discovery_wait,
    Config,
    true
);

async fn dispatch(
    state: &KernelRuntimeState,
    session_runtime: &crate::runtime::session_actor::SessionRuntime,
    command: crate::runtime::command::KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let agent_runtime = AgentRuntime::new(
        state.clone(),
        state.provider_runtime_lanes.clone(),
        FocusedAgentProjection::default(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
        state.owned.prompt_state_owner.clone(),
        Default::default(),
    );
    let dispatched = crate::runtime::interactive_command_dispatcher::dispatch_interactive_command(
        session_runtime,
        &agent_runtime,
        state,
        command,
        request,
    );
    // MP-08 / MP-10 / MP-11: unrelated controls must not carry the protected
    // browser action's large future on the default test/runtime stack.
    assert!(std::mem::size_of_val(&dispatched) <= 1024);
    dispatched.await
}

async fn revoked_control(operation: Operation, discovery: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-control-authority");
    let worker = WorkerSpy::new(discovery);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("control-authority-fixture".into());
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request().with_agent_defaults(
            crate::session::SessionAgentDefaults {
                provider: "dev-stub".into(),
                ..Default::default()
            },
        ))
        .unwrap();
    if !matches!(operation, Operation::MoveRemote) {
        daemon
            .agents_mut()
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
    }
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let session_runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(session.id());
    let before = state.owned.agent_store.get_agent(agent.id()).unwrap();
    let request = match operation {
        Operation::MoveLocal => {
            LocalDaemonRequest::MoveAgentToLocal(crate::local::MoveAgentToLocalRequest {
                session_id: session.id().into(),
                agent_ref: agent.id().into(),
            })
        }
        Operation::MoveRemote => {
            LocalDaemonRequest::MoveAgentToRemote(crate::local::MoveAgentToRemoteRequest {
                session_id: session.id().into(),
                agent_ref: agent.id().into(),
                machine_ref: "fixture-machine".into(),
            })
        }
        Operation::Config => {
            LocalDaemonRequest::UpdateAgentConfig(crate::local::UpdateAgentConfigRequest {
                session_id: session.id().into(),
                agent_id: agent.id().into(),
                execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
                clear_execution_mode: false,
                permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                clear_permission_level: false,
                workspace_id: None,
                clear_workspace_id: false,
                worktree_id: None,
                clear_worktree_id: false,
            })
        }
    };
    state.authorize_external_request(&grant, &request).unwrap();
    let guard = if discovery {
        None
    } else {
        Some(app.lock().await)
    };
    let pending = tokio::spawn({
        let state = state.clone();
        let session_runtime = session_runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { dispatch(&state, &session_runtime, command, request).await }
    });
    let reached = if discovery {
        &worker.discovery_started
    } else {
        &probe
    };
    timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked agent control reached worker"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert_eq!(
        state.owned.agent_store.get_agent(agent.id()).unwrap(),
        before
    );
    let terminal = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-control",
        None,
        None,
        &request,
    );
    let response = dispatch(&state, &session_runtime, terminal, request)
        .await
        .unwrap();
    assert!(match operation {
        Operation::MoveLocal => matches!(response, LocalDaemonResponse::AgentMovedToLocal { .. }),
        Operation::MoveRemote => matches!(response, LocalDaemonResponse::AgentMovedToRemote { .. }),
        Operation::Config => matches!(response, LocalDaemonResponse::AgentConfigUpdated { .. }),
    });
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        if matches!(operation, Operation::Config) {
            1
        } else {
            2
        }
    );
}
