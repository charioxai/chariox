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

/// The child's run becomes a native Claude run whose transcript the bridge reads.
async fn native_child(team: &Team, dir: &std::path::Path) -> crate::provider::RuntimeProviderRun {
    std::fs::create_dir_all(dir).unwrap();
    let context = dir.join("context.txt");
    let events = dir.join("events.jsonl");
    let transcript = dir.join("session.jsonl");
    std::fs::write(&context, "").unwrap();
    std::fs::write(&transcript, "").unwrap();
    std::fs::write(
        &events,
        serde_json::json!({
            "hook_event_name": "SessionStart",
            "transcript_path": transcript.display().to_string(),
        })
        .to_string(),
    )
    .unwrap();
    let (child, child_run) = (team.child.clone(), team.child_run.clone());
    let room = team.room.clone();
    let env = std::collections::BTreeMap::from([
        (
            "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
            context.display().to_string(),
        ),
        (
            "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
            events.display().to_string(),
        ),
    ]);
    team.runtime
        .with_app_side_effect(move |app| {
            let request = crate::provider::LaunchProviderRequest::new(
                &room, "claude", "claude", "default", "haiku",
            )
            .with_agent_id(&child)
            .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
            let mut run = crate::provider::RuntimeProviderRun::new(
                &child_run,
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                    process_label: "native-background-hold".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: Vec::new(),
                    pty_env: env,
                    pty_env_remove: Vec::new(),
                    working_directory: None,
                    structured_endpoint: None,
                },
            );
            run.mark_running();
            app.providers_mut().insert_run_for_test(run.clone());
            app.update_provider_run_projection(run.clone());
            super::spawn_inert_pty_for_run(app, run.id());
            run
        })
        .await
}

/// The native bridge reads what the provider's transcript holds now.
async fn read_native_transcript(
    team: &Team,
    run: &crate::provider::RuntimeProviderRun,
    transcript: &std::path::Path,
    lines: &[serde_json::Value],
) {
    let text = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(transcript, text + "\n").unwrap();
    let (room, run_id, run) = (team.room.clone(), team.child_run.clone(), run.clone());
    let owned = &team.runtime.owned;
    team.runtime
        .with_app_side_effect(|app| {
            app.process_claude_native_bridge_with_handoff_for_runtime(&room, &run_id, &run, owned)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_native_notification_turn_that_executes_beyond_the_grace_is_held_until_it_completes() {
    let team = child_with_a_call_running("background-hold-native-executing").await;
    let dir = std::env::temp_dir().join(format!(
        "chariox-background-hold-native-{}",
        std::process::id()
    ));
    let run = native_child(&team, &dir).await;
    let transcript = dir.join("session.jsonl");
    // The call ended two minutes ago, and the provider's notification turn has since started.
    apply(
        &team,
        vec![ended_at(crate::session::unix_epoch_ms() - 2 * MINUTE_MS)],
    )
    .await;
    let line = |id: &str, text: &str| {
        serde_json::json!({
            "type": "assistant",
            "uuid": id,
            "sessionId": "native-hold-session",
            "message": {
                "id": id, "model": "claude-haiku-5-5", "role": "assistant",
                "stop_reason": null,
                "content": [{"type": "text", "text": text}],
            }
        })
    };
    read_native_transcript(
        &team,
        &run,
        &transcript,
        &[line("m1", "reading the server output")],
    )
    .await;
    sweep(&team).await;
    assert!(
        working(&team),
        "the executing notification turn is not released"
    );
    assert!(child_finished(&team).is_empty());

    // The notification turn ends with the provider's Stop hook, and its last answer reaches the
    // transcript after the hook, so the pump's final drain must capture it.
    let events = dir.join("events.jsonl");
    let final_line = line("m2", "the server exited with code 0").to_string();
    let (flushed, late) = final_line.split_at(final_line.len() / 2);
    std::fs::write(
        &transcript,
        format!("{}\n{flushed}", line("m1", "reading the server output")),
    )
    .unwrap();
    std::fs::write(
        &events,
        serde_json::json!({
            "hook_event_name": "Stop",
            "transcript_path": transcript.display().to_string(),
        })
        .to_string(),
    )
    .unwrap();
    let late_flush = {
        let (transcript, late) = (transcript.clone(), format!("{late}\n"));
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(transcript)
                .unwrap();
            std::io::Write::write_all(&mut file, late.as_bytes()).unwrap();
        })
    };
    team.runtime
        .pump_owned_provider_output(&team.room, &team.child_run, vec![], true)
        .await
        .unwrap();
    late_flush.await.unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    let excerpt = events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap();
    assert!(
        excerpt.contains("the server exited with code 0"),
        "{excerpt}"
    );
    sweep(&team).await;
    assert_eq!(child_finished(&team).len(), 1);
    let child_run = team.child_run.clone();
    team.runtime
        .with_app_side_effect(move |app| {
            let _ = app.pty_mut().remove_process(&child_run);
        })
        .await;
    let _ = std::fs::remove_dir_all(dir);
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
