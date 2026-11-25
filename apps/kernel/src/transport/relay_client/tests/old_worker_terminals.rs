//! Row 549: a Room whose agent is leased to a worker too old for Room terminals still lists its terminals
//! and opens one on its own kernel; asking for the old worker says what to do, as a refusal a client shows.
use super::support::*;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult};

async fn old_worker_rig() -> (
    CommandRouter,
    String,
    String,
    crate::test_support::TestWorktree,
) {
    let root = crate::test_support::TestWorktree::new("old-worker-terminals");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("home bootstraps");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-old-worker",
            root.path().display().to_string(),
        ))
        .expect("session is created");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "kernel-too-old".to_string(),
                worker_machine_id: "machine-too-old".to_string(),
                execution_lease_id: "old-lease".to_string(),
                leased_agent_id: "old-leased-agent".to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION - 1,
                ),
                worker_worktree_id: None,
            },
        )
        .expect("agent binds to the old worker");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    (
        router,
        session.id().to_string(),
        agent.id().to_string(),
        root,
    )
}

async fn call(
    router: &CommandRouter,
    session_id: &str,
    host_agent: Option<&str>,
    operation: RoomTerminalOperation,
) -> Result<RoomTerminalResult, crate::error::DaemonError> {
    let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
        session_id: session_id.to_string(),
        acting_agent: None,
        host_agent: host_agent.map(str::to_string),
        operation,
    });
    let command = KernelCommand::from_local_request("room-terminal", None, None, &request);
    match router.dispatch(command, request).await? {
        LocalDaemonResponse::RoomTerminal { result } => Ok(result),
        other => panic!("unexpected terminal response: {other:?}"),
    }
}

#[test]
fn a_room_list_leaves_out_a_worker_too_old_for_terminals_and_the_home_kernel_opens_one() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("old-worker-room-terminals", || async {
        let (router, session, agent, _root) = old_worker_rig().await;
        let listed = call(&router, &session, None, RoomTerminalOperation::List)
            .await
            .expect("the Room-wide list answers although one worker is too old");
        assert!(matches!(listed, RoomTerminalResult::Listed { terminals } if terminals.is_empty()));
        let opened = call(
            &router,
            &session,
            None,
            RoomTerminalOperation::Open {
                cols: 80,
                rows: 24,
                title: None,
                command: None,
                stdin: None,
            },
        )
        .await
        .expect("a terminal opens on the Room's own kernel");
        assert!(
            matches!(opened, RoomTerminalResult::Opened { terminal } if terminal.host_agent.is_none())
        );
        let asked_for_the_old_worker =
            call(&router, &session, Some(&agent), RoomTerminalOperation::List)
                .await
                .expect_err("a terminal on the old worker is refused");
        let crate::error::DaemonError::MachineRequest { message, .. } = asked_for_the_old_worker
        else {
            panic!(
                "the refusal is the terminal service's own sentence: {asked_for_the_old_worker:?}"
            );
        };
        assert!(
            message.contains("is too old for Room terminals; update it")
                && message.contains("own kernel"),
            "{message}"
        );
    });
}
