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
    running_turn_of(name, "claude-headless")
}

fn running_turn_of(name: &str, provider: &str) -> RunningTurn {
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
        provider,
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

    /// The assistant rows as a client builds them from the live records: a
    /// record is added to its merge key's row, or replaces it.
    fn live_rows(&self) -> Vec<String> {
        let mut rows: Vec<(Option<String>, String)> = Vec::new();
        for record in self.app.terminal().output_records() {
            if record.kind != TerminalOutputKind::ProviderOutput {
                continue;
            }
            let text = String::from_utf8_lossy(&record.bytes).into_owned();
            match rows
                .iter_mut()
                .find(|(key, _)| key.is_some() && *key == record.merge_key)
            {
                Some((_, row)) if record.replaces => *row = text,
                Some((_, row)) => row.push_str(&text),
                None => rows.push((record.merge_key, text)),
            }
        }
        rows.into_iter().map(|(_, row)| row).collect()
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

#[test]
fn a_prompt_typed_in_claudes_own_terminal_after_a_stop_owns_what_claude_writes_next() {
    let mut turn = running_turn_of("native-prompt-after-stop", "claude");
    turn.hook_event("UserPromptSubmit");
    turn.append_transcript(tool_call("toolu_stopped", "sleep 20"));
    turn.pass();
    let stopped = turn.active_prompt_id();
    crate::app::fence_cancelled_claude_turn(&turn.run, &stopped);
    turn.app
        .prompt_owner_cancel_active_prompt_only(&turn.session_id, &turn.agent_id)
        .expect("the turn should cancel");
    turn.append_transcript(assistant_text("stopped-tail", "I was interrupted."));
    turn.pass();
    assert!(turn.output_rows().is_empty(), "{:?}", turn.output_rows());

    // The person types the next prompt into Claude's own terminal: only the
    // UserPromptSubmit hook tells the kernel.
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(
        &turn.events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Now list the files.",
            "transcript_path": turn.transcript.display().to_string(),
        })
        .to_string()
            + "\n",
    )
    .expect("hook event should be written");
    turn.pass();
    turn.append_transcript(tool_call("toolu_next", "ls"));
    turn.append_transcript(assistant_text("next", "Here are the files."));
    turn.pass();
    assert!(
        turn.tool_rows()
            .iter()
            .any(|row| row.contains("toolu_next")),
        "the follow-up's tool row was dropped: {:?}",
        turn.tool_rows()
    );
    assert_eq!(turn.output_rows(), ["Here are the files."]);
}

/// The terminal reads of a recorded Claude Code 2.1.296 session (40×120).
fn recorded_terminal(recording: &str) -> Vec<Vec<u8>> {
    use base64::Engine;
    recording
        .lines()
        .map(|line| {
            let read: serde_json::Value = serde_json::from_str(line).unwrap();
            base64::engine::general_purpose::STANDARD
                .decode(read[1].as_str().unwrap())
                .unwrap()
        })
        .collect()
}

impl RunningTurn {
    fn terminal_shows(&mut self, bytes: &[u8]) {
        ProviderOutputClaudeNativeBridge::new(&mut self.app)
            .stream_reply(&self.session_id, self.run.id(), &self.run, bytes)
            .expect("the terminal should be read");
    }

    /// The assistant rows a reload of the session shows.
    fn stored_replies(&mut self) -> Vec<String> {
        let session = self
            .app
            .sessions
            .get_session(&self.session_id)
            .expect("session should exist");
        self.app
            .load_session_history_entries(&session, Some(&self.agent_id))
            .expect("history should load")
            .into_iter()
            .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::ProviderOutput)
            .map(|entry| entry.text)
            .collect()
    }
}

#[test]
fn a_reply_streams_from_the_terminal_and_its_transcript_block_completes_the_same_row() {
    const PLAIN: &str = include_str!("fixtures/claude-2.1.296-reply-40x120.pty.jsonl");
    const PLAIN_REPLY: &str = include_str!("fixtures/claude-2.1.296-reply-40x120.reply.txt");
    let mut turn = running_turn("streamed-reply");
    turn.hook_event("UserPromptSubmit");
    turn.pass();

    for bytes in recorded_terminal(PLAIN) {
        turn.terminal_shows(&bytes);
    }
    // Before Claude writes the block to its transcript, it is already out.
    let streamed = turn.output_rows().concat();
    assert!(PLAIN_REPLY.starts_with(&streamed), "{streamed}");
    assert!(streamed.len() * 10 >= PLAIN_REPLY.len() * 9);
    assert_eq!(turn.stored_replies(), [streamed.as_str()], "one stored row");

    turn.append_transcript(assistant_text("reply", PLAIN_REPLY));
    turn.pass();
    assert_eq!(
        turn.live_rows(),
        [PLAIN_REPLY],
        "the streamed row becomes the transcript text"
    );
    assert_eq!(turn.stored_replies(), [PLAIN_REPLY]);
}

#[test]
fn a_reply_drawn_with_emphasis_streams_plain_and_the_transcript_text_replaces_it() {
    const EMPHASIS: &str = include_str!("fixtures/claude-2.1.296-reply-with-emphasis.pty.jsonl");
    const EMPHASIS_REPLY: &str =
        include_str!("fixtures/claude-2.1.296-reply-with-emphasis.reply.txt");
    let mut turn = running_turn("streamed-emphasis");
    turn.hook_event("UserPromptSubmit");
    turn.pass();

    for bytes in recorded_terminal(EMPHASIS) {
        turn.terminal_shows(&bytes);
    }
    let streamed = turn.output_rows().concat();
    assert!(
        streamed.len() * 10 >= EMPHASIS_REPLY.len() * 8,
        "emphasis must not stop the stream: {streamed}"
    );

    turn.append_transcript(assistant_text("reply", EMPHASIS_REPLY));
    turn.pass();
    assert_eq!(
        turn.stored_replies(),
        [EMPHASIS_REPLY],
        "the transcript text takes the streamed row's place, nothing else stays"
    );
    assert_eq!(
        turn.live_rows(),
        [EMPHASIS_REPLY],
        "live clients show the markdown, the row the stream withheld included"
    );
}

#[test]
fn a_heading_reply_shows_its_last_row_live_without_reloading_history() {
    const HEADING: &str = include_str!("fixtures/claude-2.1.296-reply-with-heading.pty.jsonl");
    const HEADING_REPLY: &str =
        include_str!("fixtures/claude-2.1.296-reply-with-heading.reply.txt");
    let mut turn = running_turn("streamed-heading");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    for bytes in recorded_terminal(HEADING) {
        turn.terminal_shows(&bytes);
    }
    assert!(!turn.output_rows().concat().contains("Final plain line."));

    turn.append_transcript(assistant_text("reply", HEADING_REPLY));
    turn.pass();
    assert_eq!(
        turn.live_rows(),
        [HEADING_REPLY],
        "the complete answer, as live clients show it"
    );
    assert_eq!(turn.stored_replies(), [HEADING_REPLY]);
}

fn screen(rows: &[&str]) -> Vec<u8> {
    let mut bytes = b"\x1b[2J\x1b[H".to_vec();
    for row in rows {
        bytes.extend_from_slice(row.as_bytes());
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

#[test]
fn a_code_block_streamed_before_its_last_paragraph_is_not_repeated_by_the_transcript() {
    let mut turn = running_turn("streamed-code-block");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Here is the loop:",
        "",
        "  fn main() {",
        "      run();",
        "  }",
        "",
        "  It runs once and stops.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(
        turn.output_rows().concat(),
        "Here is the loop:\n\nfn main() {\n    run();\n}"
    );

    let reply =
        "Here is the loop:\n\n```rust\nfn main() {\n    run();\n}\n```\n\nIt runs once and stops.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply], "every row once, as written");
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_command_with_a_wildcard_streamed_before_the_transcript_block_is_not_repeated() {
    let mut turn = running_turn("streamed-wildcard");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Run this:",
        "",
        "  ls *",
        "",
        "  Then check the output.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(turn.output_rows().concat(), "Run this:\n\nls *");

    let reply = "Run this:\n\n```\nls *\n```\n\nThen check the output.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply]);
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_url_wrapped_over_two_rows_keeps_its_end_when_the_transcript_block_arrives() {
    let mut turn = running_turn("streamed-wrapped-url");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let token = format!("https://example.com/{}", "a".repeat(160));
    let (head, last) = token.split_at(118);
    let drawn = screen(&[
        "● Open",
        &format!("  {head}"),
        &format!("  {last}"),
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let reply = format!("Open {token}");
    turn.append_transcript(assistant_text("reply", &reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply.as_str()], "the URL keeps its end");
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn bold_labels_streamed_before_the_last_paragraph_do_not_cost_the_paragraph() {
    let mut turn = running_turn("streamed-bold-labels");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let labels = [
        "CPU", "RAM", "GPU", "SSD", "HDD", "NIC", "USB", "DNS", "TLS", "API",
    ];
    let drawn_row = labels.join(" ");
    let drawn = screen(&[
        &format!("● {drawn_row}"),
        "",
        "  Ready.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let reply = format!(
        "{}\n\nReady.",
        labels.map(|label| format!("**{label}**")).join(" ")
    );
    turn.append_transcript(assistant_text("reply", &reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply.as_str()]);
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_stopped_turn_streams_nothing_more() {
    const PLAIN: &str = include_str!("fixtures/claude-2.1.296-reply-40x120.pty.jsonl");
    let mut turn = running_turn("streamed-stop");
    let stopped = turn.active_prompt_id();
    crate::app::fence_cancelled_claude_turn(&turn.run, &stopped);
    for bytes in recorded_terminal(PLAIN) {
        turn.terminal_shows(&bytes);
    }
    assert!(turn.output_rows().is_empty(), "{:?}", turn.output_rows());
}

#[test]
fn a_command_in_a_language_tagged_fence_is_shown_once_live() {
    let mut turn = running_turn("streamed-tagged-fence");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&["● Run:", "", "  $ sh", "", "  Done.", "", "✻ Working (3s)"]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let reply = "Run:\n\n```sh\n$ sh\n```\n\nDone.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply]);
    let records = turn.app.terminal().output_records();
    assert!(
        records.last().is_some_and(|record| record.replaces),
        "the transcript text goes out as a replacement, not added to the stream"
    );
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_wrapped_code_token_keeps_its_symbol_only_end_live() {
    let mut turn = running_turn("streamed-wrapped-symbols");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let identifier = format!("f{}", "a".repeat(117));
    let drawn = screen(&[
        "● Run:",
        "",
        &format!("  {identifier}"),
        "  ();",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let reply = format!("Run:\n\n```\n{identifier}();\n```");
    turn.append_transcript(assistant_text("reply", &reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply.as_str()]);
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_reply_streamed_before_a_kernel_restart_is_replaced_not_shown_twice() {
    let mut turn = running_turn("streamed-restart");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Here is the plan:",
        "",
        "  First we read the file.",
        "",
        "  Then we write it.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(
        turn.live_rows(),
        ["Here is the plan:\n\nFirst we read the file."]
    );
    // The kernel restarts: what the reader streamed is gone, history keeps the row.
    super::live_reply::forget(turn.run.id());

    let reply = "Here is the plan:\n\n1. First we **read** the file.\n\nThen we write it.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply], "one row, the transcript's");
    assert_eq!(turn.stored_replies(), [reply]);
    // Claude draws the completed block again, with no reader that knows it was completed.
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(turn.live_rows(), [reply], "a redraw adds no row");
    assert_eq!(turn.stored_replies(), [reply]);
}

// The same, when a reader had already taken over (it streamed nothing yet) before the transcript text arrived.
#[test]
fn a_block_completed_without_its_reader_is_not_streamed_again_even_by_a_reader_that_took_over_before(
) {
    let mut turn = running_turn("streamed-restart-fresh-reader");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Here is the plan:",
        "",
        "  First we read the file.",
        "",
        "  Then we write it.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    super::live_reply::forget(turn.run.id());
    // A reader takes over while the screen shows only that the turn is working.
    turn.terminal_shows(&screen(&["✻ Working (4s)"]));
    let reply = "Here is the plan:\n\n1. First we **read** the file.\n\nThen we write it.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(turn.live_rows(), [reply], "a redraw adds no row");
    assert_eq!(turn.stored_replies(), [reply]);
}

// The reader is lost while a block is still being written, and Claude draws that same block again before its transcript
// text arrives: the new reader streams the block from its start, and it must go on in the row already streamed, not
// open a second one that the transcript block then leaves behind as a duplicate prefix.
#[test]
fn an_unfinished_block_drawn_again_after_the_reader_was_lost_goes_on_in_its_row() {
    let mut turn = running_turn("streamed-resumed");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Here is the plan:",
        "",
        "  First we read the file.",
        "",
        "  Then we write it.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let partial = "Here is the plan:\n\nFirst we read the file.";
    assert_eq!(turn.live_rows(), [partial]);
    super::live_reply::forget(turn.run.id());
    // The same block is drawn again; the new reader reads it twice.
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    assert_eq!(
        turn.live_rows(),
        [partial],
        "the redrawn block goes on in the row it began in"
    );
    assert_eq!(turn.stored_replies(), [partial]);
    let reply = "Here is the plan:\n\n1. First we **read** the file.\n\nThen we write it.";
    turn.append_transcript(assistant_text("reply", reply));
    turn.pass();
    assert_eq!(turn.live_rows(), [reply], "one completed row");
    assert_eq!(turn.stored_replies(), [reply]);
}

#[test]
fn a_reply_longer_than_one_live_record_is_replaced_whole() {
    let mut turn = running_turn("streamed-long-reply");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● The long answer:",
        "",
        "  First paragraph of it.",
        "",
        "  Second paragraph.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let tail = "More of the **answer**, line after line.\n".repeat(1000);
    let reply =
        format!("The long answer:\n\nFirst paragraph of it.\n\nSecond paragraph.\n\n{tail}");
    assert!(
        reply.len()
            > 2 * crate::provider_output_policy::output_bounds::MAX_PROVIDER_OUTPUT_RECORD_BYTES
    );
    turn.append_transcript(assistant_text("reply", &reply));
    turn.pass();
    assert_eq!(
        turn.live_rows(),
        [reply.as_str()],
        "clients get the whole reply, not its first 16 KiB"
    );
}

// A reader that is lost (a kernel restart, or the registry evicting the least recently read run) starts counting its
// blocks again. What it streams next must not take the key of a block an earlier reader streamed: the key is the row's
// identity in history and on clients, so a reused one would replace the earlier answer.
#[test]
fn a_block_streamed_after_the_reader_was_lost_keeps_the_earlier_answer() {
    let mut turn = running_turn("streamed-after-reader-loss");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let first_drawn = screen(&[
        "● Checked the files.",
        "",
        "  All of them pass.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&first_drawn);
    turn.terminal_shows(&first_drawn);
    let first = "Checked the files.\n\n**All** of them pass.";
    turn.append_transcript(assistant_text("first", first));
    turn.pass();
    assert_eq!(
        turn.live_rows(),
        [first],
        "the first block is the transcript's"
    );
    // The reader is lost; the screen it had is gone. Claude draws the completed block again before anything new.
    super::live_reply::forget(turn.run.id());
    turn.terminal_shows(&first_drawn);
    turn.terminal_shows(&first_drawn);
    assert_eq!(
        turn.live_rows(),
        [first],
        "the completed block is not streamed again"
    );
    assert_eq!(turn.stored_replies(), [first]);
    let second_drawn = screen(&[
        "● Then the build.",
        "",
        "  It is green.",
        "",
        "✻ Working (9s)",
    ]);
    turn.terminal_shows(&second_drawn);
    turn.terminal_shows(&second_drawn);
    assert_eq!(
        turn.live_rows(),
        [first, "Then the build."],
        "the second block streams as its own row; the first answer stays"
    );
    assert_eq!(
        turn.stored_replies(),
        [first, "Then the build."],
        "and so does history"
    );
    let second = "Then the build.\n\nIt is **green**.";
    turn.append_transcript(assistant_text("second", second));
    turn.pass();
    assert_eq!(turn.live_rows(), [first, second]);
    assert_eq!(turn.stored_replies(), [first, second]);
}

// A client puts the pieces of a long replacement into the row the first piece replaced, wherever that row now is:
// the first piece says it replaces, every later one says it continues that replacement, and nothing else does.
#[test]
fn the_pieces_of_a_long_replacement_say_which_is_first_and_which_continue_it() {
    let mut turn = running_turn("streamed-long-pieces");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● The long answer:",
        "",
        "  First paragraph of it.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let tail = "More of the **answer**, line after line.\n".repeat(1000);
    let reply = format!("The long answer:\n\nFirst paragraph of it.\n\n{tail}");
    turn.append_transcript(assistant_text("reply", &reply));
    turn.pass();
    let records = turn
        .app
        .terminal()
        .output_records()
        .into_iter()
        .filter(|record| record.kind == TerminalOutputKind::ProviderOutput)
        .collect::<Vec<_>>();
    let first = records
        .iter()
        .position(|record| record.replaces)
        .expect("the reply is a replacement");
    let pieces = &records[first..];
    assert!(pieces.len() >= 3, "the reply needs more than two records");
    assert!(pieces[0].replaces && !pieces[0].continues_replacement);
    for piece in &pieces[1..] {
        assert!(piece.continues_replacement && !piece.replaces, "{piece:?}");
        assert_eq!(piece.merge_key, pieces[0].merge_key);
    }
    assert!(records[..first]
        .iter()
        .all(|record| !record.replaces && !record.continues_replacement));
}

#[test]
fn a_later_block_of_a_prompt_that_opens_like_an_adopted_one_keeps_its_own_row() {
    let mut turn = running_turn("streamed-two-blocks");
    turn.hook_event("UserPromptSubmit");
    turn.pass();
    let drawn = screen(&[
        "● Checked the files.",
        "",
        "  All of them pass.",
        "",
        "✻ Working (3s)",
    ]);
    turn.terminal_shows(&drawn);
    turn.terminal_shows(&drawn);
    let first = "Checked the files.\n\nAll of them pass.";
    turn.append_transcript(assistant_text("first", first));
    turn.pass();
    // A second text block Claude never drew, opening like the first.
    let second = "Checked the files.\n\nAll of them pass. The build is green too.";
    turn.append_transcript(assistant_text("second", second));
    turn.pass();
    assert_eq!(turn.live_rows(), [first, second], "two blocks, two rows");
    assert_eq!(turn.stored_replies(), [first, second]);
}
