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
            deadline: deadline_ms,
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
