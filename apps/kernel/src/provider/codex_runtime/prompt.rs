//! Codex prompt turn start, interruption, and turn-id extraction.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::{CodexClient, CodexNotification, RuntimeProviderRun};

use super::input::codex_input;
use super::run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
use super::turn::CodexTurnTracker;
use super::CodexRuntimeState;

const CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT: Duration = Duration::from_secs(150);
const CODEX_MCP_THREAD_INIT_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const CODEX_TURN_INTERRUPT_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const CODEX_TURN_INTERRUPT_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub fn submit_codex_prompt(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    envelope: &PromptEnvelope,
) -> Result<(), DaemonError> {
    // A steering command can wait in the provider actor mailbox while the
    // original turn is cancelled or completes. Never reinterpret it as a new
    // turn once that target has gone away.
    if envelope.steering && state.active_turn_id.is_none() {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "turn/steer",
            message: "active Codex turn settled before steering delivery".to_string(),
        }
        .steer_not_submitted(true));
    }
    let client = codex_client_for_run(run, state.endpoint(), None)
        .map_err(|error| error.steer_not_submitted(envelope.steering))?;
    let client = if state.read_only_discovery_permissions() || run.read_only_discovery() {
        client.with_read_only_discovery_permissions()
    } else {
        client
    };
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let existing_thread = state.thread_ready() || state.pending_thread_id().is_some();
    // Durable steering settles only on an RPC acknowledgement, never a buffered error.
    if let Err(error) = ensure_codex_thread_ready(
        &client,
        run,
        state,
        cwd.as_deref(),
        model.as_deref(),
        hidden_context_for_provider(&envelope.hidden_system_context),
    ) {
        if envelope.steering {
            return Err(error.steer_not_submitted(true));
        }
        state.buffered_notifications.push(CodexNotification::Error {
            message: error.to_string(),
        });
        return Ok(());
    }
    // Every existing thread keeps its conversation and provider identity.
    // Fresh thread/start already carries this context; later refreshes use the
    // same documented developer-item seam as native/resumed threads.
    let thread_id = state.thread_id().to_string();
    if existing_thread {
        if let Some(context) = hidden_context_for_provider(&envelope.hidden_system_context) {
            if let Err(error) = client.thread_inject_hidden_context(
                &mut state.socket,
                &mut state.next_request_id,
                &thread_id,
                context,
                &mut state.buffered_notifications,
            ) {
                if envelope.steering {
                    return Err(error.steer_not_submitted(true));
                }
                state.buffered_notifications.push(CodexNotification::Error {
                    message: error.to_string(),
                });
                return Ok(());
            }
        }
    }
    let input = codex_input(&envelope.visible_user_prompt, &envelope.attachments);
    let active_steering_turn_id = envelope.steering.then(|| {
        state
            .active_turn_id
            .clone()
            .expect("steering requires an active turn")
    });
    let response_result = match active_steering_turn_id.as_deref() {
        Some(active_turn_id) => client.turn_steer(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            active_turn_id,
            input,
            &mut state.buffered_notifications,
        ),
        None => client.turn_start(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            cwd.as_deref(),
            model.as_deref(),
            effort.as_deref(),
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
            hidden_context_for_provider(&envelope.hidden_system_context),
            input,
            &mut state.buffered_notifications,
        ),
    };
    let response = match response_result {
        Ok(response) => response,
        Err(error) => {
            if envelope.steering {
                return Err(error);
            }
            state.buffered_notifications.push(CodexNotification::Error {
                message: error.to_string(),
            });
            return Ok(());
        }
    };
    note_codex_turn_start_response(
        &mut state.active_turn_id,
        &mut state.turn_tracker,
        &response,
        envelope.steering,
    );
    if active_steering_turn_id.is_none() {
        state.authoritative_backfill_gate.reset();
    }
    crate::logging::debug_with_fields(
        "daemon.provider.codex",
        "codex turn start response trace",
        json!({
            "provider_run_id": run.id(),
            "active_turn_id": state.active_turn_id,
            "response": response,
        }),
    );
    Ok(())
}

pub(super) fn note_codex_turn_start_response(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    response: &Value,
    steering: bool,
) {
    let preserve_active_turn = steering && active_turn_id.is_some();
    if let Some(turn_id) = codex_turn_id_from_start_response(response) {
        if !preserve_active_turn {
            *active_turn_id = Some(turn_id);
        }
    }
    if !preserve_active_turn {
        *turn_tracker = CodexTurnTracker::default();
    }
}

fn ensure_codex_thread_ready(
    client: &CodexClient,
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    cwd: Option<&str>,
    model: Option<&str>,
    developer_instructions: Option<&str>,
) -> Result<(), DaemonError> {
    if state.thread_ready() {
        return Ok(());
    }
    let deadline = Instant::now() + CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT;
    loop {
        let result = if let Some(thread_id) = state.pending_thread_id().map(str::to_string) {
            // No new turn has been submitted yet. Resume may replay old item
            // deltas and uncorrelated legacy abort/error events; they must not
            // enter the buffer later drained for the newly admitted prompt.
            // turn/start and turn/steer retain their own interleaved events.
            client.thread_resume(
                &mut state.socket,
                &mut state.next_request_id,
                &thread_id,
                cwd,
                model,
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
                developer_instructions,
                &mut Vec::new(),
            )
        } else {
            client.thread_start(
                &mut state.socket,
                &mut state.next_request_id,
                cwd,
                model,
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
                developer_instructions,
            )
        };
        match result {
            Ok(thread) => {
                state.mark_thread_ready(thread.thread.id);
                return Ok(());
            }
            Err(error) if is_codex_mcp_handshake_timeout(&error) && Instant::now() < deadline => {
                crate::logging::warn_with_fields(
                    "daemon.provider.codex",
                    "retrying codex thread init after MCP handshake timeout",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "error": error.to_string(),
                    }),
                );
                sleep(CODEX_MCP_THREAD_INIT_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

fn hidden_context_for_provider(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn is_codex_mcp_handshake_timeout(error: &DaemonError) -> bool {
    let DaemonError::ProviderProtocol {
        operation, message, ..
    } = error
    else {
        return false;
    };

    matches!(*operation, "thread/start" | "thread/resume")
        && message.contains("required MCP servers failed to initialize")
        && message.contains("timed out handshaking with MCP server")
}

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let thread_id = state.thread_id().to_string();
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    let deadline = Instant::now() + CODEX_TURN_INTERRUPT_RETRY_TIMEOUT;
    loop {
        match client.turn_interrupt(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            &turn_id,
            &mut state.buffered_notifications,
        ) {
            Ok(()) => {
                note_codex_turn_interrupt_accepted(
                    &mut state.active_turn_id,
                    &mut state.turn_tracker,
                    &mut state.buffered_notifications,
                );
                return Ok(());
            }
            Err(error) if codex_turn_interrupt_is_waiting_for_task_start(&error) => {
                if !state.ephemeral
                    && client
                        .thread_turns_list(
                            &mut state.socket,
                            &mut state.next_request_id,
                            &thread_id,
                            &mut state.buffered_notifications,
                        )
                        .is_ok_and(|response| codex_turn_is_terminal(&response, &turn_id))
                {
                    note_codex_turn_interrupt_accepted(
                        &mut state.active_turn_id,
                        &mut state.turn_tracker,
                        &mut state.buffered_notifications,
                    );
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(error);
                }
                sleep(CODEX_TURN_INTERRUPT_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

fn codex_turn_interrupt_is_waiting_for_task_start(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::ProviderProtocol {
            operation: "turn/interrupt",
            message,
            ..
        } if message.contains("no active turn to interrupt")
    )
}

fn codex_turn_is_terminal(response: &Value, turn_id: &str) -> bool {
    response
        .get("data")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        })
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "completed" | "failed" | "cancelled" | "canceled"))
}

fn note_codex_turn_interrupt_accepted(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    buffered_notifications: &mut Vec<CodexNotification>,
) {
    *active_turn_id = None;
    turn_tracker.reset_for_started();
    // These notifications were received before the interrupt acknowledgement and belong to the
    // cancelled turn. Carrying them into the next FIFO submit would project stale output onto the
    // promoted prompt, which the kernel has already made authoritative.
    buffered_notifications.clear();
}

pub(super) fn codex_turn_id_from_start_response(response: &Value) -> Option<String> {
    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| response.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod cancellation_tests;

#[cfg(test)]
mod turn_attribution_tests;

#[cfg(test)]
mod prompt_tests {
    use super::{
        codex_turn_interrupt_is_waiting_for_task_start, codex_turn_is_terminal,
        note_codex_turn_interrupt_accepted, CodexNotification, CodexTurnTracker,
    };
    use crate::error::DaemonError;
    use crate::prompt_assembly::{PromptEnvelope, PromptManifest};
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    use crate::session::PromptAttachment;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::net::TcpListener;
    use std::thread;
    use tokio_tungstenite::tungstenite::{accept, connect, Message};

    // MP-08/MP-10: native input and Chariox-origin input share this provider seam.
    #[test]
    fn native_codex_turns_keep_hidden_context_out_of_user_input() {
        assert_codex_hidden_context_continuity(false, false);
    }

    // MP-08/MP-10: managed initialization and resume must keep the first
    // conversation when a later prompt refreshes hidden runtime context.
    #[test]
    fn managed_codex_hidden_context_refresh_preserves_conversation() {
        assert_codex_hidden_context_continuity(true, false);
    }

    #[test]
    fn resumed_codex_hidden_context_refresh_preserves_conversation() {
        assert_codex_hidden_context_continuity(true, true);
    }

    fn assert_codex_hidden_context_continuity(managed: bool, resumed: bool) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept fixture");
            let mut socket = accept(stream).expect("upgrade fixture");
            let mut requests = Vec::new();
            if managed {
                let init = read_json_request(
                    &mut socket,
                    if resumed {
                        "thread/resume"
                    } else {
                        "thread/start"
                    },
                );
                assert!(init["params"]["developerInstructions"]
                    .as_str()
                    .unwrap()
                    .contains("FIRST"));
                if resumed {
                    assert_eq!(init["params"]["threadId"], "native-thread");
                }
                socket
                    .send(Message::Text(
                        json!({"id": init["id"], "result": {
                            "thread": {"id": "native-thread"}, "model": "gpt-6.1-sol"
                        }})
                        .to_string()
                        .into(),
                    ))
                    .expect("initialize thread");
            }
            for index in 0..2 {
                let context = if managed && !resumed && index == 0 {
                    // Fresh thread/start already supplied its developer context.
                    serde_json::Value::Null
                } else {
                    let context = read_json_request(&mut socket, "thread/inject_items");
                    socket
                        .send(Message::Text(
                            json!({"id": context["id"], "result": {}})
                                .to_string()
                                .into(),
                        ))
                        .expect("inject context");
                    context
                };
                let request = read_json_request(&mut socket, "turn/start");
                socket
                    .send(Message::Text(
                        json!({
                            "id": request["id"],
                            "result": {"turn": {"id": format!("turn-{index}")}}
                        })
                        .to_string()
                        .into(),
                    ))
                    .expect("admit turn");
                requests.push((context, request));
            }
            let rejected = read_json_request(&mut socket, "thread/inject_items");
            socket
                .send(Message::Text(
                    json!({
                        "id": rejected["id"],
                        "error": {"code": -32601, "message": "context injection unavailable"}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("reject context");
            // A rejected bridge must not fall back to a visible turn.
            assert!(!matches!(socket.read(), Ok(Message::Text(_))));
            requests
        });
        let endpoint = format!("ws://{address}");
        let (socket, _) = connect(&endpoint).expect("connect fixture");
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "default")
            .with_agent_id("agent");
        let run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "fixture".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let mut state = if managed {
            super::super::state::CodexRuntimeState::pending(
                endpoint,
                resumed.then(|| "native-thread".to_string()),
                socket,
                1,
            )
        } else {
            super::super::state::CodexRuntimeState::new(
                endpoint,
                "native-thread".to_string(),
                socket,
                1,
            )
        };
        for hidden in [
            "<runtime-instructions>FIRST</runtime-instructions>",
            "<native-permission-instructions>SECOND</native-permission-instructions>",
        ] {
            state.active_turn_id = None;
            super::submit_codex_prompt(
                &run,
                &mut state,
                &PromptEnvelope::new(
                    "visible prompt",
                    hidden,
                    vec![PromptAttachment::new(
                        "file:///tmp/native%20image.png",
                        "image/png",
                        None,
                    )],
                    PromptManifest::current(),
                ),
            )
            .expect("submit prompt");
        }
        state.active_turn_id = None;
        super::submit_codex_prompt(
            &run,
            &mut state,
            &PromptEnvelope::new(
                "must not submit",
                "hidden rejected",
                Vec::new(),
                PromptManifest::current(),
            ),
        )
        .expect("provider rejection is projected through notifications");
        assert!(state.active_turn_id.is_none());
        assert!(state.buffered_notifications.iter().any(|notification| matches!(
            notification, CodexNotification::Error { message } if message.contains("context injection unavailable")
        )));
        drop(state);
        let requests = server.join().expect("join fixture");
        for ((context, request), marker) in requests.iter().zip(["FIRST", "SECOND"]) {
            assert_eq!(request["params"]["threadId"], "native-thread");
            assert_eq!(
                request["params"]["input"],
                json!([
                    {"type": "text", "text": "visible prompt"},
                    {"type": "localImage", "path": "/tmp/native image.png"}
                ])
            );
            if context.is_null() {
                continue;
            }
            assert_eq!(context["params"]["threadId"], "native-thread");
            assert_eq!(context["params"]["items"][0]["role"], "developer");
            assert!(context["params"]["items"][0]["content"][0]["text"]
                .as_str()
                .expect("hidden channel")
                .contains(marker));
        }
    }

    #[test]
    fn late_steering_cannot_start_a_new_codex_turn() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept fixture client");
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(500)))
                .expect("bound fixture read");
            let mut socket = accept(stream).expect("upgrade fixture client");
            socket.read().ok()
        });
        let endpoint = format!("ws://{address}");
        let (socket, _) = connect(&endpoint).expect("connect fixture");
        let request = LaunchProviderRequest::new(
            "session-late-steer",
            "codex",
            "codex",
            "default",
            "default",
        )
        .with_agent_id("agent-late-steer");
        let run = RuntimeProviderRun::new(
            "provider-run-late-steer",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "codex-test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let mut state = super::super::state::CodexRuntimeState::new(
            endpoint,
            "thread-settled".to_string(),
            socket,
            1,
        );
        let error = super::submit_codex_prompt(
            &run,
            &mut state,
            &PromptEnvelope::new("late message", "", Vec::new(), PromptManifest::current())
                .with_steering(true),
        )
        .expect_err("steering after cancellation must fail closed");
        assert!(error.to_string().contains("active Codex turn settled"));
        assert!(state.active_turn_id.is_none());
        drop(state);
        assert!(server.join().expect("join fixture").is_none());
    }

    #[test]
    fn accepted_interrupt_releases_runtime_for_the_next_fifo_prompt() {
        let mut active_turn_id = Some("turn-cancelled".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        turn_tracker.note_tool_started("tool-still-active");
        let mut buffered_notifications = vec![CodexNotification::TurnCompleted {
            turn_id: "turn-cancelled".to_string(),
            status: "interrupted".to_string(),
            error_message: None,
            items: Vec::new(),
        }];

        note_codex_turn_interrupt_accepted(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut buffered_notifications,
        );

        assert_eq!(active_turn_id, None);
        assert_eq!(turn_tracker.active_tool_count(), 0);
        assert!(!turn_tracker.has_pending_terminal());
        assert!(buffered_notifications.is_empty());
    }

    #[test]
    fn abort_retries_only_the_codex_task_start_race() {
        assert!(codex_turn_interrupt_is_waiting_for_task_start(
            &DaemonError::ProviderProtocol {
                provider_run_id: "provider-run-codex".to_string(),
                operation: "turn/interrupt",
                message: "no active turn to interrupt".to_string(),
            }
        ));
        assert!(!codex_turn_interrupt_is_waiting_for_task_start(
            &DaemonError::ProviderProtocol {
                provider_run_id: "provider-run-codex".to_string(),
                operation: "turn/interrupt",
                message: "thread was not found".to_string(),
            }
        ));
    }

    #[test]
    fn terminal_turns_make_an_already_settled_interrupt_successful() {
        for status in ["completed", "failed", "cancelled", "canceled"] {
            assert!(codex_turn_is_terminal(
                &json!({
                    "data": [
                        {"id": "turn-other", "status": "completed"},
                        {"id": "turn-target", "status": status}
                    ]
                }),
                "turn-target"
            ));
        }
        assert!(!codex_turn_is_terminal(
            &json!({"data": [{"id": "turn-target", "status": "inProgress"}]}),
            "turn-target"
        ));
        assert!(!codex_turn_is_terminal(
            &json!({"data": [{"id": "turn-other", "status": "completed"}]}),
            "turn-target"
        ));
    }

    #[test]
    fn resumed_prompt_drains_large_response_and_preserves_admission_notification() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Codex websocket fixture");
        let address = listener
            .local_addr()
            .expect("resolve Codex websocket fixture");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept Codex websocket client");
            let mut socket = accept(stream).expect("upgrade Codex websocket fixture");

            let resume_request = read_json_request(&mut socket, "thread/resume");
            // Keep the response large enough to exercise tungstenite frame growth and the
            // kernel's read loop before the next turn can be admitted.
            let large_history = "resume-history".repeat(220_000);
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": resume_request["id"],
                        "result": {
                            "thread": {"id": "thread-reused"},
                            "model": "gpt-5.5",
                            "history": large_history
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send large resume response");

            let context = read_json_request(&mut socket, "thread/inject_items");
            assert_eq!(context["params"]["items"][0]["role"], "developer");
            assert_eq!(
                context["params"]["items"][0]["content"][0]["text"],
                "resumed hidden context"
            );
            socket
                .send(Message::Text(
                    json!({"id": context["id"], "result": {}})
                        .to_string()
                        .into(),
                ))
                .expect("inject resumed context");
            let turn_request = read_json_request(&mut socket, "turn/start");
            assert_eq!(
                turn_request["params"]["input"],
                json!([{"type": "text", "text": "continue after the failed prompt"}])
            );
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "thread/tokenUsage/updated",
                        "params": {
                            "threadId": "thread-reused",
                            "turnId": "turn-after-resume",
                            "tokenUsage": {
                                "total": {"totalTokens": 42},
                                "last": {"totalTokens": 7},
                                "modelContextWindow": 100
                            }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send interleaved current-turn notification");
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": turn_request["id"],
                        "result": {"turn": {"id": "turn-after-resume"}}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send turn admission response");
        });

        let endpoint = format!("ws://{address}");
        let (socket, _) = connect(&endpoint).expect("connect Codex websocket client");
        let request = LaunchProviderRequest::new(
            "session-resume-submit",
            "codex",
            "codex",
            "default",
            "default",
        )
        .with_agent_id("agent-resume-submit");
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex-test".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let run = RuntimeProviderRun::new("provider-run-resume-submit", &request, launch_result);
        let mut state = super::super::state::CodexRuntimeState::pending(
            endpoint,
            Some("thread-reused".to_string()),
            socket,
            1,
        );

        super::super::prompt::submit_codex_prompt(
            &run,
            &mut state,
            &PromptEnvelope::new(
                "continue after the failed prompt",
                "resumed hidden context",
                Vec::new(),
                PromptManifest::current(),
            ),
        )
        .expect("resumed prompt should admit the next turn");

        assert_eq!(state.thread_id(), "thread-reused");
        assert_eq!(state.active_turn_id.as_deref(), Some("turn-after-resume"));
        assert!(state.buffered_notifications.iter().any(|notification| {
            matches!(
                notification,
                CodexNotification::TokenUsageUpdated {
                    thread_id,
                    turn_id,
                    ..
                } if thread_id == "thread-reused" && turn_id == "turn-after-resume"
            )
        }));

        drop(state);
        server.join().expect("join Codex websocket fixture");
    }

    #[test]
    fn resumed_prompt_does_not_apply_previous_turn_events_to_new_turn() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Codex websocket fixture");
        let address = listener.local_addr().expect("resolve fixture address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept Codex websocket client");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("bound fixture reads");
            let mut socket = accept(stream).expect("upgrade Codex websocket fixture");
            let resume = read_json_request(&mut socket, "thread/resume");
            for notification in [
                json!({"method": "item/agentMessage/delta", "params": {
                    "itemId": "old-item", "delta": "previous turn output"
                }}),
                json!({"method": "codex/event/turn_aborted", "params": {
                    "msg": {"reason": "previous turn interrupted"}
                }}),
                json!({"method": "error", "params": {
                    "error": {"message": "previous turn failed"}
                }}),
            ] {
                socket
                    .send(Message::Text(notification.to_string().into()))
                    .expect("send old resume-phase notification");
            }
            socket
                .send(Message::Text(
                    json!({
                        "id": resume["id"],
                        "result": {"thread": {"id": "thread-reused"}, "model": "gpt-5.5"}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("acknowledge resume");
            let start = read_json_request(&mut socket, "turn/start");
            // New-turn output may precede the admission response and must survive.
            socket
                .send(Message::Text(
                    json!({
                        "method": "item/agentMessage/delta",
                        "params": {"itemId": "new-item", "delta": "current turn output"}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send current turn notification");
            socket
                .send(Message::Text(
                    json!({
                        "id": start["id"], "result": {"turn": {"id": "turn-new"}}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("acknowledge current turn");
            // Serve the runtime's authoritative recovery check, if requested.
            // Keep the socket open until the client has observed its poll result.
            while let Ok(Message::Text(text)) = socket.read() {
                let request: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(request["method"], "thread/turns/list");
                socket
                    .send(Message::Text(
                        json!({
                            "id": request["id"],
                            "result": {"data": [{"id": "turn-new", "status": "inProgress"}]}
                        })
                        .to_string()
                        .into(),
                    ))
                    .expect("return current authoritative turn");
            }
        });
        let endpoint = format!("ws://{address}");
        let (socket, _) = connect(&endpoint).expect("connect Codex websocket client");
        let request = LaunchProviderRequest::new(
            "session-resume-phase",
            "codex",
            "codex",
            "default",
            "default",
        )
        .with_agent_id("agent-resume-phase");
        let run = RuntimeProviderRun::new(
            "provider-run-resume-phase",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "codex-test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let mut state = super::super::state::CodexRuntimeState::pending(
            endpoint,
            Some("thread-reused".to_string()),
            socket,
            1,
        );
        super::submit_codex_prompt(
            &run,
            &mut state,
            &PromptEnvelope::new("continue", "", Vec::new(), PromptManifest::current()),
        )
        .expect("admit new prompt");
        let result = super::super::drain::drain_codex_events(&run, &mut state, None);
        drop(state);
        server.join().expect("join Codex websocket fixture");
        let result = result.expect("poll current turn");
        assert!(
            !result.prompt_completed,
            "old resume events settled the new turn: {result:?}"
        );
        assert_eq!(result.terminal_failure, None);
        assert!(result.completions.is_empty());
        assert!(result.notices.is_empty());
        assert_eq!(
            result
                .chunks
                .iter()
                .flat_map(|chunk| chunk.bytes.iter().copied())
                .collect::<Vec<_>>(),
            b"current turn output",
            "only current-turn output may be projected after resume"
        );
    }

    fn read_json_request(
        socket: &mut tokio_tungstenite::tungstenite::WebSocket<std::net::TcpStream>,
        expected_method: &str,
    ) -> serde_json::Value {
        let message = socket.read().expect("read Codex request");
        let Message::Text(text) = message else {
            panic!("expected text Codex request");
        };
        let request: serde_json::Value =
            serde_json::from_str(&text).expect("parse Codex request payload");
        assert_eq!(request["method"], expected_method);
        request
    }
}
