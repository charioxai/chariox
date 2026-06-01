use super::*;

#[test]
fn launching_provider_via_app_marks_session_active() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");

    assert_eq!(session.active_provider_run_id(), Some(run.id()));
}

#[test]
fn detaching_last_attachment_parks_and_reattaching_resumes_same_provider_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let _detached = crate::app::KernelSessionService::new(&mut app)
        .detach(attachment.id())
        .expect("last attachment should detach cleanly");

    let parked_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after detach");
    let parked_run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should still exist after detach");

    assert_eq!(parked_session.active_provider_run_id(), None);
    assert_eq!(
        parked_run.state(),
        crate::provider::ProviderRunState::Parked
    );

    let reattached = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("reattach should resume the parked provider run");

    let resumed_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after reattach");
    let resumed_run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should still exist after reattach");

    assert_eq!(reattached.session_id(), session.id());
    assert_eq!(resumed_session.active_provider_run_id(), Some(run.id()));
    assert_eq!(
        resumed_run.state(),
        crate::provider::ProviderRunState::Running
    );
}

#[test]
fn multi_agent_reattach_resumes_focused_run_before_focus_cycle() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let extra_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("extra")
                .with_worktree("worktree-1"),
        )
        .expect("extra agent should be created");

    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet-default",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");

    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), extra_agent.id())
        .expect("extra agent should focus");
    let extra_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet-extra",
            )
            .with_agent_id(extra_agent.id()),
        )
        .expect("extra provider run should launch");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), default_agent.id())
        .expect("default agent should refocus");

    crate::app::KernelSessionService::new(&mut app)
        .detach(attachment.id())
        .expect("last attachment should detach cleanly");
    assert_eq!(
        app.providers()
            .get_run(default_run.id())
            .expect("default run should remain")
            .state(),
        crate::provider::ProviderRunState::Parked
    );

    crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("reattach should resume the focused provider run");
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain")
            .active_provider_run_id(),
        Some(default_run.id())
    );
    assert_eq!(
        app.providers()
            .get_run(default_run.id())
            .expect("default run should remain")
            .state(),
        crate::provider::ProviderRunState::Running
    );

    let cycled = crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), extra_agent.id())
        .expect("focusing another agent after reattach should not park an already parked run");
    assert_eq!(cycled.id(), extra_agent.id());
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain")
            .active_provider_run_id(),
        Some(extra_run.id())
    );
    assert_eq!(
        app.providers()
            .get_run(extra_run.id())
            .expect("extra run should remain")
            .state(),
        crate::provider::ProviderRunState::Running
    );
}

#[test]
fn launching_a_provider_run_persists_resume_state_back_to_the_agent() {
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
            .with_agent_id(agent.id())
            .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1")),
        )
        .expect("provider run should launch");

    let stored_agent = app
        .agents()
        .get_agent(agent.id())
        .expect("agent should still exist");

    assert_eq!(run.resume_state().codex_thread_id(), Some("thread-1"));
    assert_eq!(
        stored_agent.provider_resume_state().codex_thread_id(),
        Some("thread-1")
    );
}

#[test]
fn prompt_submission_queues_and_notifies_other_attachments() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let first = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("first attachment should attach");
    let second = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("second attachment should attach");

    let _run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let first_outcome = app
        .submit_prompt(session.id(), first.id(), None, "first prompt\n", Vec::new())
        .expect("first prompt should start");
    let second_outcome = app
        .submit_prompt(
            session.id(),
            second.id(),
            None,
            "second prompt\n",
            Vec::new(),
        )
        .expect("second prompt should queue");

    match first_outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        _ => panic!("expected first prompt to start"),
    }
    match second_outcome {
        PromptSubmissionOutcome::Queued { .. } => {}
        _ => panic!("expected second prompt to queue"),
    }

    assert_eq!(app.terminal().notice_records().len(), 1);
    assert!(app.terminal().notice_records()[0]
        .recipient_attachment_ids
        .contains(&first.id().to_string()));
}

#[test]
fn spawning_a_seventh_agent_in_one_session_succeeds() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    for index in 0..6 {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "opencode")
                    .with_alias(format!("agent-{index}"))
                    .with_worktree("worktree-1"),
            )
            .expect("agent spawn should succeed");
        assert_eq!(agent.session_id(), session.id());
    }

    assert_eq!(app.agents().get_session_agents(session.id()).len(), 7);
}

#[test]
fn ended_sessions_reopen_on_attach_and_preserve_history() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let _run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let _ = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            None,
            "restore me\n",
            Vec::new(),
        )
        .expect("prompt should submit");
    let _ = crate::app::KernelSessionService::new(&mut app)
        .end_session(session.id())
        .expect("session should end");

    let reopened = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("ended session should reopen on attach");
    let reopened_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist after reopen");
    let history = crate::app::KernelSessionReadService::new(&app)
        .session_history(session.id())
        .expect("history should still load");
    let operational_history = app
        .operational_history_store()
        .load_session_events(session.id(), None)
        .expect("operational history should load");

    assert_eq!(reopened.session_id(), session.id());
    assert_eq!(reopened_session.status(), SessionStatus::Parked);
    assert_eq!(reopened_session.attachment_ids().len(), 1);
    assert!(history
        .iter()
        .any(|entry| entry.text.contains("restore me")));
    assert!(operational_history.iter().any(|entry| entry
        .content
        .as_deref()
        .is_some_and(|text| text.contains("restore me"))));
}

#[test]
fn session_history_entries_read_operational_history() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "attachment-1",
        agent.id(),
        "from operational history",
    );
    app.operational_history_store()
        .append_transcript(&entry, crate::history::HistoryEventTurnContext::default())
        .expect("operational event should append");

    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history entries should load");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "from operational history");
}

#[test]
fn deleted_sessions_cannot_be_reattached() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"))
        .expect("session should be created");

    let resolved = app
        .sessions()
        .resolve_session_ref("main", Some("workspace-1"))
        .expect("session ref should resolve");
    let deleted = app
        .session_state_store()
        .delete_session(resolved.id())
        .expect("session should delete");

    assert_eq!(deleted.id(), session.id());
    assert!(matches!(
        crate::app::KernelSessionService::new(&mut app).attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        )),
        Err(crate::error::DaemonError::SessionNotFound { .. })
    ));
}

#[test]
fn terminal_flow_writes_input_resizes_and_fans_out_output() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("source attachment should attach");
    let observer = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("observer should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    crate::app::KernelSessionService::new(&mut app)
        .resize_terminal(session.id(), 90, 24)
        .expect("terminal resize should succeed");
    crate::app::terminal_input::ProviderTerminalInput::new(&mut app)
        .send_provider_input(session.id(), run.id(), source.id(), b"fanout test\n")
        .expect("attachment input should reach provider PTY");

    let records = wait_for_terminal_output(&mut app, session.id(), source.id());

    assert!(!records.is_empty());
    assert_eq!(app.terminal().input_records().len(), 1);
    assert!(records
        .iter()
        .all(|record| record.provider_run_id == run.id()));
    assert!(records.iter().all(|record| {
        record
            .recipient_attachment_ids
            .contains(&source.id().to_string())
            && record
                .recipient_attachment_ids
                .contains(&observer.id().to_string())
    }));
    let combined = records
        .into_iter()
        .flat_map(|record| record.bytes)
        .collect::<Vec<u8>>();
    let combined = String::from_utf8_lossy(&combined);
    assert!(combined.contains("fanout test"));
}

#[test]
fn config_updates_are_versioned_and_notified() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let first = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let second = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");

    let (_session, config) = crate::session::SessionStateOwner::new(app.session_state_store())
        .update_config(
            session.id(),
            first.id(),
            BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            false,
        )
        .expect("config update should succeed");
    app.record_notice(
        session.id(),
        None,
        vec![second.id().to_string()],
        format!(
            "Attachment `{}` updated configuration for session `{}`.",
            first.id(),
            session.id()
        ),
    );

    assert_eq!(config.version(), 1);
    assert_eq!(
        config.values().get("theme").map(String::as_str),
        Some("compact")
    );
    assert_eq!(app.terminal().notice_records().len(), 1);
    assert!(app.terminal().notice_records()[0]
        .recipient_attachment_ids
        .contains(&second.id().to_string()));
}

#[test]
fn failed_provider_switch_resumes_previous_run_and_records_notice() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let _attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let first_run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("first provider run should launch");

    let error = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-invalid-pty",
            "claude-code",
            "default",
            "opus",
        ))
        .expect_err("invalid PTY adapter should fail during launch");

    match error {
        DaemonError::PtySpawn { .. } => {}
        other => panic!("unexpected error: {other}"),
    }

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    let resumed_run = app
        .providers()
        .get_run(first_run.id())
        .expect("original run should still exist");

    assert_eq!(session.active_provider_run_id(), Some(first_run.id()));
    assert_eq!(
        resumed_run.state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(app.terminal().notice_records().len(), 1);
    assert!(app.terminal().notice_records()[0]
        .message
        .contains("resumed the previous provider run"));
}
