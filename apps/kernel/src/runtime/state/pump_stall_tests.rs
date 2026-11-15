//! MP-08 / MP-09 / MP-10 / MP-11: a provider lane that is held, or a trigger that waits on one,
//! never holds the one maintenance pump. The pump goes on to its other steps, and the same work is
//! done once the lane is free.
use super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt;
use super::*;
use crate::session::{
    AgentTrigger, AgentTriggerBusyPolicy, AgentTriggerOutcome, AgentTriggerRepeat,
};

/// A pump that waits on nothing returns in well under a second; this is the room for a slow builder.
const PUMP_BOUND: std::time::Duration = std::time::Duration::from_secs(4);

/// Waits until `condition` holds, without a fixed pause; `None` when it never does.
async fn until(condition: impl Fn() -> bool) -> Option<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .ok()
}

pub(in crate::runtime::state) fn insert_run(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
) {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id,
        "dev-stub",
        "claude-code",
        "default",
        "sonnet",
    )
    .with_agent_id(agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "pump-stall".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: Some("test".into()),
        },
    );
    run.mark_running();
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    runtime.owned.provider_run_projection.update(run);
}

/// A failed launch of `run_id` whose cleanup is already due.
pub(in crate::runtime::state) fn schedule_launch_failure_retry(
    runtime: &KernelRuntimeState,
    run_id: &str,
) {
    let run = runtime.owned.provider_store.get_run(run_id).unwrap();
    let started = crate::app::StartedProviderLaunch {
        run,
        previous_active_run_id: None,
        provider_credential_env: Default::default(),
    };
    let error = DaemonError::ProviderProtocol {
        provider_run_id: run_id.into(),
        operation: "thread/start",
        message: "the provider did not start".into(),
    };
    assert!(runtime
        .owned
        .provider_launch_failure_retries
        .schedule_initial(&started, &error, 0));
}

pub(in crate::runtime::state) fn retry_pending(runtime: &KernelRuntimeState, run_id: &str) -> bool {
    runtime
        .owned
        .provider_launch_failure_retries
        .due_provider_run_ids(u64::MAX)
        .iter()
        .any(|id| id == run_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pump_does_not_wait_for_a_launch_retry_whose_provider_lane_is_held() {
    let (_worktree, runtime, session_id, agent_id, _, _, held_run_id) =
        runtime_with_active_prompt().await;
    let free_run_id = "provider-run-pump-stall-free";
    insert_run(&runtime, &session_id, &agent_id, free_run_id);
    schedule_launch_failure_retry(&runtime, &held_run_id);
    schedule_launch_failure_retry(&runtime, free_run_id);
    let held = runtime.provider_runtime_lanes.acquire(&held_run_id).await;

    tokio::time::timeout(PUMP_BOUND, runtime.pump_transport_runtime())
        .await
        .expect(
            "the pump waits for the provider lane of a launch retry, so every other step waits too",
        );
    until(|| !retry_pending(&runtime, free_run_id))
        .await
        .expect("the retry of another run waits behind the held lane");
    assert!(
        retry_pending(&runtime, &held_run_id),
        "the held run's retry is kept, not dropped"
    );

    drop(held);
    until(|| !retry_pending(&runtime, &held_run_id))
        .await
        .expect("the retry runs once its lane is free");
    runtime.task_work_settled().await;
    for run_id in [held_run_id.as_str(), free_run_id] {
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(run_id)
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Ended,
            "{run_id}: the failed launch is cleaned up exactly once"
        );
    }
}

pub(in crate::runtime::state) fn add_due_trigger(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    id: &str,
    busy_policy: AgentTriggerBusyPolicy,
) {
    let mut session = runtime.owned.session_store.get_session(session_id).unwrap();
    // Created long ago, so the first run is already due.
    session.add_agent_trigger(
        AgentTrigger::schedule(
            id,
            agent_id,
            "owner",
            crate::session::AgentSchedule::interval(60, AgentTriggerRepeat::Every),
            busy_policy,
            format!("fire {id}"),
            1,
        )
        .expect("schedule should be valid"),
    );
    runtime.owned.session_store.restore_session(session);
}

/// A second agent of the room with a turn running, so that a trigger of it queues behind it.
fn add_busy_agent(
    runtime: &KernelRuntimeState,
    session_id: &str,
    like_agent_id: &str,
    attachment_id: &str,
) -> String {
    let mut other =
        serde_json::to_value(runtime.owned.agent_store.get_agent(like_agent_id).unwrap()).unwrap();
    other["id"] = serde_json::json!("pump-stall-other-agent");
    other["agent_ref"] = serde_json::json!("pump-stall-other-agent-ref");
    let other: crate::agent::AgentInstance = serde_json::from_value(other).unwrap();
    runtime.owned.agent_store.restore_agent(other.clone());
    let mut room = runtime.owned.session_store.get_session(session_id).unwrap();
    let mut agents = room.agents().to_vec();
    agents.push(other.clone());
    room.set_agents(agents);
    room.mirror_agent_prompt_state(
        other.id(),
        Some(crate::session::PromptQueueItem::new(
            "pump-stall-other-active",
            attachment_id,
            other.id(),
            "holding",
            crate::session::PromptStatus::Running,
        )),
        Default::default(),
    );
    runtime
        .owned
        .session_store
        .write()
        .restore_session(room.clone());
    runtime
        .owned
        .prompt_state_owner
        .restore_session_state(&room);
    other.id().to_string()
}

pub(in crate::runtime::state) fn trigger(
    runtime: &KernelRuntimeState,
    session_id: &str,
    id: &str,
) -> AgentTrigger {
    runtime
        .owned
        .session_store
        .get_session(session_id)
        .unwrap()
        .agent_triggers()
        .iter()
        .find(|trigger| trigger.id() == id)
        .unwrap()
        .clone()
}

pub(in crate::runtime::state) fn firings_of(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    id: &str,
) -> usize {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, agent_id);
    active
        .iter()
        .chain(queued.iter())
        .filter(|prompt| prompt.agent_trigger_id() == Some(id))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pump_does_not_wait_for_a_trigger_stuck_on_a_held_lane() {
    let (_worktree, runtime, session_id, agent_id, attachment_id, _, run_id) =
        runtime_with_active_prompt().await;
    let other_agent_id = add_busy_agent(&runtime, &session_id, &agent_id, &attachment_id);
    add_due_trigger(
        &runtime,
        &session_id,
        &agent_id,
        "stuck",
        AgentTriggerBusyPolicy::Steer,
    );
    add_due_trigger(
        &runtime,
        &session_id,
        &other_agent_id,
        "other",
        AgentTriggerBusyPolicy::Queue,
    );
    // The steer of the first trigger waits for the lane of the running turn.
    let held = runtime.provider_runtime_lanes.acquire(&run_id).await;

    tokio::time::timeout(PUMP_BOUND, runtime.pump_transport_runtime())
        .await
        .expect("the pump waits for a trigger that waits on a provider lane");
    until(|| {
        let other = trigger(&runtime, &session_id, "other");
        other.last_outcome() == Some(AgentTriggerOutcome::Queued) && !other.dispatch_in_flight()
    })
    .await
    .expect("another trigger waits behind the stuck one");
    assert!(
        trigger(&runtime, &session_id, "stuck").dispatch_in_flight(),
        "the stuck trigger keeps its claim until its firing is settled"
    );

    // Asking again neither fires the stuck trigger a second time nor loses it.
    tokio::time::timeout(PUMP_BOUND, runtime.pump_transport_runtime())
        .await
        .expect("the second pump waits");
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "stuck"), 1);
    assert_eq!(
        firings_of(&runtime, &session_id, &other_agent_id, "other"),
        1
    );

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &agent_id)
        .unwrap();
    drop(held);
    until(|| !trigger(&runtime, &session_id, "stuck").dispatch_in_flight())
        .await
        .expect("the stuck trigger settles once its lane is free");
    assert_eq!(trigger(&runtime, &session_id, "stuck").fired(), 1);
    assert_eq!(trigger(&runtime, &session_id, "other").fired(), 1);
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "stuck"), 1);
    assert_eq!(
        firings_of(&runtime, &session_id, &other_agent_id, "other"),
        1
    );
}

#[tokio::test]
async fn due_triggers_wait_for_a_place_when_every_place_is_taken() {
    let (_worktree, runtime, session_id, agent_id, _, _, _) = runtime_with_active_prompt().await;
    add_due_trigger(
        &runtime,
        &session_id,
        &agent_id,
        "waiting",
        AgentTriggerBusyPolicy::Queue,
    );
    let places = runtime.trigger_firings.hold_all_places();

    runtime.dispatch_due_agent_triggers(crate::session::unix_epoch_ms());
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(trigger(&runtime, &session_id, "waiting").dispatch_in_flight());
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "waiting"), 0);

    drop(places);
    tokio::time::timeout(PUMP_BOUND, runtime.trigger_firings_settled())
        .await
        .expect("the claimed trigger never fired");
    let fired = trigger(&runtime, &session_id, "waiting");
    assert!(!fired.dispatch_in_flight());
    assert_eq!(fired.fired(), 1);
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "waiting"), 1);
}

/// Prompts of `agent_id` waiting in its queue that came from a trigger, in queue order.
fn queued_trigger_ids(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
) -> Vec<String> {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    let (_, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, agent_id);
    queued
        .iter()
        .filter_map(|prompt| prompt.agent_trigger_id().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn triggers_of_one_agent_fire_in_claim_order_across_pumps() {
    let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
        runtime_with_active_prompt().await;
    // `first` and `second` are due at the first pump, `third` only at the second one.
    for (id, busy_policy, created_ms) in [
        ("first", AgentTriggerBusyPolicy::Steer, 1),
        ("second", AgentTriggerBusyPolicy::Queue, 1),
        ("third", AgentTriggerBusyPolicy::Queue, 1_000_000),
    ] {
        let mut session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        session.add_agent_trigger(
            AgentTrigger::schedule(
                id,
                &agent_id,
                "owner",
                crate::session::AgentSchedule::interval(60, AgentTriggerRepeat::Every),
                busy_policy,
                format!("fire {id}"),
                created_ms,
            )
            .expect("schedule should be valid"),
        );
        runtime.owned.session_store.restore_session(session);
    }
    // The steer of `first` waits for the lane of the running turn.
    let held = runtime.provider_runtime_lanes.acquire(&run_id).await;

    runtime.dispatch_due_agent_triggers(100_000);
    until(|| firings_of(&runtime, &session_id, &agent_id, "first") == 1)
        .await
        .expect("the first trigger is fired");
    runtime.dispatch_due_agent_triggers(2_000_000);
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        firings_of(&runtime, &session_id, &agent_id, "third"),
        0,
        "a trigger claimed later is fired before the one claimed before it"
    );
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "second"), 0);

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &agent_id)
        .unwrap();
    drop(held);
    tokio::time::timeout(PUMP_BOUND, runtime.trigger_firings_settled())
        .await
        .expect("the firings never end");
    assert_eq!(
        queued_trigger_ids(&runtime, &session_id, &agent_id)
            .into_iter()
            .filter(|id| id != "first")
            .collect::<Vec<_>>(),
        ["second", "third"],
        "the prompts reach the agent's queue in the order the triggers were claimed"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_launch_retry_rejected_for_lack_of_a_place_is_admitted_before_the_blocked_ones() {
    let (_worktree, runtime, session_id, agent_id, _, _, _) = runtime_with_active_prompt().await;
    // More blocked retries than places, all of them sorting before the one that is ready.
    let blocked = (0..task_work_flight::WORK_AT_ONCE)
        .map(|i| format!("provider-run-pump-stall-blocked-{i:02}"))
        .collect::<Vec<_>>();
    let ready_run_id = "provider-run-pump-stall-ready";
    let mut held = Vec::new();
    for run_id in &blocked {
        insert_run(&runtime, &session_id, &agent_id, run_id);
        schedule_launch_failure_retry(&runtime, run_id);
        held.push(runtime.provider_runtime_lanes.acquire(run_id).await);
    }
    insert_run(&runtime, &session_id, &agent_id, ready_run_id);
    schedule_launch_failure_retry(&runtime, ready_run_id);

    // Each pump finds the same blocked retries first; their lane waits run out between pumps.
    for _ in 0..3 {
        runtime.pump_transport_runtime().await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        if !retry_pending(&runtime, ready_run_id) {
            break;
        }
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }
    assert!(
        !retry_pending(&runtime, ready_run_id),
        "the ready retry was rejected for lack of a place on every pump"
    );
    for run_id in &blocked {
        assert!(
            retry_pending(&runtime, run_id),
            "{run_id}: a blocked retry is kept, not dropped"
        );
    }
    drop(held);
}

#[tokio::test]
async fn concurrent_agent_triggers_share_one_live_attachment() {
    let (_worktree, runtime, session_id, _, _, _, _) = runtime_with_active_prompt().await;
    for _ in 0..32 {
        let start = std::sync::Barrier::new(8);
        let ids = std::thread::scope(|scope| {
            let handles = (0..8)
                .map(|_| {
                    let start = &start;
                    let owned = &runtime.owned;
                    let session_id = &session_id;
                    scope.spawn(move || {
                        start.wait();
                        owned.ensure_agent_trigger_attachment(session_id).unwrap()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(ids.iter().all(|id| id == &ids[0]), "{ids:?}");
        runtime
            .owned
            .ensure_attachment_in_session(&session_id, &ids[0])
            .expect("the shared attachment survives every concurrent ensure");
        assert_eq!(
            runtime
                .owned
                .attachment_store
                .list_client_attachments(&format!("agent-schedules:{session_id}"))
                .len(),
            1,
            "one attachment owns every firing's input"
        );
        runtime.owned.detach(&ids[0]).unwrap();
    }
}
