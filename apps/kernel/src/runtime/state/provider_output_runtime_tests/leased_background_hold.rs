//! A child on another kernel whose turn ends while its provider call still runs in the
//! background is not finished: its home keeps its task Working until the provider's follow-up
//! turn ends on the worker, and tells the parent once, with that turn's answer.
use super::background_tool_settlement::{lifecycle, tool_chunk};
use super::child_finished::{child_finished, team_with, Team};
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::runtime::relay_peer_authority::{test_projection_authority, RemoteProjectionAuthority};
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk,
    LEASED_NOTIFICATION_COMPLETION_PREFIX,
};

const WORKER: &str = "worker-kernel-1";
const LEASED_AGENT: &str = "leased-child";

/// A child whose agent runs on `WORKER`; the fixture's run stands for the worker's run.
struct Leased {
    team: Team,
    worker_run: String,
}

async fn leased_child(label: &str) -> Leased {
    let team = team_with(label, None, false).await;
    let worker_run = team.child_run.clone();
    let projected_run =
        crate::provider::projected_leased_provider_run_id(LEASED_AGENT, &worker_run);
    let binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: WORKER.into(),
        worker_machine_id: "worker-machine-1".into(),
        execution_lease_id: "lease-1".into(),
        leased_agent_id: LEASED_AGENT.into(),
        active_worker_provider_run_id: Some(worker_run.clone()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
        worker_worktree_id: None,
    };
    let owned = &team.runtime.owned;
    owned
        .agent_store
        .bind_remote_execution(&team.child, binding)
        .unwrap();
    owned
        .bind_agent_task_run(
            &team.room,
            &team.child,
            team.child_prompt.id(),
            &projected_run,
        )
        .unwrap();
    owned
        .mark_active_prompt_delivery(
            &team.room,
            &team.child,
            team.child_prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(worker_run.clone()),
            None,
        )
        .unwrap();
    Leased { team, worker_run }
}

fn authority(leased: &Leased, prompt: Option<&str>, held: bool) -> RemoteProjectionAuthority {
    let mut authority = test_projection_authority(WORKER);
    // A drain names the binding the home holds now: its prompt has ended, so no run is bound.
    authority.expected_binding = Some(
        leased
            .team
            .runtime
            .owned
            .agent_store
            .get_agent(&leased.team.child)
            .unwrap()
            .remote_execution()
            .cloned()
            .unwrap(),
    );
    authority.expected_prompt_id = prompt.map(str::to_owned);
    authority.held_provider_run_id = held.then(|| leased.worker_run.clone());
    authority
}

fn output(
    kind: TerminalOutputKind,
    merge_key: Option<String>,
    bytes: &[u8],
) -> RelayProjectedOutputChunk {
    RelayProjectedOutputChunk {
        kind,
        merge_key,
        bytes: bytes.to_vec(),
    }
}

fn text(text: &str) -> RelayProjectedOutputChunk {
    output(TerminalOutputKind::ProviderOutput, None, text.as_bytes())
}

fn tool(payload: &serde_json::Value) -> RelayProjectedOutputChunk {
    let chunk = tool_chunk(payload);
    output(chunk.kind, chunk.merge_key, &chunk.bytes)
}

fn completion(message_id: &str, home_prompt_id: Option<&str>) -> RelayProjectedCompletion {
    RelayProjectedCompletion {
        message_id: message_id.into(),
        completed_at_ms: crate::session::unix_epoch_ms(),
        home_prompt_id: home_prompt_id.map(str::to_owned),
        provider_termination: None,
        failure: None,
    }
}

/// A drain scopes every completion to the prompt its leased agent last answered.
fn notification_end(leased: &Leased) -> RelayProjectedCompletion {
    completion(
        &format!("{LEASED_NOTIFICATION_COMPLETION_PREFIX}worker:1"),
        Some(leased.team.child_prompt.id()),
    )
}

async fn project(
    leased: &Leased,
    authority: RemoteProjectionAuthority,
    output_chunks: Vec<RelayProjectedOutputChunk>,
    completions: Vec<RelayProjectedCompletion>,
    provider_run: Option<crate::provider::RuntimeProviderRun>,
) {
    leased
        .team
        .runtime
        .project_remote_runtime_projection_event(
            authority,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: leased.team.room.clone(),
                home_agent_id: leased.team.child.clone(),
                provider_run_id: leased.worker_run.clone(),
                provider_run,
                prompts: vec![],
                output_chunks,
                notices: vec![],
                completions,
            },
        )
        .await
        .unwrap();
}

/// The child's turn ended on its worker with the provider's background call still running.
async fn child_with_a_call_running(label: &str) -> Leased {
    let leased = leased_child(label).await;
    let prompt = leased.team.child_prompt.id().to_owned();
    project(
        &leased,
        authority(&leased, Some(&prompt), false),
        vec![
            tool(&lifecycle("claude_background_launched")),
            text("started the helper"),
        ],
        vec![completion("m1", Some(&prompt))],
        None,
    )
    .await;
    leased
}

fn working(leased: &Leased) -> bool {
    leased
        .team
        .runtime
        .owned
        .durable_state_store
        .agent_task(&leased.team.task.task_id)
        .unwrap()
        .unwrap()
        .state
        == ExecutionState::Working
}

#[tokio::test]
async fn a_remote_child_that_ends_its_turn_with_a_call_running_stays_working() {
    let leased = child_with_a_call_running("leased-hold-running").await;
    assert!(working(&leased));
    assert!(child_finished(&leased.team).is_empty());
    assert!(
        leased
            .team
            .runtime
            .awaits_held_run(&leased.team.room, &leased.team.child),
        "the home keeps asking its worker for the provider's follow-up turn"
    );
    let (binding, run) = leased
        .team
        .runtime
        .remote_prompt_projection_drain_target(&leased.team.room, &leased.team.child)
        .expect("a held task drains its worker run with no prompt active");
    assert_eq!(binding.worker_kernel_id, WORKER);
    assert_eq!(run, leased.worker_run);
}

#[tokio::test]
async fn the_parent_hears_one_finish_with_the_follow_up_turns_answer() {
    let leased = child_with_a_call_running("leased-hold-follow-up").await;
    let mut ended = lifecycle("claude_background_finished");
    ended["finished_at_ms"] = crate::session::unix_epoch_ms().into();
    project(
        &leased,
        authority(&leased, None, true),
        vec![tool(&ended), text("HELPERRESULT42")],
        vec![notification_end(&leased)],
        None,
    )
    .await;
    let events = child_finished(&leased.team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], true);
    let excerpt = events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap();
    assert!(excerpt.contains("HELPERRESULT42"), "{excerpt}");
    assert!(!leased
        .team
        .runtime
        .awaits_held_run(&leased.team.room, &leased.team.child));
    leased
        .team
        .runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(child_finished(&leased.team).len(), 1);
}

#[tokio::test]
async fn a_follow_up_turn_that_has_not_ended_does_not_finish_the_child() {
    let leased = child_with_a_call_running("leased-hold-executing").await;
    let mut ended = lifecycle("claude_background_finished");
    ended["finished_at_ms"] = (crate::session::unix_epoch_ms() - 120_000).into();
    project(
        &leased,
        authority(&leased, None, true),
        vec![tool(&ended), text("reading the helper output")],
        vec![],
        None,
    )
    .await;
    // The call ended two minutes ago, but its notification turn is executing.
    leased
        .team
        .runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert!(working(&leased));
    assert!(child_finished(&leased.team).is_empty());
}

#[tokio::test]
async fn a_worker_run_that_ends_while_the_call_runs_ends_the_task_with_the_reason() {
    let leased = child_with_a_call_running("leased-hold-run-ended").await;
    let mut run = leased
        .team
        .runtime
        .owned
        .provider_store
        .get_run(&leased.worker_run)
        .unwrap();
    run.mark_ended();
    project(
        &leased,
        authority(&leased, None, true),
        vec![],
        vec![],
        Some(run),
    )
    .await;
    assert!(!working(&leased));
    let events = child_finished(&leased.team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], false);
    assert!(
        events[0].payload.to_string().contains("background command"),
        "the parent is told why: {}",
        events[0].payload
    );
}

#[tokio::test]
async fn a_worker_lost_while_the_call_runs_keeps_its_run_owed_a_stop() {
    let leased = child_with_a_call_running("leased-hold-worker-lost").await;
    let runtime = &leased.team.runtime;
    let task = runtime
        .owned
        .durable_state_store
        .agent_task(&leased.team.task.task_id)
        .unwrap()
        .unwrap();
    let binding = runtime
        .owned
        .agent_store
        .get_agent(&leased.team.child)
        .unwrap()
        .remote_execution()
        .cloned()
        .unwrap();
    runtime
        .end_turn_on_lost_worker(&task, &binding, 30_000)
        .await
        .unwrap();
    assert!(!working(&leased));
    let events = child_finished(&leased.team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], false);
    // The task is over, but the worker's run is not known to be stopped.
    let owed = runtime
        .owned
        .durable_state_store
        .unstopped_worker_runs()
        .unwrap();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert_eq!(owed[0].worker_provider_run_id, leased.worker_run);
    assert_eq!(owed[0].binding.execution_lease_id, "lease-1");
    assert_eq!(owed[0].home_prompt_id, leased.team.child_prompt.id());
}

/// Stop on a child whose turn ended with the provider's call still running: the home has no
/// prompt to cancel, so the worker is owed a stop for the held run.
#[tokio::test]
async fn stop_of_a_held_remote_child_owes_its_worker_a_stop() {
    let leased = child_with_a_call_running("leased-hold-user-stop").await;
    let runtime = &leased.team.runtime;
    let request = crate::local::CancelActivePromptRequest {
        session_id: leased.team.room.clone(),
        attachment_id: runtime
            .owned
            .attachment_store
            .list_session_attachment_ids(&leased.team.room)
            .remove(0),
        target_agent_id: Some(leased.team.child.clone()),
        expected_prompt_id: None,
    };
    runtime
        .cancel_parked_agent_task(&request, &leased.team.child)
        .await
        .unwrap()
        .expect("Stop ends the held task");
    assert!(!working(&leased));
    let owed = runtime
        .owned
        .durable_state_store
        .unstopped_worker_runs()
        .unwrap();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert_eq!(owed[0].worker_provider_run_id, leased.worker_run);
    assert_eq!(owed[0].binding.execution_lease_id, "lease-1");
    assert_eq!(owed[0].home_prompt_id, leased.team.child_prompt.id());
}

/// Stop ends the task first and releases its resources after; a lifecycle sweep that runs in
/// between must not drop the held run before the worker is owed its stop.
#[tokio::test]
async fn a_sweep_between_stop_and_its_cleanup_keeps_the_held_run_owed_a_stop() {
    let leased = child_with_a_call_running("leased-hold-stop-sweep").await;
    let runtime = &leased.team.runtime;
    let task = runtime
        .owned
        .durable_state_store
        .agent_task(&leased.team.task.task_id)
        .unwrap()
        .unwrap();
    let attachment = runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(&leased.team.room)
        .remove(0);
    let owner: String = runtime
        .owned
        .session_store
        .get_session(&leased.team.room)
        .unwrap()
        .owner_user_id()
        .into();
    let cancel = runtime.owned.cancel_initiator(&attachment).cancel_task(
        task.task_id.clone(),
        owner,
        task.revision,
    );
    let crate::durable_state::agent_lifecycle::Outcome::Task(cancelled) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(cancel)
        .unwrap()
    else {
        unreachable!()
    };
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    runtime
        .cancel_agent_task_resources(&cancelled)
        .await
        .unwrap();
    let owed = runtime
        .owned
        .durable_state_store
        .unstopped_worker_runs()
        .unwrap();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert_eq!(owed[0].worker_provider_run_id, leased.worker_run);
}

#[tokio::test]
async fn a_worker_that_lost_the_agent_ends_the_task_instead_of_waiting_for_it() {
    let leased = child_with_a_call_running("leased-hold-agent-lost").await;
    leased
        .team
        .runtime
        .end_held_tasks_after_drain_error(
            &leased.team.room,
            &leased.team.child,
            &crate::error::DaemonError::LeasedAgentNotFound {
                leased_agent_id: LEASED_AGENT.into(),
            },
        )
        .await;
    assert!(!working(&leased));
    let events = child_finished(&leased.team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], false);
}

#[tokio::test]
async fn a_drain_that_merely_failed_keeps_the_task_waiting() {
    let leased = child_with_a_call_running("leased-hold-drain-failed").await;
    leased
        .team
        .runtime
        .end_held_tasks_after_drain_error(
            &leased.team.room,
            &leased.team.child,
            &crate::error::DaemonError::LocalTransport {
                operation: "drain remote prompt projection",
                message: "timed out".into(),
            },
        )
        .await;
    assert!(working(&leased));
    assert!(child_finished(&leased.team).is_empty());
}

#[tokio::test]
async fn a_projection_for_another_run_is_not_admitted_as_the_held_run() {
    let leased = child_with_a_call_running("leased-hold-other-run").await;
    let mut other = authority(&leased, None, true);
    other.held_provider_run_id = Some("another-worker-run".into());
    project(
        &leased,
        other,
        vec![text("not mine")],
        vec![notification_end(&leased)],
        None,
    )
    .await;
    assert!(working(&leased));
    assert!(child_finished(&leased.team).is_empty());
}
