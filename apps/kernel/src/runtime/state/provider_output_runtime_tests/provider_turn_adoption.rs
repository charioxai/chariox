//! Claude starts a turn of its own when a background command ends. The kernel adopts it as the
//! agent's running turn with a task (so its tools act for a turn), and its end settles that task.
use super::background_tool_settlement::{lifecycle, tool_chunk};
use super::child_finished::{child_finished, team_with, Team};
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::provider::{
    ProviderAssistantCompletion, ProviderOwnTurnSignal, ProviderPromptChunk,
    ProviderPromptSignalBatch,
};
use crate::runtime::prompt_state::{OwnTurnBinding, ProviderTurnAdoption};
use crate::terminal::TerminalOutputKind;

async fn apply(team: &Team, batch: ProviderPromptSignalBatch) {
    team.runtime
        .apply_owned_structured_output_batch(&team.room, &team.child_run, Vec::new(), batch)
        .await
        .unwrap();
}

fn own_turn_start() -> ProviderPromptSignalBatch {
    ProviderPromptSignalBatch {
        provider_own_turn: ProviderOwnTurnSignal::Running,
        chunks: vec![ProviderPromptChunk {
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: None,
            bytes: b"the background command finished".to_vec(),
        }],
        ..Default::default()
    }
}

fn own_turn_end() -> ProviderPromptSignalBatch {
    ProviderPromptSignalBatch {
        completions: vec![ProviderAssistantCompletion {
            message_id: "claude:own-turn:1".into(),
            completed_at_ms: crate::session::unix_epoch_ms(),
        }],
        prompt_completed: true,
        provider_own_turn: ProviderOwnTurnSignal::Ended,
        ..Default::default()
    }
}

/// Claude's own turn starts and ends before the kernel drains again.
fn own_turn_within_one_batch() -> ProviderPromptSignalBatch {
    ProviderPromptSignalBatch {
        provider_own_turn: ProviderOwnTurnSignal::StartedAndEnded,
        chunks: own_turn_start().chunks,
        ..own_turn_end()
    }
}

fn active_prompt(team: &Team) -> Option<crate::session::PromptQueueItem> {
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    team.runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.child)
}

fn task_of(team: &Team, prompt: &str) -> Option<ExecutionState> {
    team.runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, prompt)
        .unwrap()
        .map(|task| task.state)
}

/// The child's prompted turn ended and its task settled; Claude is idle with no kernel turn.
async fn idle_child(label: &str, background_call_running: bool) -> Team {
    let team = team_with(label, None, false).await;
    if background_call_running {
        apply(
            &team,
            ProviderPromptSignalBatch {
                chunks: vec![tool_chunk(&lifecycle("claude_background_launched"))],
                ..Default::default()
            },
        )
        .await;
        team.runtime
            .flush_provider_history(&team.child_run)
            .await
            .unwrap();
    }
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    team.runtime
        .owned
        .settle_agent_task(
            &team.room,
            &team.child,
            &team.child_prompt,
            &team.child_run,
            false,
        )
        .unwrap();
    assert!(active_prompt(&team).is_none());
    team
}

#[tokio::test]
async fn a_turn_claude_starts_on_its_own_runs_as_a_kernel_turn_with_a_task_and_settles_at_its_end()
{
    let team = idle_child("own-turn-adopted", false).await;
    apply(&team, own_turn_start()).await;
    let adopted = active_prompt(&team).expect("Claude's own turn is the agent's running turn");
    assert_eq!(adopted.status(), crate::session::PromptStatus::Running);
    assert_ne!(adopted.id(), team.child_prompt.id());
    assert_eq!(task_of(&team, adopted.id()), Some(ExecutionState::Working));

    apply(&team, own_turn_end()).await;
    // The pump settles a completed turn once its output has been quiet for a moment.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, false, false, false)
        .await
        .unwrap();
    assert!(
        active_prompt(&team).is_none(),
        "the turn's end leaves no turn behind"
    );
    assert_eq!(task_of(&team, adopted.id()), Some(ExecutionState::Done));
}

#[tokio::test]
async fn claudes_own_turn_never_displaces_a_running_kernel_prompt() {
    let team = team_with("own-turn-behind-kernel-prompt", None, false).await;
    let before = active_prompt(&team).expect("the child's kernel prompt runs");
    apply(&team, own_turn_start()).await;
    assert_eq!(active_prompt(&team).unwrap().id(), before.id());
    let tasks = team
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.room), Some(&team.child))
        .unwrap();
    assert_eq!(tasks.len(), 1, "no second task: {tasks:?}");
}

/// The quiet window after which the pump settles a completed turn, then that settlement.
async fn settle_after_quiet(team: &Team) {
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, false, false, false)
        .await
        .unwrap();
}

async fn events_inbox(
    team: &Team,
    task_id: &str,
    origin_prompt_id: &str,
) -> Result<serde_json::Value, crate::error::DaemonError> {
    let token = team
        .runtime
        .owned
        .provider_store
        .get_run(&team.child_run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    team.runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.inbox",
            serde_json::json!({"task_id": task_id, "origin_prompt_id": origin_prompt_id}),
        )
        .await
        .map(|result| result.payload)
}

// Review 5968062224 P2-1: a held child's notification turn had no running turn, so its tools
// failed with "no running turn". It runs under the held task's prompt: one task, one finish.
#[tokio::test]
async fn a_held_childs_notification_turn_runs_under_its_held_task_and_finishes_it_once() {
    let team = idle_child("own-turn-held-child", true).await;
    assert!(team
        .runtime
        .owned
        .background_holds
        .holds_run(&team.child_run));
    // Claude starts the notification turn once the background command has ended.
    let mut start = own_turn_start();
    let mut finished = lifecycle("claude_background_finished");
    finished["finished_at_ms"] = crate::session::unix_epoch_ms().into();
    start.chunks.insert(0, tool_chunk(&finished));
    apply(&team, start).await;
    let adopted = active_prompt(&team).expect("the notification turn is the child's running turn");
    assert_eq!(
        adopted.id(),
        team.child_prompt.id(),
        "it runs under the held task's prompt"
    );
    assert_eq!(adopted.status(), crate::session::PromptStatus::Running);
    let inbox = events_inbox(&team, &team.task.task_id, team.child_prompt.id())
        .await
        .expect("the held task's tools work in its notification turn");
    assert!(inbox["events"].is_array(), "{inbox}");
    let tasks = team
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.room), Some(&team.child))
        .unwrap();
    assert_eq!(tasks.len(), 1, "no second task: {tasks:?}");

    apply(&team, own_turn_end()).await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none());
    assert_eq!(
        task_of(&team, team.child_prompt.id()),
        Some(ExecutionState::Done)
    );
    assert!(!team
        .runtime
        .owned
        .is_held_for_background_calls(&team.task.task_id));
    let finished = child_finished(&team);
    assert_eq!(finished.len(), 1, "{finished:?}");
    let excerpt = finished[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap_or_default();
    assert!(
        excerpt.contains("the background command finished"),
        "the finish carries the notification turn: {excerpt}"
    );
}

/// The child's prompted turn has ended (its `result` was read) but the pump has not settled it
/// yet: it waits for its output to be quiet.
async fn child_with_a_settling_prompt(label: &str) -> Team {
    let team = team_with(label, None, false).await;
    apply(&team, own_turn_end()).await;
    assert_eq!(
        active_prompt(&team).map(|prompt| prompt.id().to_string()),
        Some(team.child_prompt.id().to_string()),
        "the prompted turn is still settling"
    );
    team
}

// Review 5968062224 P2-2 and 5968870730 P2-1: Claude starts its own turn right after the prompt's
// result, before the kernel settled that prompt. The start was lost; then its output was kept
// under the preceding turn. That prompt settles at once, and the own turn is adopted with its own
// task and history boundary: the parent reads the own turn's answer in its finish.
#[tokio::test]
async fn an_own_turn_that_starts_while_the_prompt_before_it_settles_is_adopted_with_its_own_answer()
{
    let team = child_with_a_settling_prompt("own-turn-while-settling").await;
    apply(&team, own_turn_start()).await;
    assert_eq!(
        task_of(&team, team.child_prompt.id()),
        Some(ExecutionState::Done),
        "the prompt before the own turn settled"
    );
    let adopted = active_prompt(&team).expect("the own turn is adopted");
    assert_ne!(adopted.id(), team.child_prompt.id());
    assert_eq!(adopted.status(), crate::session::PromptStatus::Running);
    let task = team
        .runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, adopted.id())
        .unwrap()
        .expect("the adopted turn has a task");
    assert_eq!(task.state, ExecutionState::Working);
    events_inbox(&team, &task.task_id, adopted.id())
        .await
        .expect("the adopted turn's tools work");

    apply(&team, own_turn_end()).await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none());
    assert_eq!(task_of(&team, adopted.id()), Some(ExecutionState::Done));
    let finished = child_finished(&team);
    let excerpt_of = |task_id: &str| {
        finished
            .iter()
            .find(|event| {
                event.payload["task_id"] == task_id
                    || event.payload["public_answer"]["task_id"] == task_id
            })
            .map(|event| {
                event.payload["public_answer"]["excerpt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
    };
    let own = excerpt_of(&task.task_id).expect("the parent hears the own turn's finish");
    assert!(
        own.contains("the background command finished"),
        "{own:?} in {finished:?}"
    );
    let before = excerpt_of(&team.task.task_id).unwrap_or_default();
    assert!(
        !before.contains("the background command finished"),
        "the own turn's output is not the preceding turn's answer: {before:?}"
    );
}

// The own turn can start and end right after the prompt's result: nothing is left running that
// the sweep would later end as interrupted.
#[tokio::test]
async fn an_own_turn_right_after_a_prompt_leaves_no_turn_behind() {
    let team = child_with_a_settling_prompt("own-turn-ends-while-settling").await;
    apply(&team, own_turn_start()).await;
    apply(&team, own_turn_end()).await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none(), "{:?}", active_prompt(&team));
    let tasks = team
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.room), Some(&team.child))
        .unwrap();
    assert!(
        tasks.iter().all(|task| task.state == ExecutionState::Done),
        "{tasks:?}"
    );
}

fn tasks(team: &Team) -> Vec<crate::durable_state::agent_lifecycle::AgentTaskExecution> {
    team.runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.room), Some(&team.child))
        .unwrap()
}

// Transition T9: the own turn starts and ends within one batch. It is adopted and completed by
// that batch: its task finishes with the turn's answer and no turn is left running.
#[tokio::test]
async fn an_own_turn_that_starts_and_ends_in_one_batch_is_adopted_and_completed() {
    let team = idle_child("own-turn-one-batch", false).await;
    apply(&team, own_turn_within_one_batch()).await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none(), "{:?}", active_prompt(&team));
    let own = tasks(&team)
        .into_iter()
        .find(|task| task.prompt_id != team.child_prompt.id())
        .expect("the own turn ran as a kernel turn with a task");
    assert_eq!(own.state, ExecutionState::Done);
    let finished = child_finished(&team);
    let excerpt = finished
        .iter()
        .find(|event| event.payload["child_task_id"] == own.task_id)
        .and_then(|event| event.payload["public_answer"]["excerpt"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        excerpt.contains("the background command finished"),
        "{excerpt:?} in {finished:?}"
    );
}

// Transitions T2 + T9: the prompt before has ended but not settled, and the own turn starts and
// ends in the next batch: that prompt settles, the own turn is adopted and completed.
#[tokio::test]
async fn an_own_turn_in_one_batch_after_a_settling_prompt_is_its_own_finished_turn() {
    let team = child_with_a_settling_prompt("own-turn-one-batch-while-settling").await;
    apply(&team, own_turn_within_one_batch()).await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none(), "{:?}", active_prompt(&team));
    let all = tasks(&team);
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(
        all.iter().all(|task| task.state == ExecutionState::Done),
        "{all:?}"
    );
}

// Transitions T3 + T4: a turn reported while a prompt runs waits with the binding of its first
// report; a later attempt keeps that binding (no recompute) and adopts under it once the agent
// has no prompt.
#[tokio::test]
async fn a_pending_own_turn_is_adopted_under_the_binding_of_its_first_report() {
    let team = team_with("own-turn-binding-kept", None, false).await;
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let owner = &team.runtime.owned.prompt_state_owner;
    let fresh = |id: &str| {
        crate::session::PromptQueueItem::new(
            id,
            "attachment",
            &team.child,
            "",
            crate::session::PromptStatus::Running,
        )
    };
    let held = OwnTurnBinding::Held {
        prompt_id: "held-prompt".into(),
    };
    assert_eq!(
        owner.adopt_provider_turn(&session, fresh("own-1"), &team.child_run, held.clone()),
        ProviderTurnAdoption::Pending { newly: true }
    );
    assert_eq!(
        owner.adopt_provider_turn(
            &session,
            fresh("own-2"),
            &team.child_run,
            OwnTurnBinding::Fresh
        ),
        ProviderTurnAdoption::Pending { newly: false }
    );
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    match owner.adopt_provider_turn(
        &session,
        fresh("own-3"),
        &team.child_run,
        OwnTurnBinding::Fresh,
    ) {
        ProviderTurnAdoption::Adopted { prompt, binding } => {
            assert_eq!(binding, held);
            assert_eq!(prompt.id(), "held-prompt");
        }
        other => panic!("not adopted: {other:?}"),
    }
}

// Transition T8: the adopted own turn's process exits. The ordinary failure path ends the turn and
// the record lets it go: nothing stays running as the provider's own turn.
#[tokio::test]
async fn an_adopted_own_turn_whose_process_exits_leaves_no_turn_behind() {
    let team = idle_child("own-turn-exits", false).await;
    apply(&team, own_turn_start()).await;
    let adopted = active_prompt(&team).expect("adopted");
    apply(
        &team,
        ProviderPromptSignalBatch {
            terminal_failure: Some("Claude Code exited before completing the active turn".into()),
            prompt_completed: true,
            ..Default::default()
        },
    )
    .await;
    settle_after_quiet(&team).await;
    assert!(active_prompt(&team).is_none(), "{:?}", active_prompt(&team));
    assert_ne!(task_of(&team, adopted.id()), Some(ExecutionState::Working));
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert!(!team
        .runtime
        .owned
        .prompt_state_owner
        .provider_turn_running(&session, &team.child));
}
