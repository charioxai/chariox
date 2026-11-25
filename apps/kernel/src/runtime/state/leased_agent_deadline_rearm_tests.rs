//! A home start gives back its downtime; the worker that runs a leased agent must confirm the
//! shifted deadline, and until it does the value it holds governs.
use super::leased_agent_deadline_tests::worker_with_leased_agent;
use super::leased_worker_route_tests::{leased_home, LeasedHome, LEASED_AGENT};
use super::*;
use crate::durable_state::agent_lifecycle::Operation;
use crate::execution_lease::LeasedAgent;
use crate::local::{AgentDeadline, AgentDeadlineState, SetAgentDeadlineRequest};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const DOWN_MS: u64 = 120_000;

/// The worker kernel's own deadline handler answers the home's request for `leased_agent_id`.
async fn worker_answers(
    worker: &KernelRuntimeState,
    leased_agent_id: &str,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let RelayPeerRequest::SetLeasedAgentDeadline {
        leased_agent_id: sent_for,
        deadline_ms,
    } = request
    else {
        panic!("expected a deadline request, got {request:?}")
    };
    worker
        .set_relay_leased_agent_deadline(leased_agent_id, deadline_ms)
        .await?;
    Ok(RelayPeerResponse::LeasedAgentDeadlineSet {
        leased_agent_id: sent_for,
    })
}

fn owner(home: &LeasedHome) -> String {
    home.state
        .owned
        .session_store
        .get_session(&home.session_id)
        .unwrap()
        .owner_user_id()
        .to_string()
}

fn home_deadline(home: &LeasedHome, agent: &str) -> AgentDeadline {
    home.state
        .owned
        .durable_state_store
        .agent_deadline(&home.session_id, agent)
        .unwrap()
        .unwrap()
}

fn worker_deadline(worker: &KernelRuntimeState, leased: &LeasedAgent) -> AgentDeadline {
    worker
        .owned
        .durable_state_store
        .agent_deadline(&leased.backing_session_id, &leased.backing_agent_id)
        .unwrap()
        .unwrap()
}

fn awaiting(home: &LeasedHome) -> Vec<String> {
    home.state
        .owned
        .durable_state_store
        .agent_deadlines_awaiting_rearm()
        .unwrap()
        .into_iter()
        .map(|held| held.agent_id)
        .collect()
}

/// The home kernel stopped for `DOWN_MS` right after its last heartbeat and started again.
fn restart(home: &LeasedHome) {
    home.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::ResumeDeadlines {
            now: crate::session::unix_epoch_ms() + DOWN_MS,
        })
        .unwrap();
}

/// A home whose leased agent's deadline, `lead_ms` from now, a real worker armed; then a home
/// restart.
async fn restarted_home_with_worker(
    name: &str,
    lead_ms: u64,
) -> (LeasedHome, KernelRuntimeState, LeasedAgent, u64) {
    let mut home = leased_home(name).await;
    let (worker, leased) = worker_with_leased_agent();
    let armed_ms = crate::session::unix_epoch_ms() + lead_ms;
    let owner = owner(&home);
    let call = home.state.set_agent_deadline(
        &owner,
        SetAgentDeadlineRequest {
            session_id: home.session_id.clone(),
            agent_id: home.agent_id.clone(),
            deadline_ms: Some(armed_ms),
        },
    );
    let serve = home
        .rig
        .serve(|request| worker_answers(&worker, &leased.id, request));
    let (result, ()) = tokio::join!(call, serve);
    result.unwrap();
    assert_eq!(worker_deadline(&worker, &leased).deadline_ms, armed_ms);
    restart(&home);
    (home, worker, leased, armed_ms)
}

#[tokio::test]
async fn after_a_restart_the_home_tells_the_worker_the_shifted_deadline() {
    let (mut home, worker, leased, armed_ms) =
        restarted_home_with_worker("leased-deadline-rearm", 600_000).await;
    let shifted = home_deadline(&home, &home.agent_id).deadline_ms;
    assert!(
        shifted >= armed_ms + DOWN_MS - 1_000,
        "{shifted} {armed_ms}"
    );
    let held = home
        .state
        .owned
        .durable_state_store
        .agent_deadlines_awaiting_rearm()
        .unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].held_ms, armed_ms, "the value the worker holds");

    let call = home.state.rearm_leased_agent_deadlines();
    let serve = home
        .rig
        .serve(|request| worker_answers(&worker, &leased.id, request));
    let (result, ()) = tokio::join!(call, serve);
    result.expect("the worker confirms the shifted deadline");
    assert_eq!(worker_deadline(&worker, &leased).deadline_ms, shifted);
    assert!(awaiting(&home).is_empty());
    // Nothing is left to tell the worker: no second request, so no answer is needed.
    home.state.rearm_leased_agent_deadlines().await.unwrap();
}

/// P1: the worker kept counting while the home was down and stopped the agent at the deadline it
/// held. The home must not advertise the time it gave back: it reaches the worker's deadline too.
#[tokio::test]
async fn a_worker_that_already_reached_the_deadline_ends_it_at_the_home_too() {
    let (mut home, worker, leased, armed_ms) =
        restarted_home_with_worker("leased-deadline-worker-reached", 500).await;
    // The home is still down: the worker's own tick reaches the deadline it holds.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    worker
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(
        worker_deadline(&worker, &leased).state,
        AgentDeadlineState::Reached
    );

    let call = home.state.rearm_leased_agent_deadlines();
    let serve = home
        .rig
        .serve(|request| worker_answers(&worker, &leased.id, request));
    let (result, ()) = tokio::join!(call, serve);
    result.expect("a refusal for good ends the entry; it is not retried");
    assert!(awaiting(&home).is_empty());
    let kept = home_deadline(&home, &home.agent_id);
    assert_eq!(kept.deadline_ms, armed_ms, "the worker's deadline governs");
    let notices = home
        .state
        .agent_notices_for_test(&home.session_id, &home.agent_id);
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("kept its deadline")
                && notice.contains("agent_deadline_reached")),
        "{notices:?}"
    );

    home.state
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    let reached = home_deadline(&home, &home.agent_id);
    assert_eq!(reached.state, AgentDeadlineState::Reached);
    assert!(reached.stopped_at_ms.is_some(), "{reached:?}");
}

#[tokio::test]
async fn a_worker_that_no_longer_runs_the_agent_ends_the_rearm_with_a_reason() {
    let (mut home, worker, _, armed_ms) =
        restarted_home_with_worker("leased-deadline-worker-gone", 600_000).await;
    let call = home.state.rearm_leased_agent_deadlines();
    // The worker's handler does not know this leased agent any more.
    let serve = home
        .rig
        .serve(|request| worker_answers(&worker, "ended-lease", request));
    let (result, ()) = tokio::join!(call, serve);
    result.expect("a refusal for good ends the entry; it is not retried");
    assert!(awaiting(&home).is_empty());
    let kept = home_deadline(&home, &home.agent_id);
    assert_eq!(kept.deadline_ms, armed_ms);
    assert_eq!(kept.state, AgentDeadlineState::Armed);
    let notices = home
        .state
        .agent_notices_for_test(&home.session_id, &home.agent_id);
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("leased_agent_not_found")),
        "{notices:?}"
    );
}

/// Another agent of the home leased to the same worker kernel as `leased_agent_id`.
fn add_leased_agent(home: &LeasedHome, leased_agent_id: &str) -> String {
    let session = home
        .state
        .owned
        .session_store
        .get_session(&home.session_id)
        .unwrap();
    let agent = home
        .state
        .owned
        .agent_store
        .create_agent_for_session(
            crate::agent::CreateAgentRequest::new(&home.session_id, "codex"),
            &session,
        )
        .unwrap();
    home.state
        .owned
        .agent_store
        .bind_remote_execution(
            agent.id(),
            serde_json::from_value(serde_json::json!({
                "worker_kernel_id": home.rig.worker_kernel_id(), "worker_machine_id": "machine",
                "execution_lease_id": "lease", "leased_agent_id": leased_agent_id,
                "active_worker_provider_run_id": "worker-run",
                "relay_peer_protocol_version": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            }))
            .unwrap(),
        )
        .unwrap();
    agent.id().to_string()
}

fn arm_at_home(home: &LeasedHome, agent: &str, deadline_ms: u64) {
    home.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::SetDeadline {
            room: home.session_id.clone(),
            agent: agent.into(),
            deadline: Some(deadline_ms),
        })
        .unwrap();
}

/// Three leased agents whose deadlines a restart moved; the healthy worker's agent comes last
/// in the order the entries are read.
fn three_restarted_agents(home: &LeasedHome) -> (Vec<String>, String) {
    let others = [
        add_leased_agent(home, "leased-b"),
        add_leased_agent(home, "leased-c"),
    ];
    let mut agents = vec![home.agent_id.clone(), others[0].clone(), others[1].clone()];
    agents.sort();
    let deadline_ms = crate::session::unix_epoch_ms() + 600_000;
    for agent in &agents {
        arm_at_home(home, agent, deadline_ms);
    }
    restart(home);
    let healthy = agents.pop().unwrap();
    (agents, healthy)
}

fn leased_id(home: &LeasedHome, agent: &str) -> String {
    home.state
        .owned
        .agent_store
        .get_agent(agent)
        .unwrap()
        .remote_execution()
        .unwrap()
        .leased_agent_id
        .clone()
}

/// P1: two workers that time out come first; the sweep still reaches the third in one pass.
#[tokio::test]
async fn a_worker_that_times_out_is_told_again_and_does_not_hold_back_the_others() {
    let mut home = leased_home("leased-deadline-rearm-fair").await;
    let (unreachable, healthy) = three_restarted_agents(&home);
    let healthy_lease = leased_id(&home, &healthy);
    let now = crate::session::unix_epoch_ms();

    let started = std::time::Instant::now();
    let sweep = home.state.sweep_agent_wakes(now);
    let answer_healthy = |request: &RelayPeerRequest| match request {
        RelayPeerRequest::SetLeasedAgentDeadline {
            leased_agent_id, ..
        } if *leased_agent_id == healthy_lease => Some(RelayPeerResponse::LeasedAgentDeadlineSet {
            leased_agent_id: leased_agent_id.clone(),
        }),
        _ => None,
    };
    home.rig.answer_while(sweep, answer_healthy).await.unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(9),
        "one request timeout bounds the pass: {:?}",
        started.elapsed()
    );
    let mut left = awaiting(&home);
    left.sort();
    assert_eq!(left, unreachable, "only the unanswered workers are retried");

    let answer_all = |request: &RelayPeerRequest| match request {
        RelayPeerRequest::SetLeasedAgentDeadline {
            leased_agent_id, ..
        } => Some(RelayPeerResponse::LeasedAgentDeadlineSet {
            leased_agent_id: leased_agent_id.clone(),
        }),
        _ => None,
    };
    home.rig
        .answer_while(home.state.rearm_leased_agent_deadlines(), answer_all)
        .await
        .unwrap();
    assert!(awaiting(&home).is_empty());
}

/// P1: the 1 s deadline tick does not wait for a worker that does not answer the rearm.
#[tokio::test]
async fn the_deadline_tick_runs_while_a_rearm_waits_for_a_worker() {
    let mut home = leased_home("leased-deadline-rearm-tick").await;
    arm_at_home(
        &home,
        &home.agent_id.clone(),
        crate::session::unix_epoch_ms() + 600_000,
    );
    restart(&home);
    let state = home.state.clone();
    let both = async {
        tokio::join!(state.rearm_leased_agent_deadlines(), async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                state.tick_agent_deadlines(crate::session::unix_epoch_ms()),
            )
            .await
        })
    };
    let (rearm, tick) = home.rig.answer_while(both, |_| None).await;
    assert!(rearm.is_err(), "the worker never answered");
    tick.expect("the tick does not wait for the worker")
        .unwrap();
    assert_eq!(awaiting(&home), vec![home.agent_id.clone()]);
}

/// An owner's edit while a rearm waits for the worker is sent after it, so the worker ends on
/// the edited value, and the edit ends the entry.
#[tokio::test]
async fn an_edit_waits_for_a_rearm_in_flight() {
    let mut home = leased_home("leased-deadline-rearm-edit").await;
    arm_at_home(
        &home,
        &home.agent_id.clone(),
        crate::session::unix_epoch_ms() + 600_000,
    );
    restart(&home);
    let shifted = home_deadline(&home, &home.agent_id).deadline_ms;
    let edited = shifted + 60_000;
    let owner = owner(&home);
    let state = home.state.clone();
    let edit = async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        state
            .set_agent_deadline(
                &owner,
                SetAgentDeadlineRequest {
                    session_id: home.session_id.clone(),
                    agent_id: home.agent_id.clone(),
                    deadline_ms: Some(edited),
                },
            )
            .await
    };
    let rig = &mut home.rig;
    let confirmed = || RelayPeerResponse::LeasedAgentDeadlineSet {
        leased_agent_id: LEASED_AGENT.into(),
    };
    let worker = async {
        let (rearm_id, rearm) = rig.receive().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(800), rig.receive())
                .await
                .is_err(),
            "the edit is not sent while the rearm waits for the worker"
        );
        rig.reply(rearm_id, Ok(confirmed())).await;
        let (edit_id, edit) = rig.receive().await;
        rig.reply(edit_id, Ok(confirmed())).await;
        [rearm, edit].map(|request| match request {
            RelayPeerRequest::SetLeasedAgentDeadline { deadline_ms, .. } => deadline_ms,
            other => panic!("{other:?}"),
        })
    };
    let (rearm, edit, sent) = tokio::join!(state.rearm_leased_agent_deadlines(), edit, worker);
    rearm.unwrap();
    edit.unwrap();
    assert_eq!(sent, [Some(shifted), Some(edited)]);
    assert_eq!(home_deadline(&home, &home.agent_id).deadline_ms, edited);
    assert!(awaiting(&home).is_empty());
}
