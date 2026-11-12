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
            deadline_ms: Some(deadline_ms),
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
            if leased_agent_id == LEASED_AGENT && sent == Some(deadline_ms)
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
            deadline_ms: Some(crate::session::unix_epoch_ms() + 60_000),
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

/// A worker with one leased agent and no home connected.
fn worker_with_leased_agent() -> (KernelRuntimeState, crate::execution_lease::LeasedAgent) {
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
    (router.runtime_state(), leased)
}

/// The worker alone, with no home connected: its own tick ends the leased agent's work.
#[tokio::test]
async fn a_worker_stops_a_leased_agent_at_its_deadline_without_the_home() {
    let (worker, leased) = worker_with_leased_agent();

    let deadline_ms = crate::session::unix_epoch_ms() + 200;
    worker
        .set_relay_leased_agent_deadline(&leased.id, Some(deadline_ms))
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

    // A completed deadline cannot revive the ended agent.
    let later = worker
        .set_relay_leased_agent_deadline(&leased.id, Some(deadline_ms + 1))
        .await;
    assert!(
        matches!(&later, Err(DaemonError::MachineRequest { code, .. })
            if code == "agent_deadline_reached"),
        "{later:?}"
    );
}

/// A request the home sent before the deadline can reach the worker after it: the expired agent
/// starts no provider run and takes no prompt.
#[tokio::test]
async fn a_leased_prompt_that_arrives_after_the_deadline_starts_nothing() {
    let (worker, leased) = worker_with_leased_agent();
    let deadline_ms = crate::session::unix_epoch_ms() + 100;
    worker
        .set_relay_leased_agent_deadline(&leased.id, Some(deadline_ms))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    worker.tick_agent_deadlines(deadline_ms + 1).await.unwrap();

    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session".into(),
        home_agent_id: "home-agent".into(),
        home_prompt_id: "home-late-prompt".into(),
        home_turn_id: "home-late-prompt".into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "late".into(),
    };
    let refused = worker
        .submit_relay_leased_prompt(
            &leased.id,
            crate::transport::relay_peer::RelayAgentExecutionProfile {
                provider: leased.provider.clone(),
                account_profile: leased.account_profile.clone(),
                model: leased.model.clone(),
                effort: leased.effort.clone(),
            },
            None,
            "one more thing",
            "",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            Some(Vec::new()),
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await;
    assert!(
        matches!(&refused, Err(DaemonError::MachineRequest { code, .. })
            if code == "agent_deadline_reached"),
        "{refused:?}"
    );
    assert!(
        worker.owned.provider_store.list_runs().is_empty(),
        "no provider run starts for an expired agent"
    );
    let session = worker
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    assert!(worker
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &leased.backing_agent_id)
        .is_none());
    assert!(worker
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(&session, &leased.backing_agent_id)
        .is_none());
}

#[tokio::test]
async fn a_worker_accepts_extensions_and_clears_before_the_deadline_fires() {
    let (worker, leased) = worker_with_leased_agent();
    let first = crate::session::unix_epoch_ms() + 60_000;
    for time in [
        Some(first),
        Some(first + 60_000),
        None,
        Some(first - 10_000),
    ] {
        worker
            .set_relay_leased_agent_deadline(&leased.id, time)
            .await
            .unwrap();
        let actual = worker
            .owned
            .durable_state_store
            .agent_deadline(&leased.backing_session_id, &leased.backing_agent_id)
            .unwrap();
        assert_eq!(actual.map(|deadline| deadline.deadline_ms), time);
    }
}

#[tokio::test]
async fn edited_worker_deadlines_do_not_fire_at_the_old_time() {
    let (worker, leased) = worker_with_leased_agent();
    let old = crate::session::unix_epoch_ms() + 60_000;
    worker
        .set_relay_leased_agent_deadline(&leased.id, Some(old))
        .await
        .unwrap();
    worker
        .set_relay_leased_agent_deadline(&leased.id, Some(old + 60_000))
        .await
        .unwrap();
    worker.tick_agent_deadlines(old + 1).await.unwrap();
    let get = || {
        worker
            .owned
            .durable_state_store
            .agent_deadline(&leased.backing_session_id, &leased.backing_agent_id)
            .unwrap()
    };
    assert_eq!(get().unwrap().state, AgentDeadlineState::Armed);
    worker
        .set_relay_leased_agent_deadline(&leased.id, None)
        .await
        .unwrap();
    worker.tick_agent_deadlines(old + 120_000).await.unwrap();
    assert!(get().is_none());
}
