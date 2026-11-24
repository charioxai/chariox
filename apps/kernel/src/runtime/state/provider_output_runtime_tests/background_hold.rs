//! MP-08 / MP-09 / MP-10 / MP-11: a sub-agent whose turn ends while a `run_in_background` call
//! still runs is not finished. The provider wakes it with a notification turn when the call
//! ends, and its parent hears of one `child_finished` that carries that turn's text.
use super::background_tool_settlement::{lifecycle, tool_chunk};
use super::child_finished::{child_finished, team_with, Team};
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
use crate::provider::{
    ProviderAssistantCompletion, ProviderOwnTurnSignal, ProviderPromptChunk,
    ProviderPromptSignalBatch,
};
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

// On a worker the held task is its home's: the provider's turn about the ended call is reported
// to the home as a completion record that names no prompt, and the worker releases nothing.
#[tokio::test]
async fn a_worker_reports_the_end_of_the_notification_turn_to_its_home() {
    let team = child_with_a_call_running("background-hold-worker-report").await;
    team.runtime
        .owned
        .provider_run_projection
        .mark_leased_provider_run(&team.child_run);
    let recipients = team
        .runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(&team.room);
    apply(
        &team,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    let reported = recipients
        .iter()
        .flat_map(|attachment| {
            team.runtime
                .owned
                .terminal_stream
                .drain_completion_records(&team.room, attachment)
        })
        .filter(|record| record.provider_run_id == team.child_run)
        .any(|record| {
            record
                .message_id
                .starts_with(crate::transport::relay_peer::LEASED_NOTIFICATION_COMPLETION_PREFIX)
        });
    assert!(reported, "the home never hears the turn ended");
    assert!(
        child_finished(&team).is_empty(),
        "a worker finishes no task"
    );
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
async fn a_running_call_has_no_implicit_deadline() {
    let team = child_with_a_call_running("background-hold-no-limit").await;
    assert!(!team
        .runtime
        .owned
        .held_task_due(
            &team.task,
            crate::session::unix_epoch_ms() + 120 * MINUTE_MS,
        )
        .unwrap());
    sweep(&team).await;
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
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
            deadline: Some(deadline_ms),
            inherited: Vec::new(),
        })
        .unwrap();
    sweep(&team).await;
    assert!(
        working(&team),
        "the explicit deadline keeps the task until it fires"
    );
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

#[tokio::test]
async fn kwait_native_helpers_keep_the_child_working_through_interim_notifications() {
    let team = team_with("kwait-native-helper", None, false).await;
    let dir = std::env::temp_dir().join(format!("kwait-native-helper-{}", std::process::id()));
    let run = native_child(&team, &dir).await;
    let transcript = dir.join("session.jsonl");
    let lines = vec![
        serde_json::json!({"type":"assistant", "uuid":"helper-start", "message":{"content":[{
            "type":"tool_use", "id":"helper", "name":"Agent", "input":{"prompt":"Inspect the project", "run_in_background":true}
        }]}}),
        serde_json::json!({"type":"user", "uuid":"helper-launched", "message":{"content":[{
            "type":"tool_result", "tool_use_id":"helper", "content":"Async agent launched successfully. agentId: a123"
        }]}, "toolUseResult":{"isAsync":true,"agentId":"a123"}}),
        serde_json::json!({"type":"assistant", "uuid":"interim-answer", "message":{"content":[{"type":"text","text":"My helper is still inspecting."}]}}),
    ];
    read_native_transcript(&team, &run, &transcript, &lines).await;
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
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
        "a provider helper still runs: {settlement:?}"
    );
    assert!(working(&team));
    assert!(child_finished(&team).is_empty());
    let mut lines = lines;
    lines.push(serde_json::json!({"type":"user","uuid":"helper-progress", "message":{"content":
        "<task-notification><task-id>a123</task-id><tool-use-id>helper</tool-use-id><status>running</status><summary>Still inspecting</summary></task-notification>"}}));
    read_native_transcript(&team, &run, &transcript, &lines).await;
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    assert!(
        working(&team),
        "an interim notification cannot finish the child"
    );
    assert!(child_finished(&team).is_empty());
    lines.push(serde_json::json!({"type":"user","uuid":"helper-done", "message":{"content":
        "<task-notification><task-id>a123</task-id><tool-use-id>helper</tool-use-id><status>completed</status><summary>Inspection complete</summary></task-notification>"}}));
    lines.push(serde_json::json!({"type":"assistant","uuid":"final-answer", "message":{"content":[{"type":"text","text":"The helper completed its inspection."}]}}));
    read_native_transcript(&team, &run, &transcript, &lines).await;
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("helper completed"));
    let child_run = team.child_run.clone();
    team.runtime
        .with_app_side_effect(move |app| {
            let _ = app.pty_mut().remove_process(&child_run);
        })
        .await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn kwait_held_background_work_keeps_the_provider_in_the_output_pump() {
    let team = child_with_a_call_running("kwait-held-output-pump").await;
    let owned = &team.runtime.owned;
    let session = owned.session_store.get_session(&team.room).unwrap();
    assert!(owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.child)
        .is_none());
    assert!(
        !owned
            .git_turn_snapshots
            .provider_run_ids_for_session(&team.room)
            .contains(&team.child_run),
        "the fixture must exercise held work, not a pending snapshot"
    );
    let run = owned.provider_store.get_run(&team.child_run).unwrap();
    assert!(run.client_interface().is_chariox());
    assert!(!crate::provider::provider_run_uses_claude_native_bridge(
        &run
    ));
    let selected = super::super::provider_output_runtime::provider_run_ids_for_owned_output_pump(
        owned, &session,
    );
    assert!(
        selected.contains(&team.child_run),
        "the held child's completion frames must still be drained: {selected:?}"
    );
    apply(
        &team,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the background work completed"),
        ],
    )
    .await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
        .await
        .unwrap();
    assert!(!team
        .runtime
        .owned
        .is_held_for_background_calls(&team.task.task_id));
}

#[tokio::test]
async fn kwait_parked_background_work_is_selected_and_resumed_by_both_output_pumps() {
    for structured in [false, true] {
        let team = child_with_a_call_running(&format!("kwait-parked-output-{structured}")).await;
        let owned = &team.runtime.owned;
        let parked = owned
            .provider_store
            .park_run_provider_only(&team.room, &team.child_run)
            .unwrap()
            .into_run();
        owned.provider_run_projection.update(parked);
        let session = owned.session_store.get_session(&team.room).unwrap();
        let selected =
            super::super::provider_output_runtime::provider_run_ids_for_owned_output_pump(
                owned, &session,
            );
        assert!(selected.contains(&team.child_run));
        if structured {
            team.runtime
                .pump_owned_structured_provider_output(&team.room, &team.child_run, Vec::new())
                .await
                .unwrap();
        } else {
            team.runtime
                .pump_owned_provider_output(&team.room, &team.child_run, Vec::new(), false)
                .await
                .unwrap();
        }
        assert_eq!(
            owned
                .provider_store
                .get_run(&team.child_run)
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Running,
            "parked background work still needs output polling"
        );
        assert!(owned.is_held_for_background_calls(&team.task.task_id));
        apply(
            &team,
            vec![
                ended_at(crate::session::unix_epoch_ms()),
                text_chunk("the parked background work completed"),
            ],
        )
        .await;
        team.runtime
            .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, false)
            .await
            .unwrap();
        assert!(!owned.is_held_for_background_calls(&team.task.task_id));
    }
}

#[tokio::test]
async fn a_long_running_background_job_stays_working_after_a_message_turn() {
    let team = child_with_a_call_running("background-hold-long-message").await;
    let message = crate::session::PromptQueueItem::new(
        "housekeeping-message",
        "message-attachment",
        &team.child,
        "Report status and keep the original job running",
        crate::session::PromptStatus::Completed,
    );
    team.runtime
        .owned
        .ensure_agent_task(&team.room, &team.child, &message, Some(&team.child_run))
        .unwrap();
    team.runtime
        .owned
        .settle_agent_task(&team.room, &team.child, &message, &team.child_run, false)
        .unwrap();
    assert!(
        !team
            .runtime
            .owned
            .held_task_due(
                &team.task,
                crate::session::unix_epoch_ms() + 120 * MINUTE_MS,
            )
            .unwrap(),
        "a live command has no implicit half-hour deadline",
    );
    sweep(&team).await;
    assert!(
        working(&team),
        "a status message cannot finish the original background job"
    );
    assert!(child_finished(&team)
        .iter()
        .all(|event| { event.payload["child_task_id"] != team.task.task_id }));
}

async fn status_message(team: &Team, reply: bool) -> String {
    let mut event = crate::durable_state::agent_lifecycle::occurrence(
        &team.room,
        &team.child,
        &team.parent,
        "status-message",
        "message",
        serde_json::json!({"message": "Report status; keep the original job running"}),
    );
    event.reply_requested = reply;
    // Only an urgent message starts its idle recipient.
    event.urgent = true;
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(event))
        .unwrap();
    team.runtime
        .deliver_agent_inbox_for_test(&team.room, &team.child)
        .await
        .unwrap();
    let event = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.child, 0)
        .unwrap()
        .remove(0);
    event.prompt_id.expect("message starts its own turn")
}

#[tokio::test]
async fn a_message_reply_does_not_wait_for_an_earlier_tasks_background_command() {
    let team = child_with_a_call_running("background-hold-message-reply").await;
    let prompt = status_message(&team, true).await;
    apply(
        &team,
        vec![text_chunk("The original command is still running")],
    )
    .await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, true)
        .await
        .unwrap();
    let reply = team
        .runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, &prompt)
        .unwrap()
        .unwrap();
    assert_eq!(
        reply.state,
        ExecutionState::Done,
        "a reply does not own the earlier command"
    );
    assert!(
        working(&team),
        "the message reply preserves the original job"
    );
}

#[tokio::test]
async fn a_status_message_is_not_the_background_completion_notification() {
    let team = child_with_a_call_running("background-hold-message-notification").await;
    status_message(&team, false).await;
    apply(
        &team,
        vec![text_chunk("The original command is still running")],
    )
    .await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, true)
        .await
        .unwrap();
    // No completion notification followed the call's end. The unrelated status reply must not
    // suppress the normal grace-period recovery of the original task.
    apply(
        &team,
        vec![ended_at(crate::session::unix_epoch_ms() - 2 * MINUTE_MS)],
    )
    .await;
    sweep(&team).await;
    assert!(
        !working(&team),
        "the status reply was not an executing completion notification"
    );
    assert!(child_finished(&team)
        .iter()
        .any(|event| event.payload["child_task_id"] == team.task.task_id));
}

#[tokio::test]
async fn a_messages_own_background_command_holds_only_its_own_task() {
    let team = child_with_a_call_running("background-hold-message-own-call").await;
    let prompt = status_message(&team, false).await;
    let mut launched = lifecycle("claude_background_launched");
    launched["id"] = "message-job".into();
    apply(
        &team,
        vec![tool_chunk(&launched), text_chunk("started the second job")],
    )
    .await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, true)
        .await
        .unwrap();
    let message_task = || {
        team.runtime
            .owned
            .durable_state_store
            .agent_turn_task(&team.room, &team.child, &prompt)
            .unwrap()
            .unwrap()
    };
    assert_eq!(message_task().state, ExecutionState::Working);
    assert!(working(&team));

    apply(
        &team,
        vec![ended_at(crate::session::unix_epoch_ms() - 2 * MINUTE_MS)],
    )
    .await;
    sweep(&team).await;
    assert!(
        !working(&team),
        "a newer command cannot hold the older task"
    );
    assert_eq!(
        message_task().state,
        ExecutionState::Working,
        "the message still owns its own running command"
    );

    let mut ended = lifecycle("claude_background_finished");
    ended["id"] = "message-job".into();
    ended["finished_at_ms"] = (crate::session::unix_epoch_ms() - 2 * MINUTE_MS).into();
    apply(&team, vec![tool_chunk(&ended)]).await;
    sweep(&team).await;
    assert_eq!(message_task().state, ExecutionState::Done);
}

// Review 5968870730 P2-2: two held tasks on one run. The call of the one that is not first ends,
// and Claude's notification turn for it starts in the same batch: that turn runs under the task
// whose call ended, and its tools act for that task.
#[tokio::test]
async fn a_notification_turn_runs_under_the_held_task_whose_call_ended_in_its_batch() {
    let team = child_with_a_call_running("background-hold-two-held").await;
    let message_prompt = status_message(&team, false).await;
    let mut launched = lifecycle("claude_background_launched");
    launched["id"] = "message-job".into();
    apply(&team, vec![tool_chunk(&launched)]).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, true)
        .await
        .unwrap();
    let message_task = team
        .runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, &message_prompt)
        .unwrap()
        .unwrap();
    assert_eq!(message_task.state, ExecutionState::Working);
    assert!(working(&team));
    assert!(
        message_task.task_id < team.task.task_id,
        "the fixture needs the original task to be the second one held"
    );

    // The original task's call ends and Claude's turn about it starts, in one batch.
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks: vec![
                    ended_at(crate::session::unix_epoch_ms()),
                    text_chunk("the server exited with code 0"),
                ],
                provider_own_turn: ProviderOwnTurnSignal::Running,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let running = team
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.child)
        .expect("the notification turn runs");
    assert_eq!(running.id(), team.child_prompt.id());
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
            serde_json::json!({"task_id": team.task.task_id, "origin_prompt_id": team.child_prompt.id()}),
        )
        .await
        .expect("the turn acts for the task whose call ended");
}

/// One batch of Claude's output that reports its own turn as `signal`; `result` reads a turn's
/// end.
async fn apply_own_turn(
    team: &Team,
    signal: ProviderOwnTurnSignal,
    result: bool,
    chunks: Vec<ProviderPromptChunk>,
) {
    let completions = if result {
        vec![ProviderAssistantCompletion {
            message_id: format!(
                "claude:{}:{}",
                team.child_run,
                crate::session::unix_epoch_ms()
            ),
            completed_at_ms: crate::session::unix_epoch_ms(),
        }]
    } else {
        Vec::new()
    };
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks,
                completions,
                prompt_completed: result,
                provider_own_turn: signal,
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

fn running_prompt(team: &Team) -> Option<String> {
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
        .map(|prompt| prompt.id().to_string())
}

fn finishes_of_task(team: &Team) -> Vec<String> {
    child_finished(team)
        .iter()
        .filter(|event| event.payload["child_task_id"] == team.task.task_id)
        .map(|event| {
            event.payload["public_answer"]["excerpt"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

// Transition T2 + H1 (review 5969266242-1): two held tasks on one run; the status message's turn
// has ended but not settled when the original task's call ends and Claude's turn about it
// starts. The turn waits for that settlement, and is then adopted under the binding chosen from
// its first batch: the original task, not the first one held.
#[tokio::test]
async fn a_held_binding_chosen_while_the_prompt_before_settles_is_kept_through_the_settle() {
    let team = child_with_a_call_running("background-hold-binding-through-settle").await;
    let message_prompt = status_message(&team, false).await;
    let mut launched = lifecycle("claude_background_launched");
    launched["id"] = "message-job".into();
    apply(&team, vec![tool_chunk(&launched)]).await;
    apply_own_turn(&team, ProviderOwnTurnSignal::None, true, Vec::new()).await;
    assert_eq!(
        running_prompt(&team).as_deref(),
        Some(message_prompt.as_str())
    );

    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    let message_task = team
        .runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, &message_prompt)
        .unwrap()
        .unwrap();
    assert!(
        message_task.task_id < team.task.task_id,
        "the fixture needs the original task to be the second one held"
    );
    assert_eq!(
        message_task.state,
        ExecutionState::Working,
        "the message's own command still runs"
    );
    assert_eq!(
        running_prompt(&team).as_deref(),
        Some(team.child_prompt.id()),
        "the notification turn runs for the task whose call ended"
    );
}

/// The child's own task is held, a status message's turn runs, and the original call ends:
/// Claude's notification turn waits for adoption behind the message's prompt.
async fn held_turn_pending_behind_a_message(label: &str) -> (Team, String) {
    let team = child_with_a_call_running(label).await;
    let message_prompt = status_message(&team, false).await;
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    assert_eq!(
        running_prompt(&team).as_deref(),
        Some(message_prompt.as_str())
    );
    assert!(working(&team));
    (team, message_prompt)
}

/// What history records as the message prompt's answer so far.
fn message_answer(team: &Team, message_prompt: &str) -> String {
    let entries = team
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&team.room, Some(&team.child))
        .unwrap();
    crate::runtime::state::agent_task_projection::task_public_outputs(
        &entries,
        message_prompt,
        Some(&team.child_run),
    )
    .join("\n")
}

// A prompt write cannot end the notification turn it joins (review 5972527077).
// The held task keeps every batch through the result; the prompt reads from its fold boundary.
async fn folded_turn_finishes_at_result(signal: ProviderOwnTurnSignal) {
    let (team, message_prompt) = held_turn_pending_behind_a_message("background-hold-folded").await;
    team.runtime
        .owned
        .mark_active_prompt_delivery(
            &team.room,
            &team.child,
            &message_prompt,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(team.child_run.clone()),
            None,
        )
        .unwrap();
    apply_own_turn(&team, signal, false, vec![text_chunk("more notification")]).await;
    assert!(
        working(&team),
        "a prompt write does not finish the notification turn"
    );
    assert!(finishes_of_task(&team).is_empty());
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Ended,
        true,
        vec![text_chunk(
            "the log shows a clean shutdown; status: the server is done",
        )],
    )
    .await;
    assert!(!working(&team), "the held task finished at the result");
    let finishes = finishes_of_task(&team);
    assert_eq!(finishes.len(), 1, "{finishes:?}");
    for text in [
        "started the server",
        "the server exited with code 0",
        "more notification",
        "the log shows a clean shutdown",
    ] {
        assert!(finishes[0].contains(text), "missing {text}: {finishes:?}");
    }
    let answer = message_answer(&team, &message_prompt);
    assert!(answer.contains("status: the server is done"), "{answer}");
    assert!(
        !answer.contains("the server exited with code 0"),
        "{answer}"
    );
    sweep(&team).await;
    assert_eq!(finishes_of_task(&team).len(), 1);
}

#[tokio::test]
async fn a_prompt_written_into_a_pending_held_turn_finishes_only_after_its_result() {
    folded_turn_finishes_at_result(ProviderOwnTurnSignal::None).await;
}

#[tokio::test]
async fn a_delivered_prompt_folds_inside_the_held_notification_until_its_result() {
    folded_turn_finishes_at_result(ProviderOwnTurnSignal::Running).await;
}

// Review 5975524512: the adapter ends the notification before the queued prompt's result.
#[tokio::test]
async fn a_folded_held_answer_excludes_the_separate_queued_prompt_reply_and_calls() {
    separate_queued_answer(false).await;
}

#[tokio::test]
async fn a_prompt_written_before_the_notification_keeps_its_separate_answer() {
    separate_queued_answer(true).await;
}

async fn separate_queued_answer(written_before_notification: bool) {
    let team = child_with_a_call_running("background-hold-separate-queued-reply").await;
    // The output pump holds the run's lane while the next prompt waits for its actual write.
    let mut lane = Some(
        team.runtime
            .provider_runtime_lanes
            .acquire(&team.child_run)
            .await,
    );
    let message_prompt = status_message(&team, false).await;
    if written_before_notification {
        drop(lane.take());
        wait_for_prompt_delivery(&team, &message_prompt).await;
    }
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("notification began"),
        ],
    )
    .await;
    if !written_before_notification {
        drop(lane.take());
        wait_for_prompt_delivery(&team, &message_prompt).await;
    }
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![text_chunk("notification continued after the queued write")],
    )
    .await;
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Ended,
        false,
        vec![text_chunk("notification ended before the queued reply")],
    )
    .await;
    assert!(!working(&team));
    assert_eq!(finishes_of_task(&team).len(), 1);
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::None,
        true,
        vec![
            text_chunk("SEPARATE_QUEUED_REPLY"),
            tool_chunk(&serde_json::json!({
                "id": "queued-only-kernel-call",
                "tool": "chariox_kernel_request",
                "status": "completed",
                "input": {"request": {"GetSessionState": {"session_id": team.room}}},
            })),
        ],
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            apply_own_turn(&team, ProviderOwnTurnSignal::None, false, Vec::new()).await;
            if team
                .runtime
                .owned
                .durable_state_store
                .agent_turn_task(&team.room, &team.child, &message_prompt)
                .unwrap()
                .unwrap()
                .state
                == ExecutionState::Done
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the separate queued reply must settle after its output becomes quiet");
    assert_eq!(
        team.runtime
            .owned
            .durable_state_store
            .agent_turn_task(&team.room, &team.child, &message_prompt)
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Done,
        "the separate queued reply must finish normally",
    );
    let held_answer = message_answer(&team, team.child_prompt.id());
    let queued_answer = message_answer(&team, &message_prompt);
    assert!(
        held_answer.contains("notification ended before the queued reply"),
        "{held_answer}"
    );
    assert!(
        queued_answer.contains("SEPARATE_QUEUED_REPLY"),
        "{queued_answer}"
    );
    if written_before_notification {
        assert!(
            !queued_answer.contains("notification began")
                && !queued_answer.contains("notification continued")
                && !queued_answer.contains("notification ended"),
            "the independent reply must exclude the held notification: {queued_answer}"
        );
    }
    let entries = team
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&team.room, Some(&team.child))
        .unwrap();
    let held_entries =
        super::super::agent_task_projection::prompt_turn_entries(&entries, team.child_prompt.id());
    let queued_entries =
        super::super::agent_task_projection::prompt_turn_entries(&entries, &message_prompt);
    assert!(queued_entries
        .iter()
        .any(|entry| entry.text.contains("queued-only-kernel-call")));
    let held_has_queued_call = held_entries
        .iter()
        .any(|entry| entry.text.contains("queued-only-kernel-call"));
    assert!(
        !held_answer.contains("SEPARATE_QUEUED_REPLY") && !held_has_queued_call,
        "the held task's answer and completed-call evidence must close before the separate reply: {held_answer}; queued_call={held_has_queued_call}",
    );
    assert_eq!(finishes_of_task(&team).len(), 1);
}

// Transition T6 (review 5971171601): the notification turn ends before it was adopted. The held
// task finishes at that end, after the ending batch is in history: its one finish carries the
// output of every batch of the turn, and the message's answer carries none of it.
#[tokio::test]
async fn a_pending_held_turn_that_ends_finishes_the_held_task() {
    let (team, message_prompt) =
        held_turn_pending_behind_a_message("background-hold-pending-ended").await;
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Ended,
        true,
        vec![text_chunk("the log shows a clean shutdown")],
    )
    .await;
    assert!(!working(&team), "the held task finished");
    let finishes = finishes_of_task(&team);
    assert_eq!(finishes.len(), 1, "{finishes:?}");
    assert!(
        finishes[0].contains("the server exited with code 0")
            && finishes[0].contains("the log shows a clean shutdown"),
        "the finish carries the whole notification turn: {finishes:?}"
    );
    let answer = message_answer(&team, &message_prompt);
    assert!(
        !answer.contains("the server exited with code 0"),
        "{answer}"
    );
}

// Transition T9 still pending (review 5971171601): the notification turn starts and ends within
// one batch while the message's prompt runs. The held task's finish carries that batch's text.
#[tokio::test]
async fn a_held_turn_that_starts_and_ends_behind_a_message_finishes_with_its_output() {
    let team = child_with_a_call_running("background-hold-pending-one-batch").await;
    let message_prompt = status_message(&team, false).await;
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::StartedAndEnded,
        true,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    assert!(!working(&team), "the held task finished");
    let finishes = finishes_of_task(&team);
    assert_eq!(finishes.len(), 1, "{finishes:?}");
    assert!(
        finishes[0].contains("the server exited with code 0"),
        "{finishes:?}"
    );
    let answer = message_answer(&team, &message_prompt);
    assert!(
        !answer.contains("the server exited with code 0"),
        "{answer}"
    );
}

// P8 (review 5971171601): the hold's grace runs out while the notification turn still streams,
// pending behind the message. The sweep leaves the held task to the turn's end.
#[tokio::test]
async fn the_sweep_leaves_a_held_task_whose_notification_turn_waits_for_adoption() {
    let (team, _) = held_turn_pending_behind_a_message("background-hold-pending-sweep").await;
    team.runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms() + 2 * MINUTE_MS)
        .await
        .unwrap();
    assert!(
        working(&team),
        "the sweep released a held task whose turn still runs"
    );
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Ended,
        true,
        vec![text_chunk("the log shows a clean shutdown")],
    )
    .await;
    let finishes = finishes_of_task(&team);
    assert_eq!(finishes.len(), 1, "{finishes:?}");
    assert!(
        finishes[0].contains("the log shows a clean shutdown"),
        "{finishes:?}"
    );
}

// Transitions T7 + H2 + H3 (review 5969266242-2): a status message's turn comes between the
// held task's turn and its notification turn. The held task's one finish reads both of its
// segments (the launch turn and the resumed notification turn) and not the message's reply.
#[tokio::test]
async fn a_held_tasks_finish_reads_its_resumed_turn_past_a_message_in_between() {
    let team = child_with_a_call_running("background-hold-resumed-segment").await;
    status_message(&team, false).await;
    apply(&team, vec![text_chunk("status: the server still runs")]).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, true, false, true)
        .await
        .unwrap();
    assert_eq!(running_prompt(&team), None);

    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the server exited with code 0"),
        ],
    )
    .await;
    assert_eq!(
        running_prompt(&team).as_deref(),
        Some(team.child_prompt.id())
    );
    apply_own_turn(&team, ProviderOwnTurnSignal::Ended, true, Vec::new()).await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, false, false, false)
        .await
        .unwrap();
    assert!(!working(&team));
    let finishes = finishes_of_task(&team);
    assert_eq!(finishes.len(), 1, "{finishes:?}");
    assert!(finishes[0].contains("started the server"), "{finishes:?}");
    assert!(
        finishes[0].contains("the server exited with code 0"),
        "the finish carries the resumed notification turn: {finishes:?}"
    );
    assert!(
        !finishes[0].contains("status: the server still runs"),
        "the message's reply is not the held task's answer: {finishes:?}"
    );
}

// A cancelled/failed provider reports a completed batch with no own-turn signal. That explicit
// end must clear a pending binding too; an empty poll with None cannot end it.
#[tokio::test]
async fn a_pending_held_turn_is_released_when_the_provider_ends_without_its_own_result() {
    let (team, _) = held_turn_pending_behind_a_message("background-hold-pending-cancelled").await;
    apply_own_turn(&team, ProviderOwnTurnSignal::None, true, Vec::new()).await;
    assert!(
        !working(&team),
        "the ended provider cannot retain a pending held turn"
    );
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert!(
        !team.runtime.owned.prompt_state_owner.provider_turn_pending(
            &session,
            &team.child,
            &team.child_run,
        ),
        "an ended own turn cannot be adopted after the prompt settles"
    );
    assert_eq!(finishes_of_task(&team).len(), 1);
    sweep(&team).await;
    assert!(
        !team
            .runtime
            .owned
            .prompt_state_owner
            .provider_turn_running(&session, &team.child,),
        "the sweep cannot adopt the ended notification turn"
    );
    assert_eq!(finishes_of_task(&team).len(), 1);
}

#[tokio::test]
async fn restart_interrupts_a_provider_started_held_notification_and_tells_its_parent() {
    let mut team = child_with_a_call_running("background-hold-restart-notification").await;
    apply_own_turn(
        &team,
        ProviderOwnTurnSignal::Running,
        false,
        vec![
            ended_at(crate::session::unix_epoch_ms()),
            text_chunk("the notification started"),
        ],
    )
    .await;
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let active = team
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.child)
        .unwrap();
    assert_eq!(active.id(), team.child_prompt.id());
    assert!(
        active.durable_delivery_phase().is_none(),
        "the provider started this turn without a kernel dispatch"
    );

    // A cold restart keeps the prompt and task, but neither the process nor the in-memory hold.
    let ended = team
        .runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&team.room, &team.child_run)
        .unwrap()
        .into_run();
    team.runtime.owned.provider_run_projection.update(ended);
    team.runtime.owned.background_holds = Default::default();
    team.runtime.recover_durable_runtime_after_restart().await;

    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert!(
        team.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &team.child)
            .is_none(),
        "a provider-started notification cannot remain active after its provider was lost"
    );
    assert!(!working(&team));
    let events = child_finished(&team);
    assert_eq!(
        events.len(),
        1,
        "the parent must receive one terminal result: {events:?}"
    );
    assert_eq!(
        events[0].payload["outcome"]["kind"],
        "interrupted_by_restart"
    );
    assert_eq!(events[0].payload["outcome"]["retryable"], true);
    assert!(!events[0].payload["outcome"]["reason"]
        .as_str()
        .unwrap()
        .is_empty());
    sweep(&team).await;
    assert_eq!(child_finished(&team).len(), 1);
}

async fn wait_for_prompt_delivery(team: &Team, message_prompt: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let session = team
                .runtime
                .owned
                .session_store
                .get_session(&team.room)
                .unwrap();
            if team
                .runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &team.child)
                .is_some_and(|prompt| {
                    prompt.id() == message_prompt
                        && prompt.durable_delivery_phase()
                            == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                })
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the queued prompt must pass through the provider write and receipt");
}

// Claude can consume a background notification inside the already-open prompt turn:
// no new init/result boundary exists, so its one answer stays with that prompt.
#[tokio::test]
async fn a_merged_notification_settles_its_held_delivery_by_the_answer_turn_receipt() {
    let team = child_with_a_call_running("background-held-answer-turn-receipt").await;
    let message_prompt = status_message(&team, false).await;
    wait_for_prompt_delivery(&team, &message_prompt).await;
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                background_notifications: vec![crate::provider::ProviderBackgroundNotification {
                    tool_use_id: Some("toolu_bg".into()),
                    task_id: Some("b1".into()),
                }],
                chunks: vec![
                    ended_at(crate::session::unix_epoch_ms()),
                    text_chunk("the one combined answer"),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    apply_own_turn(&team, ProviderOwnTurnSignal::None, true, Vec::new()).await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, false, false, true)
        .await
        .unwrap();
    team.runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms() + 2 * MINUTE_MS)
        .await
        .unwrap();
    let events = child_finished(&team);
    let held = events
        .iter()
        .filter(|event| event.payload["child_task_id"] == team.task.task_id)
        .collect::<Vec<_>>();
    assert_eq!(
        held.len(),
        1,
        "the original delivery must settle exactly once: {events:?}"
    );
    assert_eq!(
        held[0].payload["outcome"]["kind"], "answered_in_turn",
        "a merged delivery points to its answering turn, not an invented second answer: {events:?}"
    );
    let answer_task = team
        .runtime
        .owned
        .durable_state_store
        .agent_turn_task(&team.room, &team.child, &message_prompt)
        .unwrap()
        .unwrap();
    let target = &held[0].payload["outcome"]["answered_in_turn"];
    assert_eq!(target["task_id"], answer_task.task_id);
    assert_eq!(target["prompt_id"], message_prompt);
    assert_eq!(target["agent_id"], team.child);
    assert_eq!(target["session_id"], team.room);
    assert!(
        held[0].payload["public_answer"].is_null(),
        "the reference must not copy the combined answer"
    );
    let answers = events
        .iter()
        .filter(|event| event.payload["child_task_id"] == answer_task.task_id)
        .collect::<Vec<_>>();
    assert_eq!(answers.len(), 1, "one answer receipt: {events:?}");
    assert!(answers[0].payload["public_answer"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("the one combined answer"));
    sweep(&team).await;
    assert_eq!(
        child_finished(&team)
            .iter()
            .filter(|event| event.payload["child_task_id"] == team.task.task_id)
            .count(),
        1
    );
}

async fn restart_recovers_a_merged_notification_receipt(
    cancelled: bool,
    held_already_terminal: bool,
) {
    let mut team = child_with_a_call_running("background-held-answer-restart").await;
    let message_prompt = status_message(&team, false).await;
    wait_for_prompt_delivery(&team, &message_prompt).await;
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                background_notifications: vec![crate::provider::ProviderBackgroundNotification {
                    tool_use_id: Some("toolu_bg".into()),
                    task_id: Some("b1".into()),
                }],
                chunks: vec![
                    ended_at(crate::session::unix_epoch_ms()),
                    text_chunk("the one combined answer"),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
    let completion = if cancelled {
        team.runtime
            .owned
            .fail_local_prompt_without_advance_with_termination_if_matches(
                &team.room,
                &team.child,
                Some(&team.child_run),
                Some(&message_prompt),
                None,
                crate::provider::TurnOutcome::cancelled(),
            )
    } else {
        team.runtime.owned.complete_local_prompt_without_advance(
            &team.room,
            &team.child,
            Some(&team.child_run),
        )
    }
    .unwrap()
    .unwrap();
    let answer_prompt = completion.completion.completed;
    assert_eq!(answer_prompt.id(), message_prompt);
    let (answer, _) = team
        .runtime
        .owned
        .settle_agent_task(
            &team.room,
            &team.child,
            &answer_prompt,
            &team.child_run,
            cancelled,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        answer.state,
        if cancelled {
            ExecutionState::Cancelled
        } else {
            ExecutionState::Done
        }
    );
    // Crash after Q's terminal ledger write, before publishing it or releasing H.
    assert!(child_finished(&team).is_empty());
    assert!(working(&team));
    if held_already_terminal {
        let (held, _) = team
            .runtime
            .owned
            .settle_agent_task_answered_by(
                &team.room,
                &team.child,
                &team.child_prompt,
                &team.child_run,
                &team.child_run,
                cancelled,
            )
            .unwrap()
            .unwrap();
        assert_eq!(held.state, answer.state);
        assert!(
            team.runtime
                .owned
                .operational_history_store
                .load_prompt_settlement_event(&team.room, &team.child, team.child_prompt.id())
                .unwrap()
                .is_none_or(
                    |entry| entry.metadata[crate::history::TURN_OUTCOME_METADATA_KEY]
                        ["answered_in_turn"]
                        .is_null()
                ),
            "crash after the held task's terminal ledger write but before its history outcome"
        );
    }
    let state_path = team.runtime.owned.durable_state_store.path().to_path_buf();
    team.runtime.owned.durable_state_store =
        crate::durable_state::DurableKernelStateStore::open(state_path).unwrap();
    team.runtime.owned.background_holds = Default::default();
    sweep(&team).await;
    if held_already_terminal && cancelled {
        // The cancellation sweep publishes from its own task.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !child_finished(&team)
                .iter()
                .any(|event| event.payload["child_task_id"] == team.task.task_id)
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    let events = child_finished(&team);
    let held = events
        .iter()
        .filter(|event| event.payload["child_task_id"] == team.task.task_id)
        .collect::<Vec<_>>();
    assert_eq!(held.len(), 1, "one recovered delivery: {events:?}");
    assert_eq!(held[0].payload["success"], !cancelled, "{events:?}");
    let outcome = &held[0].payload["outcome"];
    assert_eq!(
        outcome["kind"],
        if cancelled {
            "cancelled"
        } else {
            "answered_in_turn"
        },
        "{events:?}"
    );
    assert_eq!(outcome["answered_in_turn"]["task_id"], answer.task_id);
    assert_eq!(outcome["answered_in_turn"]["prompt_id"], message_prompt);
    assert_eq!(outcome["answered_in_turn"]["session_id"], team.room);
    assert_eq!(outcome["answered_in_turn"]["agent_id"], team.child);
    assert!(held[0].payload["public_answer"].is_null());
    if !held_already_terminal {
        let history = team
            .runtime
            .owned
            .operational_history_store
            .load_prompt_settlement_event(&team.room, &team.child, team.child_prompt.id())
            .unwrap()
            .unwrap();
        assert_eq!(
            history.metadata[crate::history::TURN_OUTCOME_METADATA_KEY],
            *outcome
        );
    }
    sweep(&team).await;
    assert_eq!(
        child_finished(&team)
            .iter()
            .filter(|event| event.payload["child_task_id"] == team.task.task_id)
            .count(),
        1
    );
}

#[tokio::test]
async fn restart_keeps_the_completed_answer_turn_receipt_without_an_in_memory_hold() {
    restart_recovers_a_merged_notification_receipt(false, false).await;
}

#[tokio::test]
async fn restart_keeps_the_cancelled_answer_turn_receipt_without_reporting_success() {
    restart_recovers_a_merged_notification_receipt(true, false).await;
}

#[tokio::test]
async fn restart_recovers_the_receipt_after_the_held_task_completes() {
    restart_recovers_a_merged_notification_receipt(false, true).await;
}

#[tokio::test]
async fn restart_recovers_the_receipt_after_the_held_task_is_cancelled() {
    restart_recovers_a_merged_notification_receipt(true, true).await;
}
