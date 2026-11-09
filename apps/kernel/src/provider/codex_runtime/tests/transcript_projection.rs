use super::*;
use crate::provider::codex_runtime::transcript::{append_text_delta, sync_completed_text_item};

#[test]
fn completed_text_backfill_deduplicates_alternate_item_ids() {
    let mut text_items = BTreeMap::new();
    let mut chunks = Vec::new();
    append_text_delta(
        &mut text_items,
        "msg-live",
        "codex-agent-message",
        TerminalOutputKind::ProviderOutput,
        "DO",
        &mut chunks,
    );

    let backfill = sync_completed_text_item(
        &mut text_items,
        &json!({
            "type": "agentMessage",
            "id": "item-summary",
            "phase": "final_answer",
            "text": "DONE"
        }),
    )
    .expect("backfill should append only the missing suffix");
    assert_eq!(backfill.merge_key.as_deref(), Some("msg-live"));
    assert_eq!(backfill.bytes, b"NE");
    assert!(sync_completed_text_item(
        &mut text_items,
        &json!({
            "type": "agentMessage",
            "id": "item-summary",
            "phase": "final_answer",
            "text": "DONE"
        }),
    )
    .is_none());
}

#[test]
fn reasoning_and_agent_deltas_preserve_item_merge_keys() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ReasoningTextDelta {
            item_id: "reason-1".to_string(),
            delta: "thinking".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "msg-1".to_string(),
            delta: "answer".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (
                chunk.kind.clone(),
                chunk.merge_key.clone().unwrap_or_default(),
                String::from_utf8_lossy(&chunk.bytes).into_owned()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                TerminalOutputKind::ProviderReasoning,
                "reason-1".to_string(),
                "thinking".to_string()
            ),
            (
                TerminalOutputKind::ProviderOutput,
                "msg-1".to_string(),
                "answer".to_string()
            ),
        ]
    );
}

#[test]
fn completed_agent_message_snapshot_is_rendered_without_delta() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "agentMessage",
                "id": "msg-1",
                "text": "final answer",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        chunks,
        vec![CodexOutputChunk {
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("msg-1".to_string()),
            bytes: b"final answer".to_vec(),
        }]
    );
    assert!(completions.is_empty());
    assert!(!prompt_completed);
}

#[test]
fn completed_agent_message_snapshot_only_emits_missing_suffix_after_delta() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "msg-1".to_string(),
            delta: "hello".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "agentMessage",
                "id": "msg-1",
                "text": "hello world",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (
                chunk.kind.clone(),
                chunk.merge_key.clone().unwrap_or_default(),
                String::from_utf8_lossy(&chunk.bytes).into_owned()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                TerminalOutputKind::ProviderOutput,
                "msg-1".to_string(),
                "hello".to_string()
            ),
            (
                TerminalOutputKind::ProviderOutput,
                "msg-1".to_string(),
                " world".to_string()
            ),
        ]
    );
}

#[test]
fn completed_reasoning_snapshot_is_rendered_without_delta() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "reasoning",
                "id": "reason-1",
                "summary": ["first", "second"],
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        chunks,
        vec![CodexOutputChunk {
            kind: TerminalOutputKind::ProviderReasoning,
            merge_key: Some("reason-1".to_string()),
            bytes: b"first\nsecond".to_vec(),
        }]
    );
}

#[test]
fn token_usage_notification_is_projected() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TokenUsageUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            usage: ProviderRunTokenUsage {
                total_tokens: Some(42_100),
                last_tokens: Some(8_900),
                context_tokens: Some(8_900),
                context_window: Some(128_000),
            },
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        resolved_usage,
        Some(ProviderRunTokenUsage {
            total_tokens: Some(42_100),
            last_tokens: Some(8_900),
            context_tokens: Some(8_900),
            context_window: Some(128_000),
        })
    );
    assert!(chunks.is_empty());
    assert!(completions.is_empty());
    assert!(notices.is_empty());
    assert!(!prompt_completed);
    assert!(terminal_failure.is_none());
}

#[test]
fn command_execution_updates_are_rendered_cumulatively() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemStarted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "ls -la",
                "cwd": "/tmp",
                "status": "inProgress",
                "commandActions": [],
                "aggregatedOutput": null,
                "exitCode": null,
                "durationMs": null,
                "processId": "pty-1",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::CommandExecutionOutputDelta {
            item_id: "cmd-1".to_string(),
            delta: "alpha\n".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::CommandExecutionOutputDelta {
            item_id: "cmd-1".to_string(),
            delta: "beta\n".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "ls -la",
                "cwd": "/tmp",
                "status": "completed",
                "commandActions": [],
                "aggregatedOutput": "alpha\nbeta\n",
                "exitCode": 0,
                "durationMs": 42,
                "processId": "pty-1",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    let tool_chunks = chunks
        .into_iter()
        .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
        .collect::<Vec<CodexOutputChunk>>();
    assert_eq!(tool_chunks.len(), 4);

    let second = parse_tool_chunk(&tool_chunks[1]);
    assert_eq!(second["tool"], "bash");
    assert_eq!(second["status"], "running");
    assert_eq!(second["output"], "alpha");

    let third = parse_tool_chunk(&tool_chunks[2]);
    assert_eq!(third["output"], "alpha\nbeta");

    let fourth = parse_tool_chunk(&tool_chunks[3]);
    assert_eq!(fourth["status"], "completed");
    assert_eq!(fourth["output"], "alpha\nbeta");
}

#[test]
fn codex_exec_command_events_render_as_command_execution_tool_updates() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ExecCommandStarted {
            call_id: "cmd-event-1".to_string(),
            command: json!("/bin/zsh -lc 'pwd'"),
            cwd: Some("/tmp".to_string()),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ExecCommandOutputDelta {
            call_id: "cmd-event-1".to_string(),
            chunk: "b2sK".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ExecCommandCompleted {
            call_id: "cmd-event-1".to_string(),
            command: json!("/bin/zsh -lc 'pwd'"),
            cwd: Some("/tmp".to_string()),
            output: Some("ok\n".to_string()),
            exit_code: Some(0),
            success: Some(true),
            stderr: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    let tool_chunks = chunks
        .into_iter()
        .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
        .collect::<Vec<CodexOutputChunk>>();
    assert_eq!(tool_chunks.len(), 3);

    let first = parse_tool_chunk(&tool_chunks[0]);
    assert_eq!(first["tool"], "bash");
    assert_eq!(first["status"], "running");
    assert_eq!(first["input"]["command"], "/bin/zsh -lc 'pwd'");

    let second = parse_tool_chunk(&tool_chunks[1]);
    assert_eq!(second["output"], "ok");

    let third = parse_tool_chunk(&tool_chunks[2]);
    assert_eq!(third["status"], "completed");
    assert_eq!(third["output"], "ok");
    assert!(!prompt_completed);
    assert!(completions.is_empty());
}

#[test]
fn mcp_tool_progress_is_projected_into_tool_text() {
    let rendered = render_codex_tool_transcript_update(
        &CodexToolTranscriptState {
            item: json!({
                "type": "mcpToolCall",
                "id": "tool-1",
                "server": "chariox-runtime",
                "tool": "validate_workflow_handoff",
                "status": "inProgress",
                "arguments": { "value": 1 },
                "result": null,
                "error": null,
                "durationMs": null
            }),
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: vec!["checking schema".to_string()],
            last_emitted: None,
            pictures: None,
            started_at_ms: 0,
            background: false,
            finished_at_ms: None,
        },
        &crate::extension::RemoteExtensionManifest::default(),
    )
    .expect("payload should render");

    let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
    assert_eq!(parsed["tool"], "validate_workflow_handoff");
    assert_eq!(parsed["title"], "chariox-runtime");
    assert_eq!(parsed["text"], "checking schema");
}

#[test]
fn home_proxy_mcp_tool_is_projected_into_tool_placement() {
    let manifest = crate::extension::RemoteExtensionManifest {
        tools: vec![crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Mcp,
            name: "Home MCP".to_string(),
            tool_name: "home-mcp".to_string(),
            description: "Runs on home".to_string(),
            input_schema: json!({ "type": "object" }),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(5),
            version_hash: Some("hash-1".to_string()),
        }],
        ..crate::extension::RemoteExtensionManifest::default()
    };
    let rendered = render_codex_tool_transcript_update(
        &CodexToolTranscriptState {
            item: json!({
                "type": "mcpToolCall",
                "id": "tool-home",
                "server": "home-mcp",
                "tool": "lookup",
                "status": "completed",
                "arguments": { "value": 1 },
                "result": { "ok": true }
            }),
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
            pictures: None,
            started_at_ms: 0,
            background: false,
            finished_at_ms: None,
        },
        &manifest,
    )
    .expect("payload should render");

    let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
    assert_eq!(parsed["placement"], "home-proxy");
    assert_eq!(parsed["authority"], "home");
    assert_eq!(parsed["execution_location"], "home");
}

// MP-08 / MP-10: a web search has no status field; the started notification
// is running and the completed one, with the final query, is completed.
#[test]
fn web_search_status_follows_the_started_and_completed_notifications() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;
    let mut apply = |notification| {
        apply_notification(
            notification,
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        )
    };
    apply(CodexNotification::ItemStarted {
        item: json!({"type": "webSearch", "id": "ws-1", "query": ""}),
    });
    apply(CodexNotification::ItemCompleted {
        item: json!({"type": "webSearch", "id": "ws-1", "query": "svg viewbox"}),
    });

    let snapshots = chunks
        .iter()
        .map(|chunk| serde_json::from_slice::<Value>(&chunk.bytes).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0]["status"], "running");
    assert_eq!(snapshots[1]["status"], "completed");
    assert_eq!(snapshots[1]["input"], json!({"query": "svg viewbox"}));
}

// The notification the app-server sends when `view_image` finishes reaches the transcript with its picture.
#[test]
fn a_completed_view_image_notification_carries_the_picture_of_the_agents_working_directory() {
    let workspace = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "chariox-view-image-notification-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    // A GIF header is enough for the content sniff.
    std::fs::write(workspace.join("chart.gif"), b"GIF89a\x01\x00\x01\x00").unwrap();
    let item = json!({"type": "imageView", "id": "iv-1", "path": workspace.join("chart.gif")});
    let run = |workspace_root: Option<&std::path::Path>| {
        let mut chunks = Vec::new();
        crate::provider::codex_runtime::events::apply_notification_with_manifest(
            CodexNotification::ItemCompleted { item: item.clone() },
            &mut None,
            &mut CodexTurnTracker::default(),
            &mut BTreeMap::new(),
            &mut BTreeMap::new(),
            &mut chunks,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut false,
            &mut None,
            &mut None,
            &crate::extension::RemoteExtensionManifest::default(),
            workspace_root,
        );
        let chunk = chunks.pop().expect("the call is shown");
        serde_json::from_slice::<Value>(&chunk.bytes).unwrap()
    };
    let with_directory = run(Some(&workspace));
    assert_eq!(with_directory["tool"], "view_image");
    assert_eq!(with_directory["images"].as_array().map(Vec::len), Some(1));
    assert!(with_directory["images"][0]
        .as_str()
        .unwrap()
        .starts_with("data:image/gif;base64,"));
    assert!(
        run(None).get("images").is_none(),
        "without a working directory nothing is read"
    );
    std::fs::remove_dir_all(workspace).unwrap();
}

/// One Codex app-server connection's projection state, fed wire notifications
/// in the codex-cli 0.159.3 app-server v2 shapes.
#[derive(Default)]
struct CodexWire {
    active_turn_id: Option<String>,
    turn_tracker: CodexTurnTracker,
    text_items:
        BTreeMap<String, crate::provider::codex_runtime::transcript::CodexTextTranscriptState>,
    tool_items: BTreeMap<String, CodexToolTranscriptState>,
}

impl CodexWire {
    /// The tool snapshots one notification produced, by call id.
    fn receive(&mut self, method: &str, params: Value) -> Vec<(String, Value)> {
        let mut chunks = Vec::new();
        apply_notification(
            crate::provider::codex_client::fixture_codex_notification(method, params),
            &mut self.active_turn_id,
            &mut self.turn_tracker,
            &mut self.text_items,
            &mut self.tool_items,
            &mut chunks,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut false,
            &mut None,
            &mut None,
        );
        chunks
            .into_iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
            .map(|chunk| {
                (
                    chunk.merge_key.unwrap_or_default(),
                    serde_json::from_slice(&chunk.bytes).unwrap(),
                )
            })
            .collect()
    }
}

fn command_item(id: &str, command: &str, status: &str, exit_code: Option<i64>) -> Value {
    json!({
        "type": "commandExecution", "id": id, "command": command, "cwd": "/workspace",
        "processId": format!("pty-{id}"), "source": "agent", "status": status,
        "commandActions": [{"type": "unknown", "command": command}],
        "aggregatedOutput": exit_code.map(|_| "done\n"), "exitCode": exit_code,
        "durationMs": exit_code.map(|_| 40_012),
    })
}

fn item_event(turn_id: &str, item: Value) -> Value {
    json!({"threadId": "thread-1", "turnId": turn_id, "item": item})
}

fn turn_completed(turn_id: &str) -> Value {
    json!({"threadId": "thread-1", "turn": {"id": turn_id, "status": "completed", "items": [], "error": null}})
}

#[test]
fn codex_commands_left_running_settle_on_their_own_exit_after_newer_turns() {
    let mut wire = CodexWire {
        active_turn_id: Some("turn-1".to_string()),
        ..Default::default()
    };
    let before = crate::session::unix_epoch_ms();
    // A foreground command: started time only.
    let started = wire.receive(
        "item/started",
        item_event("turn-1", command_item("call_fg", "ls", "inProgress", None)),
    );
    let completed = wire.receive(
        "item/completed",
        item_event(
            "turn-1",
            command_item("call_fg", "ls", "completed", Some(0)),
        ),
    );
    assert_eq!(completed[0].1["status"], "completed");
    assert_eq!(
        completed[0].1["started_at_ms"],
        started[0].1["started_at_ms"]
    );
    let foreground_start = started[0].1["started_at_ms"].as_u64().unwrap();
    assert!(foreground_start >= before, "{}", started[0].1);
    for (_, snapshot) in started.iter().chain(&completed) {
        assert!(snapshot.get("background").is_none(), "{snapshot}");
        assert!(snapshot.get("finished_at_ms").is_none(), "{snapshot}");
    }

    // Two commands the agent leaves running; one it polls through its terminal.
    let polled = wire.receive(
        "item/started",
        item_event(
            "turn-1",
            command_item("call_poll", "npm run dev", "inProgress", None),
        ),
    );
    wire.receive(
        "item/started",
        item_event(
            "turn-1",
            command_item("call_sleep", "sleep 40", "inProgress", None),
        ),
    );
    let interaction = wire.receive(
        "item/commandExecution/terminalInteraction",
        json!({"threadId": "thread-1", "turnId": "turn-1", "itemId": "call_poll", "processId": "pty-call_poll", "stdin": ""}),
    );
    assert_eq!(interaction.len(), 1);
    assert_eq!(interaction[0].0, "call_poll");
    assert_eq!(interaction[0].1["background"], true);
    assert!(interaction[0].1["finished_at_ms"].is_null());
    assert_eq!(
        interaction[0].1["started_at_ms"],
        polled[0].1["started_at_ms"]
    );

    // The turn completes with `sleep 40` still running: it becomes background too.
    let settled = wire.receive("turn/completed", turn_completed("turn-1"));
    assert_eq!(settled.len(), 1, "{settled:?}");
    assert_eq!(settled[0].0, "call_sleep");
    assert_eq!(settled[0].1["status"], "running");
    assert_eq!(settled[0].1["background"], true);
    assert!(settled[0].1["finished_at_ms"].is_null());

    // A newer turn runs; the old turn's exits still settle their own calls.
    wire.active_turn_id = Some("turn-2".to_string());
    wire.receive("turn/started", json!({"threadId": "thread-1", "turn": {"id": "turn-2", "status": "inProgress", "items": []}}));
    let exited = wire.receive(
        "item/completed",
        item_event(
            "turn-1",
            command_item("call_sleep", "sleep 40", "completed", Some(0)),
        ),
    );
    assert_eq!(exited.len(), 1);
    assert_eq!(exited[0].0, "call_sleep");
    assert_eq!(exited[0].1["status"], "completed");
    assert_eq!(exited[0].1["background"], true);
    assert_eq!(exited[0].1["started_at_ms"], settled[0].1["started_at_ms"]);
    assert!(exited[0].1["finished_at_ms"].as_u64().unwrap() >= foreground_start);
    // Killed by the agent: the exit reports failure and still settles.
    let killed = wire.receive(
        "item/completed",
        item_event(
            "turn-1",
            command_item("call_poll", "npm run dev", "failed", Some(143)),
        ),
    );
    assert_eq!(killed[0].1["status"], "error");
    assert!(killed[0].1["finished_at_ms"].is_u64());
    // An unrelated old-turn item is still ignored once its turn is over.
    assert!(wire
        .receive(
            "item/completed",
            item_event(
                "turn-1",
                command_item("call_other", "true", "completed", Some(0))
            )
        )
        .is_empty());
}

#[test]
fn codex_background_lifecycle_matches_the_protocol_snapshot() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../local/api/tests/protocol_shapes/provider-tool-background-lifecycle.json"
    ))
    .unwrap();
    let mut wire = CodexWire {
        active_turn_id: Some("turn-1".to_string()),
        ..Default::default()
    };
    wire.receive(
        "item/started",
        item_event(
            "turn-1",
            command_item("call_sleep", "sleep 40", "inProgress", None),
        ),
    );
    let mut running = wire
        .receive("turn/completed", turn_completed("turn-1"))
        .remove(0)
        .1;
    wire.active_turn_id = None;
    let mut finished = wire
        .receive(
            "item/completed",
            item_event(
                "turn-1",
                command_item("call_sleep", "sleep 40", "completed", Some(0)),
            ),
        )
        .remove(0)
        .1;
    // The kernel clock stamps these; pin them to the snapshot's values.
    assert!(finished["finished_at_ms"].as_u64() >= running["started_at_ms"].as_u64());
    for snapshot in [&mut running, &mut finished] {
        assert!(snapshot["started_at_ms"].is_u64());
        snapshot["started_at_ms"] = json!(1_793_700_000_000_u64);
    }
    finished["finished_at_ms"] = json!(1_793_700_040_000_u64);
    assert_eq!(running, fixture["codex_background_running"]);
    assert_eq!(finished, fixture["codex_background_finished"]);
}
