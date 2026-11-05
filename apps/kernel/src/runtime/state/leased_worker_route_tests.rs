//! MP-08/MP-09/MP-10/MP-11 A04: every home-to-worker request for a leased
//! agent takes the relay connection that already reaches the worker. A hosted
//! or slice worker is not visible to a temporary connection's metadata lookup.
use super::*;
use crate::room_terminal::TerminalActor;
use crate::runtime::router::CommandRouter;
use crate::transport::relay_client::ConnectedWorkerRig;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

pub(super) const WORKER_RUN: &str = "worker-run";
pub(super) const LEASED_AGENT: &str = "leased";

/// A home kernel with one agent leased to a worker that is reachable only over
/// the home's connected relay connection (the relay URL is unresolvable).
pub(super) struct LeasedHome {
    pub(super) state: KernelRuntimeState,
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) rig: ConnectedWorkerRig,
    _workspace: crate::test_support::TestWorktree,
}

pub(super) async fn leased_home(name: &str) -> LeasedHome {
    let workspace = crate::test_support::TestWorktree::new(name);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://fixture.invalid".into());
    config.relay_token = Some("home-token".into());
    let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    let rig =
        ConnectedWorkerRig::connect(&state.owned.relay_state, &config, "ws://fixture.invalid")
            .await;
    state
        .owned
        .agent_store
        .bind_remote_execution(
            agent.id(),
            serde_json::from_value(serde_json::json!({
                "worker_kernel_id": rig.worker_kernel_id(), "worker_machine_id": "machine",
                "execution_lease_id": "lease", "leased_agent_id": LEASED_AGENT,
                "active_worker_provider_run_id": WORKER_RUN,
                "relay_peer_protocol_version": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            }))
            .unwrap(),
        )
        .unwrap();
    LeasedHome {
        state,
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        rig,
        _workspace: workspace,
    }
}

#[tokio::test]
async fn room_terminal_request_goes_over_the_connected_relay_that_reaches_the_worker() {
    let mut home = leased_home("leased-route-terminal").await;
    let call = home.state.route_room_terminal(
        &home.session_id,
        TerminalActor::User,
        Some(&home.agent_id),
        false,
        crate::local::RoomTerminalOperation::List,
    );
    let reply = home.rig.answer(|_| RelayPeerResponse::RoomTerminal {
        result: crate::local::RoomTerminalResult::Listed { terminals: vec![] },
    });
    let (result, request) = tokio::join!(call, reply);
    result.expect("the worker answers the terminal request");
    assert!(matches!(
        request,
        RelayPeerRequest::RoomTerminal { leased_agent_id, .. } if leased_agent_id == LEASED_AGENT
    ));
}
