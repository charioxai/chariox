//! MP-08 / MP-10 / MP-11: definitive native cancellation, never idle-screen inference.
use super::*;

#[test]
fn native_claude_cancel_after_hook_return_does_not_wait_for_first_assistant_block() {
    let mut f = CancellationFixture::new(800);
    f.app
        .active_turns
        .bind_native_prompt_hook_request(f.run.id(), f.prompt.id(), "thinking-hook");
    fs::write(
        f.root.0.join("events.jsonl"),
        serde_json::json!({
            "hook_event_name":"UserPromptSubmitReady", "hook_context_request_id":"thinking-hook"
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    // The fixture has no PTY; actual delivery failure may retire the turn.
    assert!(
        f.app
            .active_turns
            .get(f.run.id())
            .is_none_or(|turn| turn.interruption_retry_sent_at_ms.is_some()),
        "hook returned but no physical interrupt was attempted"
    );
}

#[test]
fn ignored_native_prompt_hooks_receive_an_explicit_empty_response() {
    for prompt in ["", "<task-notification>internal</task-notification>"] {
        let mut f = CancellationFixture::new(0);
        fs::write(f.root.0.join("events.jsonl"), serde_json::json!({
            "hook_event_name":"UserPromptSubmit", "hook_context_request_id":"ignored-hook", "prompt":prompt
        }).to_string()+"\n").unwrap();
        f.process();
        assert_eq!(
            fs::read_to_string(f.root.0.join("hook-context-responses/ignored-hook.txt")).unwrap(),
            ""
        );
    }
}

#[test]
fn stale_managed_prompt_hooks_receive_a_response_without_binding_a_turn() {
    let mut f = CancellationFixture::new(0);
    let context = f.root.0.join("context").display().to_string();
    interruption::bind_prompt_hook_request(
        &mut f.app,
        &f.session_id,
        f.run.id(),
        &context,
        "foreign-prompt",
        &serde_json::json!({"hook_context_request_id":"stale-hook"}),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(f.root.0.join("hook-context-responses/stale-hook.txt")).unwrap(),
        ""
    );
    assert!(f
        .app
        .active_turns
        .get(f.run.id())
        .unwrap()
        .native_prompt_hook_request_id
        .is_none());
    write_claude_native_marker(&context, "injected:foreign-prompt");
    fs::write(
        f.root.0.join("events.jsonl"),
        serde_json::json!({"hook_event_name":"UserPromptSubmit",
        "hook_context_request_id":"mismatched-hook", "prompt":"different text"})
        .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    assert_eq!(
        fs::read_to_string(f.root.0.join("hook-context-responses/mismatched-hook.txt")).unwrap(),
        ""
    );
}

struct FixtureRoot(PathBuf);
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_claude_tool_use_interruption_is_an_official_transcript_receipt() {
    let at = unix_epoch_ms();
    let raw = serde_json::json!({
        "type": "user", "uuid": "interruption",
        "timestamp": chrono::DateTime::from_timestamp_millis(at as i64).unwrap().to_rfc3339(),
        "message": { "role": "user", "content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}] }
    }).to_string() + "\n";
    let drain = drain_claude_transcript_raw_since(
        "session.jsonl",
        &raw,
        &mut transcript::ClaudeTranscriptCursor::default(),
        Some(at),
    );
    assert_eq!(drain.interrupted_at_ms, Some(at));
    assert!(
        drain.chunks.is_empty(),
        "provider control receipts are not user history"
    );
}

struct CancellationFixture {
    app: DaemonApp,
    session_id: String,
    agent_id: String,
    run: RuntimeProviderRun,
    prompt: crate::session::PromptQueueItem,
    root: FixtureRoot,
}

#[test]
fn official_native_turn_abort_receipt_settles_without_a_transcript_flush() {
    let mut f = CancellationFixture::new(0);
    f.app
        .active_turns
        .bind_native_prompt_hook_request(f.run.id(), f.prompt.id(), "bound-hook");
    fs::write(f.root.0.join("context.native-abort.json"), serde_json::json!({
        "turn_id":"official-turn", "prompt_id":f.prompt.id(), "hook_context_request_id":"bound-hook",
        "reason":"aborted", "at_ms":unix_epoch_ms()
    }).to_string()).unwrap();
    f.process();
    ProviderOutputClaudeNativeBridge::new(&mut f.app)
        .finish_deferred_stop(&f.session_id, f.run.id(), &f.run)
        .unwrap();
    assert!(f.active().is_none());
    assert_eq!(
        f.settled().settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Cancelled
    );
    assert!(!f.root.0.join("context.native-abort.json").exists());
}

#[test]
fn official_native_turn_abort_receipt_rejects_foreign_prompt_hook_and_future_time() {
    for mismatch in ["prompt", "hook", "future"] {
        let mut f = CancellationFixture::new(0);
        f.app
            .active_turns
            .bind_native_prompt_hook_request(f.run.id(), f.prompt.id(), "bound-hook");
        fs::write(f.root.0.join("context.native-abort.json"), serde_json::json!({
            "turn_id":"official-turn", "prompt_id":if mismatch=="prompt" {"foreign"} else {f.prompt.id()},
            "hook_context_request_id":if mismatch=="hook" {"foreign"} else {"bound-hook"},
            "reason":"aborted", "at_ms":unix_epoch_ms()+if mismatch=="future" {60_000} else {0}
        }).to_string()).unwrap();
        f.process();
        assert!(
            f.active().is_some(),
            "MP-11 foreign/future receipt must not settle the current turn"
        );
    }
}
impl CancellationFixture {
    fn new(age_ms: u64) -> Self {
        let worktree = crate::test_support::TestWorktree::new("native-cancellation-deadline");
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "native",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let root = FixtureRoot(std::env::temp_dir().join(format!(
            "integ2-cancel-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        )));
        fs::create_dir_all(&root.0).unwrap();
        let context = root.0.join("context");
        let events = root.0.join("events.jsonl");
        fs::write(&context, "").unwrap();
        fs::write(&events, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "haiku",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut run = RuntimeProviderRun::new(
            "native-deadline-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "native-deadline".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: std::collections::BTreeMap::from([
                    (
                        "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                        context.display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                        events.display().to_string(),
                    ),
                ]),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().into()))
            .unwrap();
        let prompt = match app
            .record_native_prompt_started_with_attachments(
                session.id(),
                attachment.id(),
                attachment.id(),
                agent.id(),
                "Read public documentation",
                vec![],
            )
            .unwrap()
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            other => panic!("{other:?}"),
        };
        app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
            .unwrap();
        app.active_turns.mark_interruption_requested(
            run.id(),
            prompt.id(),
            unix_epoch_ms().saturating_sub(age_ms),
        );
        write_claude_native_marker(
            &context.display().to_string(),
            &format!("native:{}", prompt.id()),
        );
        Self {
            app,
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            run,
            prompt,
            root,
        }
    }
    fn process(&mut self) {
        ProviderOutputClaudeNativeBridge::new(&mut self.app)
            .process(&self.session_id, self.run.id(), &self.run, None)
            .unwrap();
    }
    fn active(&mut self) -> Option<crate::session::PromptQueueItem> {
        self.app
            .prompt_owner_active_prompt_for_agent(&self.session_id, &self.agent_id)
            .unwrap()
    }
    fn settled(&self) -> crate::git_observer::CompletedGitTurnActionProjection {
        self.app
            .completed_git_turn_snapshot_store()
            .latest_projection_for_agent(&self.session_id, &self.agent_id)
            .unwrap()
    }
}

#[test]
fn native_claude_cancellation_has_a_definitive_hard_deadline() {
    let mut f = CancellationFixture::new(10_001);
    f.app.active_turns.note_native_interruption_delivery(
        f.run.id(),
        f.prompt.id(),
        unix_epoch_ms().saturating_sub(10_000),
    );
    f.process();
    assert!(
        f.active().is_none(),
        "native cancellation cannot remain Cancelling beyond its hard deadline"
    );
    let settled = f.settled();
    assert_eq!(settled.prompt_id, f.prompt.id());
    assert_eq!(
        settled.settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed
    );
    assert!(settled
        .provider_termination
        .unwrap()
        .reason
        .contains("stopped responding to cancel"));
    assert!(
        f.app
            .terminal()
            .output_records()
            .iter()
            .any(|record| record.kind == TerminalOutputKind::ProviderError
                && String::from_utf8_lossy(&record.bytes).contains("stopped responding to cancel")),
        "every pane receives the explicit reason through ordinary output"
    );
    assert_eq!(
        f.app.providers.get_run(f.run.id()).unwrap().state(),
        crate::provider::ProviderRunState::Ended
    );
}

#[test]
fn native_claude_undeliverable_cancel_has_an_accurate_failure_reason() {
    let mut f = CancellationFixture::new(10_001);
    f.process();
    assert!(f
        .settled()
        .provider_termination
        .unwrap()
        .reason
        .contains("could not be delivered"));
}

#[test]
fn resolved_prompt_hook_does_not_accumulate_cancellation_markers() {
    let mut f = CancellationFixture::new(0);
    f.app
        .active_turns
        .bind_native_prompt_hook_request(f.run.id(), f.prompt.id(), "resolved-hook");
    let responses = f.root.0.join("hook-context-responses");
    fs::create_dir_all(&responses).unwrap();
    fs::write(responses.join("resolved-hook.cancelled"), "").unwrap();
    let event = serde_json::json!({"hook_context_request_id":"resolved-hook"});
    interruption::observe_prompt_hook_ready(
        &mut f.app,
        f.run.id(),
        &f.root.0.join("context").display().to_string(),
        &event,
    );
    interruption::request_pending_prompt_hook_cancellation(
        &mut f.app,
        &f.session_id,
        f.run.id(),
        &f.root.0.join("context").display().to_string(),
    )
    .unwrap();
    assert!(!responses.join("resolved-hook.cancelled").exists());
}

#[test]
fn native_claude_cancellation_does_not_infer_idle_screen_acknowledgement() {
    let mut f = CancellationFixture::new(0);
    f.process();
    assert_eq!(
        f.active().unwrap().status(),
        crate::session::PromptStatus::Cancelling
    );
    assert_eq!(
        f.app.providers.get_run(f.run.id()).unwrap().state(),
        crate::provider::ProviderRunState::Running
    );
}

#[test]
fn native_claude_completed_transcript_wins_the_cancellation_race() {
    let mut f = CancellationFixture::new(0);
    let transcript = f.root.0.join("session.jsonl");
    fs::write(&transcript, serde_json::json!({"type":"assistant", "uuid":"final", "timestamp":chrono::Utc::now().to_rfc3339(), "message":{"id":"final-answer", "role":"assistant", "stop_reason":"end_turn", "content":[{"type":"text", "text":"Public documentation result"}]}}).to_string()+"\n").unwrap();
    fs::write(f.root.0.join("events.jsonl"), serde_json::json!({"hook_event_name":"SessionStart", "transcript_path":transcript.display().to_string()}).to_string()+"\n").unwrap();
    f.process();
    ProviderOutputClaudeNativeBridge::new(&mut f.app)
        .finish_deferred_stop(&f.session_id, f.run.id(), &f.run)
        .unwrap();
    assert!(f.active().is_none());
    assert_eq!(
        f.settled().settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Completed
    );
    assert!(f.app.terminal().output_records().iter().any(|record| record
        .bytes
        .windows(b"Public documentation result".len())
        .any(|text| text == b"Public documentation result")));
}

#[test]
fn native_claude_session_exit_without_cancel_receipt_is_error() {
    let mut f = CancellationFixture::new(0);
    fs::write(
        f.root.0.join("events.jsonl"),
        serde_json::json!({"hook_event_name":"SessionEnd", "native_prompt_id":f.prompt.id()})
            .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    assert!(f.active().is_none());
    assert_eq!(
        f.settled().settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed
    );
    assert!(f
        .settled()
        .provider_termination
        .unwrap()
        .reason
        .contains("exited before acknowledging cancellation"));
}

#[test]
fn native_claude_stale_stop_cannot_settle_a_successor() {
    let mut f = CancellationFixture::new(0);
    fs::write(
        f.root.0.join("events.jsonl"),
        serde_json::json!({"hook_event_name":"Stop", "native_prompt_id":"previous-prompt"})
            .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    ProviderOutputClaudeNativeBridge::new(&mut f.app)
        .finish_deferred_stop(&f.session_id, f.run.id(), &f.run)
        .unwrap();
    assert_eq!(f.active().unwrap().id(), f.prompt.id());
}

#[test]
fn native_claude_early_cancel_waits_for_an_official_response_before_pty_interrupt() {
    let mut f = CancellationFixture::new(2_501);
    f.process();
    assert!(
        f.app
            .active_turns
            .get(f.run.id())
            .unwrap()
            .interruption_retry_sent_at_ms
            .is_none(),
        "no early physical Esc before an official response"
    );
    assert!(
        f.active().is_some_and(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling),
        "before an official assistant response, keep the hook cancellation pending instead of sending an unacknowledgeable early Esc"
    );
    assert_eq!(
        f.app.providers.get_run(f.run.id()).unwrap().state(),
        crate::provider::ProviderRunState::Running
    );
}

#[test]
fn native_claude_response_readiness_requires_fresh_official_assistant_output() {
    for (fresh, timestamped, synthetic) in [
        (true, true, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let mut f = CancellationFixture::new(0);
        // Isolate transcript readiness from the separate physical-delivery test.
        f.app.active_turns.note_native_interruption_delivery(
            f.run.id(),
            f.prompt.id(),
            unix_epoch_ms(),
        );
        let transcript = f.root.0.join("response.jsonl");
        let mut response = serde_json::json!({"type":"assistant", "uuid":"response", "message":{"id":"response-message", "model":if synthetic { "<synthetic>" } else { "claude-haiku" }, "role":"assistant", "stop_reason":"tool_use", "content":[{"type":"thinking", "thinking":"Reviewing public documentation"}]}});
        if timestamped {
            let at = if fresh {
                f.prompt.created_at_ms()
            } else {
                f.prompt.created_at_ms().saturating_sub(1)
            };
            response["timestamp"] = chrono::DateTime::from_timestamp_millis(at as i64)
                .unwrap()
                .to_rfc3339()
                .into();
        }
        fs::write(&transcript, response.to_string() + "\n").unwrap();
        fs::write(f.root.0.join("events.jsonl"), serde_json::json!({"hook_event_name":"SessionStart", "transcript_path":transcript.display().to_string()}).to_string()+"\n").unwrap();
        f.process();
        let turn = f.app.active_turns.get(f.run.id()).unwrap();
        assert_eq!(
            turn.native_response_started,
            fresh && timestamped && !synthetic
        );
        assert!(
            !turn.native_interruption_confirmed,
            "response readiness never acknowledges cancellation"
        );
        assert_eq!(
            f.active().unwrap().status(),
            crate::session::PromptStatus::Cancelling
        );
    }
}

#[test]
fn native_claude_response_readiness_is_prompt_scoped() {
    let f = CancellationFixture::new(0);
    f.app
        .active_turns
        .note_native_response_started(f.run.id(), "foreign-prompt");
    assert!(
        !f.app
            .active_turns
            .get(f.run.id())
            .unwrap()
            .native_response_started
    );
    f.app
        .active_turns
        .note_native_response_started(f.run.id(), f.prompt.id());
    assert!(
        f.app
            .active_turns
            .get(f.run.id())
            .unwrap()
            .native_response_started
    );
}

#[test]
fn native_claude_abort_replay_cannot_extend_the_hard_deadline() {
    let mut f = CancellationFixture::new(10_001);
    f.app
        .active_turns
        .mark_interruption_requested(f.run.id(), f.prompt.id(), unix_epoch_ms());
    f.process();
    assert!(
        f.active().is_none(),
        "abort replay must preserve the first cancellation deadline"
    );
    assert_eq!(
        f.settled().settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed
    );
}

#[test]
fn native_claude_cancel_observes_a_transcript_created_after_its_hook() {
    let mut f = CancellationFixture::new(0);
    let transcript = f.root.0.join("late-session.jsonl");
    let context = f.root.0.join("context").display().to_string();
    fs::write(
        f.root.0.join("events.jsonl"),
        serde_json::json!({
            "hook_event_name":"SessionStart", "transcript_path":transcript.display().to_string()
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    assert!(
        known_claude_transcript_paths(&context).contains(&transcript.display().to_string()),
        "reported transcript must remain discoverable before its first flush"
    );
    fs::write(
        &transcript,
        serde_json::json!({
            "type":"user", "uuid":"late-interrupt", "timestamp":chrono::Utc::now().to_rfc3339(),
            "message":{"role":"user", "content":"[Request interrupted by user]"}
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    f.process();
    ProviderOutputClaudeNativeBridge::new(&mut f.app)
        .finish_deferred_stop(&f.session_id, f.run.id(), &f.run)
        .unwrap();
    assert!(f.active().is_none());
    assert_eq!(
        f.settled().settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Cancelled
    );
}
