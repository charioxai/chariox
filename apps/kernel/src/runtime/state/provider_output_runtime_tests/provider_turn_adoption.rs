//! Claude starts a turn of its own when a background command ends. The kernel adopts it as the
//! agent's running turn with a task (so its tools act for a turn), and its end settles that task.
use super::background_tool_settlement::{lifecycle, tool_chunk};
use super::child_finished::{team_with, Team};
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::provider::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
};
use crate::terminal::TerminalOutputKind;

async fn apply(team: &Team, batch: ProviderPromptSignalBatch) {
    team.runtime
        .apply_owned_structured_output_batch(&team.room, &team.child_run, Vec::new(), batch)
        .await
        .unwrap();
}

fn own_turn_start() -> ProviderPromptSignalBatch {
    ProviderPromptSignalBatch {
        provider_turn_started: true,
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
        ..Default::default()
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

#[tokio::test]
async fn a_held_childs_notification_turn_stays_with_its_held_task() {
    let team = idle_child("own-turn-held-child", true).await;
    assert!(team
        .runtime
        .owned
        .background_holds
        .holds_run(&team.child_run));
    apply(&team, own_turn_start()).await;
    assert!(active_prompt(&team).is_none());
    assert_eq!(
        task_of(&team, team.child_prompt.id()),
        Some(ExecutionState::Working)
    );
}
