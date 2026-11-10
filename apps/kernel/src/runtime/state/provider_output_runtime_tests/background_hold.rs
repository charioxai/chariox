//! MP-08 / MP-09 / MP-10 / MP-11: a sub-agent whose turn ends while a `run_in_background` call
//! still runs is not finished. The provider wakes it with a notification turn when the call
//! ends, and its parent hears of one `child_finished` that carries that turn's text.
use super::background_tool_settlement::{lifecycle, tool_chunk};
use super::child_finished::{child_finished, team_with, Team};
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;

const MINUTE_MS: u64 = 60_000;

fn text_chunk(text: &str) -> ProviderPromptChunk {
    ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: None,
        bytes: text.as_bytes().to_vec(),
    }
}

async fn apply(team: &Team, chunks: Vec<ProviderPromptChunk>) {
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
}

/// The child's one background command ended at `ended_at_ms`.
fn ended_at(ended_at_ms: u64) -> ProviderPromptChunk {
    let mut finished = lifecycle("claude_background_finished");
    finished["finished_at_ms"] = ended_at_ms.into();
    tool_chunk(&finished)
}

/// A child that answered and ended its turn while its background command still runs.
async fn child_with_a_call_running(label: &str) -> Team {
    let team = team_with(label, None, false).await;
    apply(
        &team,
        vec![
            tool_chunk(&lifecycle("claude_background_launched")),
            text_chunk("started the server"),
        ],
    )
    .await;
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    let settlement = team
        .runtime
        .owned
        .settle_agent_task(
            &team.room,
            &team.child,
            &team.child_prompt,
            &team.child_run,
            false,
        )
        .unwrap();
    assert!(
        settlement.is_none(),
        "the turn ended with a call running: {settlement:?}"
    );
    team
}

fn working(team: &Team) -> bool {
    team.runtime
        .owned
        .durable_state_store
        .agent_task(&team.task.task_id)
        .unwrap()
        .unwrap()
        .state
        == ExecutionState::Working
}

async fn sweep(team: &Team) {
    team.runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_turn_that_ends_with_a_background_call_running_leaves_the_child_working() {
    let team = child_with_a_call_running("background-hold-running").await;
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
    // The sweep does not take a held child for one that lost its executor.
    sweep(&team).await;
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
}

#[tokio::test]
async fn the_notification_turn_has_a_minute_to_start_after_the_last_call_ended() {
    let team = child_with_a_call_running("background-hold-grace").await;
    apply(&team, vec![ended_at(crate::session::unix_epoch_ms())]).await;
    sweep(&team).await;
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
}

#[tokio::test]
async fn a_call_that_ended_without_a_notification_turn_releases_the_child_after_the_grace() {
    let team = child_with_a_call_running("background-hold-released").await;
    apply(
        &team,
        vec![ended_at(crate::session::unix_epoch_ms() - 2 * MINUTE_MS)],
    )
    .await;
    sweep(&team).await;
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], true);
    assert_eq!(events[0].payload["outcome"]["kind"], "completed");
    assert!(events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("started the server"));
    sweep(&team).await;
    assert_eq!(child_finished(&team).len(), 1);
}

#[tokio::test]
async fn a_notification_turn_that_is_executing_is_held_past_the_grace_until_it_completes() {
    let team = child_with_a_call_running("background-hold-executing").await;
    // The call ended long ago, but the notification turn started and is still inspecting.
    apply(
        &team,
        vec![
            ended_at(crate::session::unix_epoch_ms() - 2 * MINUTE_MS),
            text_chunk("reading the server output"),
        ],
    )
    .await;
    sweep(&team).await;
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());

    apply(&team, vec![text_chunk("the server exited with code 0")]).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    let excerpt = events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap();
    assert!(
        excerpt.contains("the server exited with code 0"),
        "{excerpt}"
    );
}

#[tokio::test]
async fn the_notification_turn_is_part_of_the_one_finish() {
    let team = child_with_a_call_running("background-hold-notification").await;
    apply(
        &team,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    // The provider's turn about the ended call completes with no prompt of the kernel's.
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    let excerpt = events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap();
    assert!(excerpt.contains("started the server"), "{excerpt}");
    assert!(
        excerpt.contains("the server exited with code 0"),
        "{excerpt}"
    );
    sweep(&team).await;
    assert_eq!(child_finished(&team).len(), 1);
}

#[tokio::test]
async fn a_call_that_is_still_running_does_not_end_the_hold_at_a_completed_turn() {
    let team = child_with_a_call_running("background-hold-other-turn").await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
}

#[tokio::test]
async fn a_call_that_never_ends_releases_a_child_without_a_deadline_after_half_an_hour() {
    let team = child_with_a_call_running("background-hold-limit").await;
    team.runtime
        .owned
        .background_holds
        .held_since_for_test(&team.task.task_id, 1);
    sweep(&team).await;
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], true);
    let excerpt = events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap();
    assert!(excerpt.contains("started the server"), "{excerpt}");
    assert!(
        excerpt.contains("background command was still running"),
        "{excerpt}"
    );
}

#[tokio::test]
async fn a_deadline_keeps_the_hold_until_it_fires_and_then_says_the_call_still_ran() {
    let team = child_with_a_call_running("background-hold-deadline").await;
    let deadline_ms = crate::session::unix_epoch_ms() + 400;
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.child.clone(),
            deadline: deadline_ms,
        })
        .unwrap();
    team.runtime
        .owned
        .background_holds
        .held_since_for_test(&team.task.task_id, 1);
    sweep(&team).await;
    assert!(working(&team), "the deadline decides, not the half hour");
    assert!(child_finished(&team).is_empty());

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    team.runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["deadline_reached"], true);
    assert_eq!(events[0].payload["success"], false);
    assert!(events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("background command was still running"));
}
