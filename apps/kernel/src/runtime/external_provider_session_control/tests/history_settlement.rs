use super::*;

#[test]
fn append_observed_external_codex_assistant_waits_for_task_complete_to_settle() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "intermediate commentary".to_string(),
        observed_at_ms: Some(84),
    };
    let task_complete = ObservedExternalProviderTurn {
        provider_turn_id: Some("task-complete-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
        observed_at_ms: Some(126),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), assistant.clone()],
        },
    )
    .expect("observed Codex assistant turn should append");

    let stable_assistant_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), assistant],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable Codex assistant turn should stay active");

    assert_eq!(stable_assistant_outcome.changed_count, 0);
    assert!(!stable_assistant_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "Codex assistant commentary can be followed by tools and must not settle the turn"
    );

    let complete_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, task_complete],
        },
    )
    .expect("Codex task_complete should settle");

    assert_eq!(complete_outcome.changed_count, 1);
    assert!(complete_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "Codex task_complete should clear the external active prompt"
    );
}

#[test]
fn append_observed_external_codex_turn_aborted_settles_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("observed prompt should append");
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "observed prompt should create an external active prompt"
    );

    let abort = ObservedExternalProviderTurn {
        provider_turn_id: Some("turn-aborted-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex event turn_aborted { \"type\": \"turn_aborted\" }".to_string(),
        observed_at_ms: Some(84),
    };
    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, abort],
        },
    )
    .expect("Codex turn_aborted should append and settle");

    assert_eq!(outcome.changed_count, 1);
    assert!(outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "Codex turn_aborted should clear the external active prompt"
    );
}

#[test]
fn append_observed_external_provider_abort_completion_statuses_settle_active_prompt() {
    for (provider, status_text) in [
        (
            "claude",
            "claude message completed\n{\"stop_reason\":\"interrupted\"}",
        ),
        (
            "opencode",
            "opencode message completed\n{\"finish\":\"cancelled\"}",
        ),
    ] {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            format!("{provider}:thread-observed"),
            provider.to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
        let prompt = ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed prompt should append");
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "{provider} prompt should mark the external turn running"
        );

        let abort_status = ObservedExternalProviderTurn {
            provider_turn_id: Some("abort-status-1".to_string()),
            role: ObservedExternalProviderTurnRole::Status,
            text: status_text.to_string(),
            observed_at_ms: Some(84),
        };
        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, abort_status],
            },
        )
        .expect("observed abort-like completion status should append and settle");

        assert!(
            outcome.external_active_prompt_settled,
            "{provider} abort-like completion status should settle"
        );
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "{provider} abort-like completion status should clear the external active prompt"
        );
        let entries = app
            .history_store()
            .load(&session)
            .expect("history should load");
        let status_entry = entries
            .iter()
            .find(|entry| entry.kind == SessionHistoryEntryKind::ProviderStatus)
            .expect("status history entry should persist");
        assert_eq!(
            status_entry
                .external_observation
                .as_ref()
                .map(|observation| observation.settles_active_prompt),
            Some(true),
            "{provider} settling status should persist structured observation metadata"
        );
    }
}

#[test]
fn append_observed_external_assistant_turn_waits_for_settlement_permission() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "external reply".to_string(),
        observed_at_ms: Some(84),
    };

    let first_assistant_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("user-1".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "external prompt".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed user turn should append");
    assert_eq!(first_assistant_outcome.changed_count, 1);

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![assistant.clone()],
        },
    )
    .expect("observed assistant turn should append");

    let early_stable_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![assistant.clone()],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: false,
        },
    )
    .expect("early stable assistant turn should not settle");

    assert_eq!(early_stable_outcome.changed_count, 0);
    assert!(!early_stable_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "stable assistant output should stay running until settlement is permitted"
    );

    let late_stable_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![assistant],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("late stable assistant turn should settle");

    assert_eq!(late_stable_outcome.changed_count, 0);
    assert!(late_stable_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "external active prompt should clear once settlement is permitted"
    );
}

#[test]
fn append_observed_external_claude_telemetry_after_assistant_still_settles() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "final external reply".to_string(),
        observed_at_ms: Some(84),
    };
    let telemetry = ObservedExternalProviderTurn {
        provider_turn_id: Some("last-prompt-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
        observed_at_ms: Some(126),
    };

    let initial_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), assistant.clone(), telemetry.clone()],
        },
    )
    .expect("observed Claude turn should append");
    assert_eq!(initial_outcome.changed_count, 3);
    assert_eq!(initial_outcome.active_relevant_changed_count, 2);

    let outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, assistant, telemetry],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable Claude telemetry should settle");

    assert_eq!(outcome.changed_count, 0);
    assert_eq!(outcome.active_relevant_changed_count, 0);
    assert!(outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "Claude passive telemetry after assistant output must not keep the external turn working"
    );
    let events = app
        .operational_history_store()
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().to_string()),
            agent_id: Some(agent.id().to_string()),
            limit: Some(20),
            ..crate::history::HistoryEventQuery::default()
        })
        .expect("history events should load");
    let settlement = events
        .iter()
        .filter_map(|event| event.to_session_history_entry())
        .find(|entry| {
            entry
                .merge_key
                .as_deref()
                .is_some_and(|merge_key| merge_key.contains(":state:active_prompt_settled:"))
        })
        .expect("implicit Claude settlement must be durable history");
    assert_eq!(settlement.text, "");
    assert_eq!(
        settlement.external_provider_turn_id.as_deref(),
        Some("user-1"),
        "durable settlement must group with the external prompt turn"
    );
    assert_eq!(settlement.observed_at_ms, Some(126));
    assert_eq!(
        settlement
            .external_observation
            .as_ref()
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );
}

#[test]
fn append_observed_external_claude_new_passive_telemetry_after_assistant_settles_immediately() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "final external reply".to_string(),
        observed_at_ms: Some(84),
    };
    let initial_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), assistant.clone()],
        },
    )
    .expect("observed Claude assistant turn should append");
    assert_eq!(initial_outcome.changed_count, 2);
    assert_eq!(initial_outcome.active_relevant_changed_count, 2);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "new assistant output should keep the external turn running through the grace window"
    );

    let telemetry = ObservedExternalProviderTurn {
        provider_turn_id: Some("last-prompt-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
        observed_at_ms: Some(126),
    };
    let telemetry_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, assistant, telemetry],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("new passive telemetry should append and settle the stable assistant turn");

    assert_eq!(telemetry_outcome.changed_count, 1);
    assert_eq!(telemetry_outcome.active_relevant_changed_count, 0);
    assert!(telemetry_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "new Claude passive telemetry after a stable assistant message should settle immediately"
    );
}

#[test]
fn append_observed_external_claude_completion_after_tool_settles() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let tool = ObservedExternalProviderTurn {
        provider_turn_id: Some("tool-1".to_string()),
        role: ObservedExternalProviderTurnRole::Tool,
        text: "TOOL_STEP_20: complete".to_string(),
        observed_at_ms: Some(84),
    };
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "FINAL_EXTERNAL_PARITY_SUMMARY".to_string(),
        observed_at_ms: Some(126),
    };
    let completion = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1:completed".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude message completed\n{\"stop_reason\":\"end_turn\"}".to_string(),
        observed_at_ms: Some(126),
    };
    let telemetry = ObservedExternalProviderTurn {
        provider_turn_id: Some("last-prompt-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
        observed_at_ms: Some(168),
    };

    let initial_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![
                prompt.clone(),
                tool.clone(),
                assistant.clone(),
                completion.clone(),
                telemetry.clone(),
            ],
        },
    )
    .expect("observed Claude turn should append");
    assert_eq!(initial_outcome.changed_count, 5);
    assert_eq!(initial_outcome.active_relevant_changed_count, 4);
    assert!(!initial_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "completed Claude imports should not create a running external prompt"
    );

    let outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, tool, assistant, completion, telemetry],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable Claude completion should settle");

    assert_eq!(outcome.changed_count, 0);
    assert_eq!(outcome.active_relevant_changed_count, 0);
    assert!(!outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "completed Claude imports should remain idle on stable reread"
    );
}

#[test]
fn append_observed_external_claude_completion_with_new_passive_telemetry_settles() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let tool = ObservedExternalProviderTurn {
        provider_turn_id: Some("tool-1".to_string()),
        role: ObservedExternalProviderTurnRole::Tool,
        text: "TOOL_STEP_20: complete".to_string(),
        observed_at_ms: Some(84),
    };
    let running_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), tool.clone()],
        },
    )
    .expect("observed Claude tool should append");
    assert_eq!(running_outcome.changed_count, 2);
    assert!(!running_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "external tool output should keep the prompt running"
    );

    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "FINAL_EXTERNAL_PARITY_SUMMARY".to_string(),
        observed_at_ms: Some(126),
    };
    let completion = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1:completed".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude message completed\n{\"stop_reason\":\"end_turn\"}".to_string(),
        observed_at_ms: Some(126),
    };
    let telemetry = ObservedExternalProviderTurn {
        provider_turn_id: Some("last-prompt-leaf-assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "claude last-prompt {\"leafUuid\":\"assistant-1\"}".to_string(),
        observed_at_ms: None,
    };

    let completed_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, tool, assistant, completion, telemetry],
        },
    )
    .expect("observed Claude completion should append and settle");

    assert_eq!(completed_outcome.changed_count, 3);
    assert_eq!(completed_outcome.active_relevant_changed_count, 2);
    assert!(completed_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "Claude completion followed by passive telemetry must clear WORKING in the same poll"
    );
}

#[test]
fn append_observed_external_tool_turn_keeps_external_active_prompt_running() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let tool = ObservedExternalProviderTurn {
        provider_turn_id: Some("tool-1".to_string()),
        role: ObservedExternalProviderTurnRole::Tool,
        text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), tool.clone()],
        },
    )
    .expect("observed prompt and tool should append");

    let stable_tool_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, tool],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable tool should not settle");

    assert_eq!(stable_tool_outcome.changed_count, 0);
    assert!(!stable_tool_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "tool output alone should not settle the external turn"
    );
}

#[test]
fn append_observed_external_codex_token_count_keeps_active_prompt_running() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let run = test_codex_run(session.id(), agent.id(), "run-imported", "thread-observed");
    app.providers_mut().insert_run_for_test(run.clone());
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        Some(run.id().to_string()),
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let token_count = ObservedExternalProviderTurn {
        provider_turn_id: Some("token-count-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
            .to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("observed prompt should append");

    let token_count_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, token_count],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("Codex token count should append without settling");

    assert_eq!(token_count_outcome.changed_count, 1);
    assert!(!token_count_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "Codex token_count is telemetry and must not settle the external turn"
    );
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("provider run should load")
            .usage(),
        crate::provider::ProviderRunTokenUsage {
            total_tokens: Some(42),
            last_tokens: Some(42),
            context_tokens: None,
            context_window: None,
        },
        "externally observed Codex token telemetry should update kernel provider-run usage"
    );
    assert_eq!(
        app.provider_run_projection_store()
            .get(run.id())
            .expect("projected provider run should load")
            .usage(),
        crate::provider::ProviderRunTokenUsage {
            total_tokens: Some(42),
            last_tokens: Some(42),
            context_tokens: None,
            context_window: None,
        },
        "externally observed Codex token telemetry should update client-visible provider-run projection"
    );
}

#[test]
fn append_observed_external_opencode_completion_status_settles_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "opencode:thread-observed".to_string(),
        "opencode".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let status = ObservedExternalProviderTurn {
        provider_turn_id: Some("message-status-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "opencode message completed\n{\"finish\":\"stop\"}".to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("observed prompt should append");
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "OpenCode prompt should mark the external turn running"
    );

    let first_status_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), status.clone()],
        },
    )
    .expect("observed prompt and status should append");
    assert!(first_status_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "OpenCode completion metadata should settle the external turn immediately"
    );

    let stable_status_outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, status],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable completion status should stay settled");

    assert_eq!(stable_status_outcome.changed_count, 0);
    assert!(!stable_status_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "OpenCode completion metadata should keep the external turn settled"
    );
}
