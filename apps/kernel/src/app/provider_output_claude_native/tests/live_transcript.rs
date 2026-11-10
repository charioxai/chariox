//! MP-08 / MP-09 / MP-10 / MP-11: the headless bridge reads Claude's transcript
//! while a turn runs, so tool rows reach clients as the calls happen and not
//! only when Claude stops.
use super::*;

struct RunningTurn {
    app: DaemonApp,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    run: RuntimeProviderRun,
    root: std::path::PathBuf,
    events_file: std::path::PathBuf,
    transcript: std::path::PathBuf,
    _worktree: crate::test_support::TestWorktree,
}

fn running_turn(name: &str) -> RunningTurn {
    let run_id = format!("provider-run-live-{name}");
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-live-transcript",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-live-transcript-{name}-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "claude-haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        &run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-live-transcript".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "Run three commands.",
            Vec::new(),
        )
        .expect("active prompt should start")
    {
        crate::session::PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt outcome: {other:?}"),
    }
    RunningTurn {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        attachment_id: attachment.id().to_string(),
        transcript: root.join("session.jsonl"),
        app,
        run,
        root,
        events_file,
        _worktree: worktree,
    }
}

impl RunningTurn {
    fn pass(&mut self) {
        ProviderOutputClaudeNativeBridge::new(&mut self.app)
            .process(&self.session_id, self.run.id(), &self.run, None)
            .expect("the bridge pass should succeed");
    }

    fn hook_event(&self, name: &str) {
        fs::write(
            &self.events_file,
            serde_json::json!({
                "hook_event_name": name,
                "transcript_path": self.transcript.display().to_string(),
            })
            .to_string()
                + "\n",
        )
        .expect("hook event should be written");
    }

    /// Claude appends one transcript line, stamped just after the prompt began.
    fn append_transcript(&self, mut line: serde_json::Value) {
        use std::io::Write;
        line["timestamp"] = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::seconds(1))
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .into();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.transcript)
            .expect("transcript should open");
        writeln!(file, "{line}").expect("transcript line should write");
    }

    fn output_rows(&self) -> Vec<String> {
        self.app
            .terminal()
            .output_records()
            .into_iter()
            .filter(|record| record.kind == TerminalOutputKind::ProviderOutput)
            .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
            .collect()
    }

    fn start_prompt(&mut self, text: &str) {
        match self
            .app
            .record_native_prompt_started_with_attachments(
                &self.session_id,
                &self.attachment_id,
                &self.attachment_id,
                &self.agent_id,
                text,
                Vec::new(),
            )
            .expect("prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { .. } => {}
            other => panic!("unexpected prompt outcome: {other:?}"),
        }
    }

    fn active_prompt_id(&mut self) -> String {
        self.app
            .prompt_owner_active_prompt_for_agent(&self.session_id, &self.agent_id)
            .expect("prompt state should read")
            .expect("a prompt should be active")
            .id()
            .to_string()
    }

    fn tool_rows(&self) -> Vec<String> {
        self.app
            .terminal()
            .output_records()
            .into_iter()
            .filter(|record| record.kind == TerminalOutputKind::ProviderTool)
            .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
            .collect()
    }
}

impl Drop for RunningTurn {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn tool_call(id: &str, command: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "assistant",
        "uuid": format!("assistant-{id}"),
        "sessionId": "claude-session-live",
        "message": {
            "id": format!("msg-{id}"),
            "role": "assistant",
            "model": "claude-haiku-5-5",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "id": id,
                "name": "Bash",
                "input": { "command": command },
            }],
        },
    })
}

fn tool_result(id: &str, output: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "user",
        "uuid": format!("result-{id}"),
        "sessionId": "claude-session-live",
        "message": {
            "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": id, "content": output }],
        },
    })
}

#[test]
fn a_tool_row_appears_while_the_turn_runs_though_claude_has_not_stopped() {
    let mut turn = running_turn("tool-row");

    // Claude reports its transcript before it has written the first line.
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    assert!(turn.tool_rows().is_empty());

    turn.append_transcript(tool_call("toolu_live_1", "sleep 8; echo one"));
    turn.pass();
    let rows = turn.tool_rows();
    assert_eq!(rows.len(), 1, "the call shows as soon as Claude writes it");
    assert!(rows[0].contains("toolu_live_1"), "{rows:?}");

    turn.append_transcript(tool_result("toolu_live_1", "one"));
    turn.pass();
    assert!(
        turn.tool_rows()
            .iter()
            .any(|row| row.contains("\"output\":\"one\"")),
        "the result shows as soon as Claude writes it: {:?}",
        turn.tool_rows()
    );

    // A pass over a transcript that did not change adds nothing.
    let rows_before = turn.tool_rows().len();
    turn.pass();
    turn.pass();
    assert_eq!(turn.tool_rows().len(), rows_before);
}

fn assistant_text(id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "assistant",
        "uuid": format!("assistant-{id}"),
        "sessionId": "claude-session-live",
        "message": {
            "id": format!("msg-{id}"),
            "role": "assistant",
            "model": "claude-haiku-5-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": text }],
        },
    })
}

#[test]
fn nothing_a_stopped_turn_writes_after_the_stop_lands_in_it() {
    let mut turn = running_turn("stopped-tail");
    turn.hook_event("UserPromptSubmit");
    turn.append_transcript(tool_call("toolu_before_stop", "sleep 20"));
    turn.pass();
    assert_eq!(turn.tool_rows().len(), 1);

    // The person presses Stop: the kernel fences the turn and settles it.
    let stopped = turn.active_prompt_id();
    crate::app::fence_cancelled_claude_turn(&turn.run, &stopped);
    turn.app
        .prompt_owner_cancel_active_prompt_only(&turn.session_id, &turn.agent_id)
        .expect("the turn should cancel");

    // Claude had not noticed: it asks for the tool again and answers.
    turn.append_transcript(tool_result("toolu_before_stop", "[Request interrupted]"));
    turn.append_transcript(tool_call("toolu_retry", "sleep 20"));
    turn.append_transcript(assistant_text("answer", "I retried the command."));
    turn.pass();
    assert!(
        turn.tool_rows()
            .iter()
            .all(|row| !row.contains("toolu_retry")),
        "the retry landed in the stopped turn: {:?}",
        turn.tool_rows()
    );
    assert!(
        turn.output_rows().is_empty(),
        "an answer landed in the stopped turn: {:?}",
        turn.output_rows()
    );

    // A prompt started after the Stop owns what Claude writes next.
    std::thread::sleep(std::time::Duration::from_millis(5));
    turn.start_prompt("Try something else.");
    turn.append_transcript(assistant_text("next", "Here is something else."));
    turn.pass();
    assert_eq!(turn.output_rows(), ["Here is something else."]);
}

#[test]
fn a_prompt_queued_before_the_stop_owns_what_claude_writes_once_it_is_handed_over() {
    let mut turn = running_turn("queued-before-stop");
    turn.hook_event("UserPromptSubmit");
    turn.append_transcript(tool_call("toolu_stopped", "sleep 20"));
    turn.pass();
    let stopped = turn.active_prompt_id();

    // The next prompt is submitted while the turn still runs: it waits in the
    // queue and keeps its submission time.
    turn.app
        .prompt_owner_submit_prepared_prompt(
            &turn.session_id,
            crate::session::PromptQueueItem::new(
                "prompt-queued-before-stop",
                &turn.attachment_id,
                &turn.agent_id,
                "Then list the files.",
                crate::session::PromptStatus::Queued,
            ),
            true,
        )
        .expect("the prompt should queue");
    std::thread::sleep(std::time::Duration::from_millis(5));
    crate::app::fence_cancelled_claude_turn(&turn.run, &stopped);
    turn.app
        .prompt_owner_cancel_active_prompt_only(&turn.session_id, &turn.agent_id)
        .expect("the turn should cancel");
    turn.append_transcript(tool_result("toolu_stopped", "[Request interrupted]"));
    turn.append_transcript(assistant_text("stopped-tail", "I was interrupted."));
    turn.pass();
    assert!(turn.output_rows().is_empty(), "{:?}", turn.output_rows());

    // The queued prompt is admitted and handed to Claude: what Claude writes
    // from now on is its turn.
    turn.app
        .prompt_owner_activate_next_queued_prompt_for_dispatch(
            &turn.session_id,
            &turn.agent_id,
            None,
        )
        .expect("the queued prompt should activate")
        .expect("the queued prompt should be next");
    turn.pass();
    turn.append_transcript(tool_call("toolu_next", "ls"));
    turn.append_transcript(assistant_text("next", "Here are the files."));
    turn.pass();
    assert!(
        turn.tool_rows()
            .iter()
            .any(|row| row.contains("toolu_next")),
        "the queued prompt's tool row was dropped: {:?}",
        turn.tool_rows()
    );
    assert_eq!(turn.output_rows(), ["Here are the files."]);
}

