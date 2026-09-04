mod drain;
mod parts;
mod permission;
#[cfg(test)]
mod retry_status_tests;
mod snapshot;
mod state;
mod transcript;

pub(in crate::provider) use drain::drain_opencode_events;
pub(crate) use state::OpenCodeRuntimeState;
pub use state::{OpenCodeAssistantCompletion, OpenCodeOutputChunk};

#[cfg(test)]
use transcript::ToolTranscriptUpdate;

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use serde_json::json;

    use crate::extension::{
        ExtensionAuthority, ExtensionDefinitionOrigin, ExtensionExecutionLocation, ExtensionKind,
        RemoteExtensionManifest, RemoteExtensionTool,
    };
    use crate::provider::{
        opencode_client::{OpenCodeMessage, OpenCodePart, OpenCodeToolState},
        AgentEndpointMode, LaunchProviderRequest, OpenCodeEvent, ProviderLaunchResult,
        RuntimeProviderRun,
    };
    use crate::terminal::TerminalOutputKind;

    use super::{
        drain_opencode_events,
        parts::{handle_message_part_delta, handle_message_part_updated},
        snapshot::{collect_new_completed_assistant_messages, latest_assistant_usage_tokens},
        snapshot::{
            opencode_messages_active_prompt_failure, opencode_messages_have_empty_active_assistant,
            render_snapshot_output_chunks,
        },
        transcript::render_tool_transcript_update,
        OpenCodeAssistantCompletion, OpenCodeRuntimeState, ToolTranscriptUpdate,
    };

    pub(super) fn test_run() -> RuntimeProviderRun {
        RuntimeProviderRun::new(
            "provider-run-1",
            &LaunchProviderRequest::new(
                "session-1",
                "opencode",
                "opencode",
                "default",
                "opencode/test-model",
            )
            .with_agent_id("agent-1"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "opencode:test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("http://localhost:1".to_string()),
            },
        )
    }

    #[test]
    fn renders_structured_tool_update_with_input_and_output() {
        let payload = render_tool_transcript_update(
            &OpenCodePart {
                id: "part-1".to_string(),
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                kind: "tool".to_string(),
                text: String::new(),
                tool: "bash".to_string(),
                state: Some(OpenCodeToolState {
                    status: "completed".to_string(),
                    input: json!({ "command": "git status" }),
                    output: String::new(),
                    title: String::new(),
                    metadata: json!({
                        "output": "On branch main",
                        "description": "Shows working tree status"
                    }),
                    error: String::new(),
                    raw: String::new(),
                }),
                time: None,
            },
            &RemoteExtensionManifest::default(),
        );

        let parsed: ToolTranscriptUpdate =
            serde_json::from_str(&payload).expect("tool payload should deserialize");
        assert_eq!(parsed.id, "part-1");
        assert_eq!(parsed.tool, "bash");
        assert_eq!(parsed.status, "completed");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Shows working tree status")
        );
        assert_eq!(parsed.output.as_deref(), Some("On branch main"));
        assert_eq!(parsed.input, Some(json!({ "command": "git status" })));
    }

    #[test]
    fn renders_home_proxy_placement_for_remote_extension_tool_update() {
        let manifest = RemoteExtensionManifest {
            tools: vec![RemoteExtensionTool {
                kind: ExtensionKind::Script,
                name: "Home lookup".to_string(),
                tool_name: "home_lookup".to_string(),
                description: "Runs on home".to_string(),
                input_schema: json!({ "type": "object" }),
                authority: ExtensionAuthority::Home,
                definition_origin: ExtensionDefinitionOrigin::Home,
                execution_location: ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: Some(5),
                version_hash: Some("hash-1".to_string()),
            }],
        };
        let payload = render_tool_transcript_update(
            &OpenCodePart {
                id: "part-home".to_string(),
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                kind: "tool".to_string(),
                text: String::new(),
                tool: "home_lookup".to_string(),
                state: Some(OpenCodeToolState {
                    status: "completed".to_string(),
                    input: json!({ "query": "status" }),
                    output: "ok".to_string(),
                    title: String::new(),
                    metadata: json!({}),
                    error: String::new(),
                    raw: String::new(),
                }),
                time: None,
            },
            &manifest,
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("tool payload should deserialize");
        assert_eq!(parsed["tool"], "home_lookup");
        assert_eq!(parsed["placement"], "home-proxy");
        assert_eq!(parsed["authority"], "home");
        assert_eq!(parsed["execution_location"], "home");
    }

    #[test]
    fn snapshot_rendering_preserves_reasoning_and_text_order() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        state.note_prompt_submitted("msg_user".to_string());
        let rendered = render_snapshot_output_chunks(
            &mut state,
            &RemoteExtensionManifest::default(),
            &[crate::provider::OpenCodeMessage {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
                parts: vec![
                    OpenCodePart {
                        id: "part-1".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "reasoning".to_string(),
                        text: "first thought\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                    OpenCodePart {
                        id: "part-2".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "text".to_string(),
                        text: "first answer\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                    OpenCodePart {
                        id: "part-3".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "reasoning".to_string(),
                        text: "second thought\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                ],
            }],
        );
        let chunks = rendered.chunks;

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.kind.clone(),
                    String::from_utf8_lossy(&chunk.bytes).into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    TerminalOutputKind::ProviderReasoning,
                    "first thought\n".to_string()
                ),
                (
                    TerminalOutputKind::ProviderOutput,
                    "first answer\n".to_string()
                ),
                (
                    TerminalOutputKind::ProviderReasoning,
                    "second thought\n".to_string()
                ),
            ]
        );
    }

    #[test]
    fn snapshot_rendering_excludes_messages_from_before_the_active_prompt() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        let messages = vec![
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-old",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user_old",
                    "time": { "completed": 1 }
                },
                "parts": [{
                    "id": "part-old",
                    "sessionID": "session-1",
                    "messageID": "message-old",
                    "type": "text",
                    "text": "old answer"
                }]
            }))
            .expect("old message should deserialize"),
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-current",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user_current",
                    "time": { "completed": 2 }
                },
                "parts": [{
                    "id": "part-current",
                    "sessionID": "session-1",
                    "messageID": "message-current",
                    "type": "text",
                    "text": "current answer"
                }]
            }))
            .expect("current message should deserialize"),
        ];
        state.baseline_existing_messages(&messages[..1]);
        state.note_prompt_submitted("msg_user_current".to_string());

        let rendered = render_snapshot_output_chunks(
            &mut state,
            &RemoteExtensionManifest::default(),
            &messages,
        );

        assert_eq!(rendered.chunks.len(), 1);
        assert_eq!(
            rendered.chunks[0].merge_key.as_deref(),
            Some("part-current")
        );
        assert_eq!(rendered.chunks[0].bytes, b"current answer");

        let mut event_chunks = Vec::new();
        handle_message_part_updated(
            &mut state,
            "provider-run-1",
            &RemoteExtensionManifest::default(),
            messages[0].parts[0].clone(),
            &mut event_chunks,
        )
        .expect("historical event should be ignored");
        assert!(event_chunks.is_empty());
    }

    #[test]
    fn rebasing_before_a_new_prompt_excludes_an_unparented_abort_message() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        let aborted = serde_json::from_value::<OpenCodeMessage>(json!({
            "info": {
                "id": "message-aborted",
                "sessionID": "session-1",
                "role": "assistant",
                "time": { "completed": 1 }
            },
            "parts": [{
                "id": "part-aborted",
                "sessionID": "session-1",
                "messageID": "message-aborted",
                "type": "text",
                "text": "Aborted"
            }]
        }))
        .expect("abort message should deserialize");

        state.baseline_existing_messages(&[aborted]);
        state.note_prompt_submitted("msg_user_next".to_string());
        state.message_parent_ids.insert(
            "message-next".to_string(),
            Some("msg_user_next".to_string()),
        );

        assert!(!state.message_belongs_to_active_prompt("message-aborted"));
        assert!(state.message_belongs_to_active_prompt("message-next"));
    }

    #[test]
    fn abort_reset_discards_the_cancelled_turn_error_before_the_next_prompt() {
        let (event_sender, event_receiver) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(event_receiver),
        );
        state.note_prompt_submitted("msg_user_cancelled".to_string());
        event_sender
            .send(OpenCodeEvent::SessionError {
                session_id: "session-1".to_string(),
                message: "Aborted".to_string(),
            })
            .expect("abort error should queue");

        state.switch_session_after_abort("session-2".to_string());
        state.note_prompt_submitted("msg_user_next".to_string());
        let drained = drain_opencode_events(&test_run(), &mut state, None)
            .expect("next prompt drain should succeed");

        assert!(drained.terminal_failure.is_none());
        assert!(drained.notices.is_empty());
        assert!(!drained.prompt_completed);
    }

    #[test]
    fn late_attach_delta_waits_for_authoritative_part_prefix() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        state.note_prompt_submitted("msg_user".to_string());
        state
            .message_roles
            .insert("message-1".to_string(), "assistant".to_string());
        state
            .part_kinds
            .insert("part-1".to_string(), "text".to_string());
        let mut chunks = Vec::new();

        handle_message_part_updated(
            &mut state,
            "provider-run-1",
            &RemoteExtensionManifest::default(),
            OpenCodePart {
                id: "part-1".to_string(),
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                kind: "text".to_string(),
                text: String::new(),
                tool: String::new(),
                state: None,
                time: None,
            },
            &mut chunks,
        )
        .expect("empty part creation should not establish an offset baseline");

        handle_message_part_delta(
            &mut state,
            "provider-run-1",
            "session-1".to_string(),
            "message-1".to_string(),
            "part-1".to_string(),
            "text".to_string(),
            "feels indistinguishable from a local one".to_string(),
            &mut chunks,
        )
        .expect("late delta should buffer");
        assert!(chunks.is_empty());

        handle_message_part_updated(
            &mut state,
            "provider-run-1",
            &RemoteExtensionManifest::default(),
            OpenCodePart {
                id: "part-1".to_string(),
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                kind: "text".to_string(),
                text: "A remote worker feels indistinguishable from a local one".to_string(),
                tool: String::new(),
                state: None,
                time: None,
            },
            &mut chunks,
        )
        .expect("full part should establish its prefix");

        handle_message_part_delta(
            &mut state,
            "provider-run-1",
            "session-1".to_string(),
            "message-1".to_string(),
            "part-1".to_string(),
            "text".to_string(),
            " and completes the illusion.".to_string(),
            &mut chunks,
        )
        .expect("subsequent delta should append");

        let rendered = render_snapshot_output_chunks(
            &mut state,
            &RemoteExtensionManifest::default(),
            &[OpenCodeMessage {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
                parts: vec![OpenCodePart {
                    id: "part-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "text".to_string(),
                    text: "A remote worker feels indistinguishable from a local one and completes the illusion.".to_string(),
                    tool: String::new(),
                    state: None,
                    time: None,
                }],
            }],
        );
        assert!(rendered.chunks.is_empty());
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
            "A remote worker feels indistinguishable from a local one and completes the illusion."
        );
    }

    #[test]
    fn latest_assistant_usage_tokens_uses_the_newest_assistant_with_tokens() {
        let messages = vec![
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "tokens": {
                        "input": 100,
                        "output": 20,
                        "reasoning": 5,
                        "cache": { "read": 10, "write": 5 }
                    },
                    "time": { "completed": 1 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-2",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "tokens": {
                        "input": 200,
                        "output": 40,
                        "reasoning": 10,
                        "cache": { "read": 20, "write": 10 }
                    },
                    "time": { "completed": 2 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
        ];

        assert_eq!(latest_assistant_usage_tokens(&messages), Some(280));
    }

    #[test]
    fn terminal_assistant_for_active_prompt_waits_for_idle_before_completion() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert_eq!(
            first.completions,
            vec![OpenCodeAssistantCompletion {
                message_id: "message-1".to_string(),
                completed_at_ms: 1,
            }]
        );
        assert!(!first.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("second drain should succeed");
        assert!(second.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }

    #[test]
    fn detects_only_an_incomplete_empty_assistant_for_the_active_prompt() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        state.note_prompt_submitted("msg_user".to_string());
        let empty_active = serde_json::from_value::<OpenCodeMessage>(json!({
            "info": {
                "id": "message-empty",
                "sessionID": "session-1",
                "role": "assistant",
                "parentID": "msg_user",
                "time": {}
            },
            "parts": []
        }))
        .expect("empty assistant should deserialize");
        let unrelated = serde_json::from_value::<OpenCodeMessage>(json!({
            "info": {
                "id": "message-other",
                "sessionID": "session-1",
                "role": "assistant",
                "parentID": "another-user",
                "time": {}
            },
            "parts": []
        }))
        .expect("unrelated assistant should deserialize");

        assert!(opencode_messages_have_empty_active_assistant(
            &state,
            &[empty_active],
        ));
        assert!(!opencode_messages_have_empty_active_assistant(
            &state,
            &[unrelated],
        ));
        assert!(state.active_prompt_has_elapsed(std::time::Duration::ZERO));
    }

    #[test]
    fn idle_after_completed_assistant_without_parent_completes_active_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "finish": "stop",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert_eq!(
            first.completions,
            vec![OpenCodeAssistantCompletion {
                message_id: "message-1".to_string(),
                completed_at_ms: 1,
            }]
        );
        assert!(!first.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("second drain should succeed");
        assert!(second.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }

    #[test]
    fn idle_after_nonterminal_assistant_completion_keeps_active_prompt_open() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-intermediate",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "finish": "unknown",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(first.completions.is_empty());
        assert!(!first.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn completed_assistant_snapshot_dedupes_full_history_by_message_id() {
        let (_tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());
        let messages = vec![
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 1 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-2",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 2 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
        ];

        let first = collect_new_completed_assistant_messages(&mut state, &messages);
        assert_eq!(
            first
                .iter()
                .map(|completion| completion.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["message-1", "message-2"]
        );

        let second = collect_new_completed_assistant_messages(&mut state, &messages);
        assert!(second.is_empty());
    }

    #[test]
    fn prompt_submission_clears_prior_unparented_assistant_completion() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user_1".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "finish": "stop",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");
        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);

        state.note_prompt_submitted("msg_user_2".to_string());
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user_2"));
    }

    #[test]
    fn idle_status_without_submitted_prompt_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert!(result.completions.is_empty());
        assert!(result
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderStatus));
    }

    #[test]
    fn session_credit_error_ends_active_prompt_with_authoritative_failure() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionError {
                session_id: "session-1".to_string(),
                message: "Insufficient balance. Manage your billing to continue.".to_string(),
            },
        )
        .expect("session error should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(result.prompt_completed);
        assert_eq!(state.active_user_message_id, None);
        assert_eq!(
            result.terminal_failure.as_deref(),
            Some("Insufficient balance. Manage your billing to continue.")
        );
        assert_eq!(
            result
                .chunks
                .iter()
                .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderError)
                .count(),
            1
        );
    }

    #[test]
    fn assistant_message_error_ends_active_prompt_with_authoritative_failure() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-error",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "error": {
                        "name": "APIError",
                        "data": { "message": "Provider finish_reason: network_error" }
                    }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(result.prompt_completed);
        assert_eq!(state.active_user_message_id, None);
        assert_eq!(
            result.terminal_failure.as_deref(),
            Some("Provider finish_reason: network_error")
        );
        assert_eq!(
            result
                .chunks
                .iter()
                .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderError)
                .count(),
            1
        );
    }

    #[test]
    fn snapshot_error_is_not_a_successful_assistant_completion() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        state.note_prompt_submitted("msg_user".to_string());
        let messages = vec![OpenCodeMessage {
            info: serde_json::from_value(json!({
                "id": "message-error",
                "sessionID": "session-1",
                "role": "assistant",
                "parentID": "msg_user",
                "error": {
                    "name": "APIError",
                    "data": { "message": "Provider finish_reason: network_error" }
                }
            }))
            .expect("message info should deserialize"),
            parts: Vec::new(),
        }];
        state.baseline_existing_messages(&[]);
        super::snapshot::record_snapshot_message_metadata(&mut state, &messages);

        assert_eq!(
            opencode_messages_active_prompt_failure(&state, &messages, Some("msg_user")).as_deref(),
            Some("Provider finish_reason: network_error")
        );
    }

    #[test]
    fn preexisting_snapshot_error_for_recovered_prompt_is_authoritative() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        let messages = vec![OpenCodeMessage {
            info: serde_json::from_value(json!({
                "id": "message-error-while-kernel-was-down",
                "sessionID": "session-1",
                "role": "assistant",
                "parentID": "msg_user",
                "error": {
                    "name": "APIError",
                    "data": { "message": "Provider finish_reason: network_error" }
                }
            }))
            .expect("message info should deserialize"),
            parts: Vec::new(),
        }];
        state.baseline_existing_messages(&messages);
        state.note_prompt_submitted("msg_user".to_string());

        assert_eq!(
            opencode_messages_active_prompt_failure(&state, &messages, Some("msg_user")).as_deref(),
            Some("Provider finish_reason: network_error")
        );
    }

    #[test]
    fn later_sibling_error_overrides_completion_in_the_same_drain() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-completed",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 42 }
                }))
                .expect("completed message should deserialize"),
            },
        )
        .expect("completed message should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-retry-error",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "error": {
                        "name": "APIError",
                        "data": { "message": "Provider finish_reason: network_error" }
                    }
                }))
                .expect("error message should deserialize"),
            },
        )
        .expect("error message should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert_eq!(
            result.terminal_failure.as_deref(),
            Some("Provider finish_reason: network_error")
        );
        assert!(result.completions.is_empty());
        assert!(result.prompt_completed);
    }

    #[test]
    fn cross_session_sibling_error_does_not_fail_the_active_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-other-session-error",
                    "sessionID": "session-2",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "error": {
                        "name": "APIError",
                        "data": { "message": "Provider finish_reason: network_error" }
                    }
                }))
                .expect("cross-session error should deserialize"),
            },
        )
        .expect("cross-session error should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(result.terminal_failure.is_none());
        assert!(result.completions.is_empty());
        assert!(!result.prompt_completed);
    }

    #[test]
    fn idle_status_after_submitted_prompt_without_response_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn idle_after_observed_busy_status_does_not_complete_without_terminal_assistant() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "busy".into(),
            },
        )
        .expect("busy status should send");
        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("busy drain should succeed");
        assert!(!first.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");
        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("idle drain should succeed");

        assert!(!second.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn idle_after_unknown_assistant_step_waits_for_provider_continuation() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-unknown",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "unknown",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("unknown assistant update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("unknown step drain should succeed");
        assert!(!first.prompt_completed);
        assert!(first.completions.is_empty());
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-final",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 2 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("final assistant update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("final idle status should send");

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("final drain should succeed");
        assert!(second.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }

    #[test]
    fn unknown_assistant_becomes_eligible_only_after_terminal_update() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "unknown",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("unknown assistant update should send");
        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("unknown update should drain");
        assert!(first.completions.is_empty());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 2 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("terminal assistant update should send");
        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("terminal update should drain");
        assert_eq!(
            second.completions,
            vec![OpenCodeAssistantCompletion {
                message_id: "message-1".to_string(),
                completed_at_ms: 2,
            }]
        );
    }

    #[test]
    fn idle_status_after_assistant_text_does_not_complete_without_terminal_assistant() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user"
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "text-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "text".to_string(),
                    text: "answer".to_string(),
                    tool: String::new(),
                    state: None,
                    time: None,
                }),
            },
        )
        .expect("text part should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn tool_call_assistant_blocks_prompt_completion_until_final_assistant() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "tool-calls",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "tool-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "tool".to_string(),
                    text: String::new(),
                    tool: "bash".to_string(),
                    state: Some(OpenCodeToolState {
                        status: "running".to_string(),
                        input: json!({ "command": "git status" }),
                        output: String::new(),
                        title: String::new(),
                        metadata: json!({}),
                        error: String::new(),
                        raw: String::new(),
                    }),
                    time: None,
                }),
            },
        )
        .expect("running tool update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);
        assert!(first
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderTool));

        let second = drain_opencode_events(&test_run(), &mut state, None)
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "tool-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "tool".to_string(),
                    text: String::new(),
                    tool: "bash".to_string(),
                    state: Some(OpenCodeToolState {
                        status: "completed".to_string(),
                        input: json!({ "command": "git status" }),
                        output: "On branch main".to_string(),
                        title: String::new(),
                        metadata: json!({}),
                        error: String::new(),
                        raw: String::new(),
                    }),
                    time: None,
                }),
            },
        )
        .expect("completed tool update should send");

        let third = drain_opencode_events(&test_run(), &mut state, None)
            .expect("third drain should succeed");
        assert!(!third.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-2",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 2 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("final assistant update should send");

        let fourth = drain_opencode_events(&test_run(), &mut state, None)
            .expect("fourth drain should succeed");
        assert!(!fourth.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let fifth = drain_opencode_events(&test_run(), &mut state, None)
            .expect("fifth drain should succeed");
        assert!(fifth.prompt_completed);
    }

    #[test]
    fn idle_status_after_tool_call_only_assistant_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-tool-calls",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "tool-calls",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("tool-call assistant update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                status: "idle".into(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");
        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn foreign_session_message_does_not_override_run_selection() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        for (session_id, model_id, variant) in [
            ("title-session", "big-pickle", "low"),
            ("session-1", "gpt-5.2", "high"),
        ] {
            tx.send(
                crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                    info: serde_json::from_value(json!({
                        "id": format!("message-{session_id}"),
                        "sessionID": session_id,
                        "role": "assistant",
                        "providerID": "opencode",
                        "modelID": model_id,
                        "variant": variant
                    }))
                    .expect("message info should deserialize"),
                },
            )
            .expect("message update should send");
        }

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");
        assert_eq!(result.resolved_model.as_deref(), Some("opencode/gpt-5.2"));
        assert_eq!(result.resolved_variant.as_deref(), Some("high"));
        assert_eq!(result.resolved_model_source, Some("message.updated"));
        assert!(!state.message_roles.contains_key("message-title-session"));
        assert_eq!(
            state
                .message_roles
                .get("message-session-1")
                .map(String::as_str),
            Some("assistant")
        );
    }

    #[test]
    fn tool_call_only_message_completion_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-tool-calls",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "finish": "tool-calls",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(first.completions.is_empty());
        assert!(!first.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }
}
