use super::*;

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
                "server": "arroba-runtime",
                "tool": "validate_workflow_handoff",
                "status": "inProgress",
                "arguments": { "value": 1 },
                "result": null,
                "error": null,
                "durationMs": null
            }),
            streamed_output: String::new(),
            progress_messages: vec!["checking schema".to_string()],
            last_emitted: None,
        },
        &crate::extension::RemoteExtensionManifest::default(),
    )
    .expect("payload should render");

    let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
    assert_eq!(parsed["tool"], "validate_workflow_handoff");
    assert_eq!(parsed["title"], "arroba-runtime");
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
            progress_messages: Vec::new(),
            last_emitted: None,
        },
        &manifest,
    )
    .expect("payload should render");

    let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
    assert_eq!(parsed["placement"], "home-proxy");
    assert_eq!(parsed["authority"], "home");
    assert_eq!(parsed["execution_location"], "home");
}
