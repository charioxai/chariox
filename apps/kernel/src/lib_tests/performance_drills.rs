use super::*;

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
fn performance_drill_large_history_projection_keeps_recent_suffix_hot() {
    let store = crate::runtime::projection::SessionHistoryProjectionStore::default();
    let entries = (0..2_500)
        .map(|index| {
            if index % 2 == 0 {
                crate::history::SessionHistoryEntry::user_prompt(
                    "session-history",
                    "attachment-1",
                    "agent-1",
                    format!("prompt {index}"),
                )
            } else {
                crate::history::SessionHistoryEntry::provider_output(
                    "session-history",
                    "provider-run-1",
                    Some("agent-1"),
                    TerminalOutputKind::ProviderOutput,
                    None,
                    format!("output {index}"),
                )
            }
        })
        .collect::<Vec<_>>();

    store.update_entries("session-history", entries);

    let recent_page = store
        .page("session-history", None, Some(3), None, None, None)
        .expect("recent unfiltered page should be served from the hot projection");
    assert!(!recent_page.entries.is_empty());
    assert!(
        recent_page.entries[0].entry_index >= 1_500,
        "projection should retain only the recent suffix with absolute entry indexes"
    );

    assert!(
        store
            .page("session-history", None, Some(1), None, Some(10), None)
            .is_none(),
        "old cursors outside the hot suffix should fall back to durable history"
    );
    assert!(
        store
            .page(
                "session-history",
                Some("agent-1"),
                Some(1),
                None,
                None,
                None
            )
            .is_none(),
        "agent-filtered reads on truncated projections should fall back to durable history"
    );
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
