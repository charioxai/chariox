//! MP-08 / MP-09 / MP-10 / MP-11: a leased agent's deadline is enforced by the worker that runs it.
use super::leased_worker_route_tests::{leased_home, LEASED_AGENT};
use super::*;
use crate::local::{AgentDeadlineState, SetAgentDeadlineRequest};
use crate::runtime::router::CommandRouter;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

#[tokio::test]
async fn the_home_asks_the_worker_to_enforce_a_leased_agents_deadline_and_then_records_it() {
    let mut home = leased_home("leased-deadline-home").await;
    let owner = home
        .state
        .owned
        .session_store
        .get_session(&home.session_id)
        .unwrap()
        .owner_user_id()
        .to_string();
    let deadline_ms = crate::session::unix_epoch_ms() + 60_000;
    let call = home.state.set_agent_deadline(
        &owner,
        SetAgentDeadlineRequest {
            session_id: home.session_id.clone(),
            agent_id: home.agent_id.clone(),
            deadline_ms,
        },
    );
    let reply = home
        .rig
        .answer(|_| RelayPeerResponse::LeasedAgentDeadlineSet {
            leased_agent_id: LEASED_AGENT.into(),
        });
    let (result, request) = tokio::join!(call, reply);
    result.expect("the worker confirms the deadline");
    assert!(matches!(
        request,
        RelayPeerRequest::SetLeasedAgentDeadline { leased_agent_id, deadline_ms: sent }
            if leased_agent_id == LEASED_AGENT && sent == deadline_ms
    ));
    let recorded = home
        .state
        .owned
        .durable_state_store
        .agent_deadline(&home.session_id, &home.agent_id)
        .unwrap()
        .expect("the home records the deadline once the worker holds it");
    assert_eq!(recorded.deadline_ms, deadline_ms);
}

#[tokio::test]
async fn a_worker_that_does_not_confirm_leaves_the_home_without_a_deadline() {
    let mut home = leased_home("leased-deadline-unconfirmed").await;
    let owner = home
        .state
        .owned
        .session_store
        .get_session(&home.session_id)
        .unwrap()
        .owner_user_id()
        .to_string();
    let call = home.state.set_agent_deadline(
        &owner,
        SetAgentDeadlineRequest {
            session_id: home.session_id.clone(),
            agent_id: home.agent_id.clone(),
            deadline_ms: crate::session::unix_epoch_ms() + 60_000,
        },
    );
    let reply = home
        .rig
        .answer(|_| RelayPeerResponse::LeasedAgentDestroyed {
            leased_agent_id: LEASED_AGENT.into(),
        });
    let (result, _) = tokio::join!(call, reply);
    assert!(
        matches!(&result, Err(DaemonError::MachineRequest { code, .. })
            if code == "agent_deadline_worker_refused"),
        "{result:?}"
    );
    assert!(home
        .state
        .owned
        .durable_state_store
        .agent_deadline(&home.session_id, &home.agent_id)
        .unwrap()
        .is_none());
}

/// The worker alone, with no home connected: its own tick ends the leased agent's work.
#[tokio::test]
async fn a_worker_stops_a_leased_agent_at_its_deadline_without_the_home() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let leased = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let worker = router.runtime_state();

    let deadline_ms = crate::session::unix_epoch_ms() + 200;
    worker
        .set_relay_leased_agent_deadline(&leased.id, deadline_ms)
        .await
        .expect("the worker arms the deadline on the agent that carries the leased work");
    let armed = worker
        .owned
        .durable_state_store
        .agent_deadline(&leased.backing_session_id, &leased.backing_agent_id)
        .unwrap()
        .expect("the backing agent holds the deadline");
    assert_eq!(armed.deadline_ms, deadline_ms);
    assert_eq!(armed.state, AgentDeadlineState::Armed);

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    worker.tick_agent_deadlines(deadline_ms + 1).await.unwrap();
    let reached = worker
        .owned
        .durable_state_store
        .agent_deadline(&leased.backing_session_id, &leased.backing_agent_id)
        .unwrap()
        .unwrap();
    assert_eq!(reached.state, AgentDeadlineState::Reached);
    assert!(reached.stopped_at_ms.is_some(), "{reached:?}");

    // A deadline only ever shortens.
    let later = worker
        .set_relay_leased_agent_deadline(&leased.id, deadline_ms + 1)
        .await;
    assert!(
        matches!(&later, Err(DaemonError::MachineRequest { code, .. })
            if code == "agent_deadline_extension_refused"),
        "{later:?}"
    );
}
