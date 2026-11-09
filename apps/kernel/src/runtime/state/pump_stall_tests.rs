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

fn insert_run(runtime: &KernelRuntimeState, session_id: &str, agent_id: &str, run_id: &str) {
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
fn schedule_launch_failure_retry(runtime: &KernelRuntimeState, run_id: &str) {
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

fn retry_pending(runtime: &KernelRuntimeState, run_id: &str) -> bool {
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

fn add_due_trigger(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    id: &str,
    busy_policy: AgentTriggerBusyPolicy,
) {
    let mut session = runtime.owned.session_store.get_session(session_id).unwrap();
    // Created long ago, so the first run is already due.
    session.add_agent_trigger(AgentTrigger::schedule(
        id,
        agent_id,
        "owner",
        60,
        AgentTriggerRepeat::Every,
        busy_policy,
        format!("fire {id}"),
        1,
    ));
    runtime.owned.session_store.restore_session(session);
}

fn trigger(runtime: &KernelRuntimeState, session_id: &str, id: &str) -> AgentTrigger {
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

fn firings_of(runtime: &KernelRuntimeState, session_id: &str, agent_id: &str, id: &str) -> usize {
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
    let (_worktree, runtime, session_id, agent_id, _, _, run_id) =
        runtime_with_active_prompt().await;
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
        &agent_id,
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
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "other"), 1);

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
    assert_eq!(firings_of(&runtime, &session_id, &agent_id, "other"), 1);
}
