use super::*;

const HISTORY_BASELINE_AGENT_COUNT: usize = 6;
const HISTORY_BASELINE_TURNS_PER_AGENT: usize = 24;
const HISTORY_BASELINE_TOOLS_PER_TURN: usize = 4;
const HISTORY_BASELINE_TOOL_BYTES: usize = 4_096;

#[tokio::test]
async fn performance_drill_session_history_outline() {
    let root = std::env::temp_dir().join(format!(
        "arroba-history-baseline-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("baseline root should be created");
    let history_path = root.join("history.db");
    let operational_history =
        crate::history::OperationalHistoryStore::open(history_path).expect("history should open");
    let session = crate::session::RuntimeSession::new(
        "session-history-baseline",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    seed_history_baseline(&operational_history, session.id());

    let agent_ids = (0..HISTORY_BASELINE_AGENT_COUNT)
        .map(|index| format!("agent-{index}"))
        .collect::<Vec<_>>();
    let total_events = HISTORY_BASELINE_AGENT_COUNT
        * HISTORY_BASELINE_TURNS_PER_AGENT
        * (2 + HISTORY_BASELINE_TOOLS_PER_TURN);
    let total_started = std::time::Instant::now();
    let response = crate::runtime::history_requests::execute_session_history_outline_request(
        operational_history.clone(),
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(agent_ids.clone()),
            latest_prompt_count: Some(4),
        },
    )
    .await
    .expect("outline should load");
    let response_bytes = serde_json::to_vec(&response)
        .expect("outline response should serialize")
        .len();
    let (returned_turns, returned_entries, returned_blobs, first_blob) = match &response {
        crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } => {
            let first_blob = agents.iter().find_map(|agent| {
                agent.turns.iter().find_map(|turn| {
                    turn.blobs
                        .first()
                        .map(|blob| (agent.agent_id.clone(), blob.blob_id.clone()))
                })
            });
            (
                agents.iter().map(|agent| agent.turns.len()).sum::<usize>(),
                agents
                    .iter()
                    .flat_map(|agent| agent.turns.iter())
                    .map(|turn| turn.entries.len() + usize::from(turn.summary.is_some()))
                    .sum::<usize>(),
                agents
                    .iter()
                    .flat_map(|agent| agent.turns.iter())
                    .map(|turn| turn.blobs.len())
                    .sum::<usize>(),
                first_blob,
            )
        }
        _ => (0, 0, 0, None),
    };
    let blob_started = std::time::Instant::now();
    let blob_response = first_blob
        .map(|(agent_id, blob_id)| async {
            crate::runtime::history_requests::execute_session_history_blob_content_request(
                operational_history.clone(),
                crate::local::GetSessionHistoryBlobContentRequest {
                    session_id: session.id().to_string(),
                    agent_id,
                    blob_id,
                },
            )
            .await
            .expect("blob content should load")
        })
        .expect("outline should include a blob")
        .await;
    let blob_expand_ms = blob_started.elapsed().as_secs_f64() * 1000.0;
    let blob_response_bytes = serde_json::to_vec(&blob_response)
        .expect("blob response should serialize")
        .len();
    let blob_entry_count = match &blob_response {
        crate::local::LocalDaemonResponse::SessionHistoryBlobContent { entries, .. } => {
            entries.len()
        }
        _ => 0,
    };

    let metrics = serde_json::json!({
        "metric": "session_history_outline",
        "agent_count": HISTORY_BASELINE_AGENT_COUNT,
        "turns_per_agent": HISTORY_BASELINE_TURNS_PER_AGENT,
        "tools_per_turn": HISTORY_BASELINE_TOOLS_PER_TURN,
        "tool_bytes": HISTORY_BASELINE_TOOL_BYTES,
        "total_seeded_events": total_events,
        "request_count": 1,
        "total_attach_history_ms": total_started.elapsed().as_secs_f64() * 1000.0,
        "outline_turns": returned_turns,
        "outline_entries": returned_entries,
        "outline_blobs": returned_blobs,
        "total_response_bytes": response_bytes,
        "blob_expand_ms": blob_expand_ms,
        "blob_response_bytes": blob_response_bytes,
        "blob_entry_count": blob_entry_count,
    });
    println!(
        "HISTORY_OUTLINE_METRICS {}",
        serde_json::to_string(&metrics).expect("metrics should serialize")
    );

    std::fs::remove_dir_all(root).expect("baseline root should be removed");
}

fn seed_history_baseline(
    operational_history: &crate::history::OperationalHistoryStore,
    session_id: &str,
) {
    for turn_index in 0..HISTORY_BASELINE_TURNS_PER_AGENT {
        for agent_index in 0..HISTORY_BASELINE_AGENT_COUNT {
            let agent_id = format!("agent-{agent_index}");
            let prompt_id = format!("{agent_id}-prompt-{turn_index}");
            let provider_run_id = format!("{agent_id}-run");
            let context = crate::history::HistoryEventTurnContext {
                session_id: Some(session_id.to_string()),
                agent_id: Some(agent_id.clone()),
                turn_id: Some(prompt_id.clone()),
                prompt_id: Some(prompt_id.clone()),
                provider_run_id: Some(provider_run_id.clone()),
                ..crate::history::HistoryEventTurnContext::default()
            };
            operational_history
                .append_transcript(
                    &crate::history::SessionHistoryEntry::user_prompt(
                        session_id,
                        "attachment-1",
                        &agent_id,
                        format!("Prompt {turn_index} for {agent_id}: summarize the latest work."),
                    ),
                    context.clone(),
                )
                .expect("user prompt should append");
            for tool_index in 0..HISTORY_BASELINE_TOOLS_PER_TURN {
                let tool_payload = serde_json::json!({
                    "id": format!("{prompt_id}-tool-{tool_index}"),
                    "tool": "bash",
                    "status": "completed",
                    "input": { "command": format!("generate-output --agent {agent_id} --turn {turn_index} --tool {tool_index}") },
                    "output": "x".repeat(HISTORY_BASELINE_TOOL_BYTES),
                });
                operational_history
                    .append_transcript(
                        &crate::history::SessionHistoryEntry::provider_output(
                            session_id,
                            &provider_run_id,
                            Some(&agent_id),
                            TerminalOutputKind::ProviderTool,
                            Some(format!("{prompt_id}-tool-{tool_index}")),
                            tool_payload.to_string(),
                        ),
                        context.clone(),
                    )
                    .expect("tool output should append");
            }
            operational_history
                .append_transcript(
                    &crate::history::SessionHistoryEntry::provider_output(
                        session_id,
                        &provider_run_id,
                        Some(&agent_id),
                        TerminalOutputKind::ProviderOutput,
                        None,
                        format!("Completed turn {turn_index} for {agent_id}."),
                    ),
                    context,
                )
                .expect("assistant output should append");
        }
    }
}

#[test]
fn performance_drill_noisy_prompt_queue_is_bounded_per_agent() {
    let owner = crate::runtime::prompt_state::PromptStateOwner::default();
    let session = crate::session::RuntimeSession::new(
        "session-perf-queue",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );

    for index in 0..crate::runtime::prompt_state::PROMPT_QUEUE_LIMIT {
        let outcome = owner
            .submit_prepared_prompt(
                &session,
                crate::session::PromptQueueItem::new(
                    format!("prompt-{index}"),
                    "attachment-1",
                    "agent-1",
                    format!("queued {index}"),
                    PromptStatus::Queued,
                ),
                true,
            )
            .expect("prompt should fit while under queue limit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));
    }

    let error = owner
        .submit_prepared_prompt(
            &session,
            crate::session::PromptQueueItem::new(
                "prompt-overflow",
                "attachment-1",
                "agent-1",
                "overflow",
                PromptStatus::Queued,
            ),
            true,
        )
        .expect_err("one busy agent must not accept unbounded queued prompts");

    assert!(error.to_string().contains("agent prompt queue overloaded"));
    assert_eq!(
        owner.queued_prompt_count_for_agent(&session, "agent-1"),
        crate::runtime::prompt_state::PROMPT_QUEUE_LIMIT
    );
}

#[test]
fn performance_drill_high_output_terminal_stream_coalesces_records() {
    let mut terminal = crate::terminal::TerminalStreamService::new();
    let chunk = vec![b'x'; 32];

    for _ in 0..1_000 {
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            &chunk,
        );
    }

    let records = terminal.output_records();
    let total_bytes = records
        .iter()
        .map(|record| record.bytes.len())
        .sum::<usize>();
    assert_eq!(total_bytes, 32_000);
    assert!(
        records.len() <= 2,
        "adjacent provider output should coalesce instead of producing 1000 records"
    );
    assert_eq!(terminal.health_store().snapshot().pending_output_records, 2);
}

#[test]
fn performance_drill_many_agent_session_keeps_runs_isolated() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut agent_ids = vec![default_agent.id().to_string()];

    for index in 0..31 {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias(format!("perf-{index}"))
                    .with_worktree("worktree-1"),
            )
            .expect("agent should spawn");
        agent_ids.push(agent.id().to_string());
    }

    for (index, agent_id) in agent_ids.iter().enumerate() {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                format!("sonnet-{index}"),
            )
            .with_agent_id(agent_id),
        )
        .expect("provider run should launch");
    }

    let session_agents = app.agents().get_session_agents(session.id());
    assert_eq!(session_agents.len(), agent_ids.len());
    assert_eq!(app.providers().list_runs().len(), agent_ids.len());
    for agent_id in agent_ids {
        assert!(
            app.providers()
                .get_latest_run_for_agent(session.id(), &agent_id)
                .is_some(),
            "each busy agent should retain an isolated provider run"
        );
    }
}

#[test]
fn performance_drill_provider_park_resume_reuses_existing_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");

    for _ in 0..50 {
        app.providers()
            .park_run_provider_only(session.id(), run.id())
            .expect("run should park");
        app.providers()
            .resume_run_provider_only(session.id(), run.id())
            .expect("run should resume");
    }

    assert_eq!(app.providers().list_runs().len(), 1);
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        crate::provider::ProviderRunState::Running
    );
}
