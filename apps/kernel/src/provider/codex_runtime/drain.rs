use std::time::Duration;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderNativeInteractionBridge, RuntimeProviderRun};

use super::events::{apply_notification_with_manifest, backfill_completed_turn};
use super::run_config::codex_client_for_run;
use super::turn::maybe_finalize_terminal_signal;
use super::{CodexPollResult, CodexRuntimeState};

const CODEX_EVENT_DRAIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
const CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS: usize = 64;
const CODEX_MANAGED_BACKFILL_QUIET_GRACE: Duration = Duration::from_millis(250);

pub fn drain_codex_events(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexPollResult, DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), native_interaction_bridge)?;
    let client = if state.read_only_discovery_permissions() || run.read_only_discovery() {
        client.with_read_only_discovery_permissions()
    } else {
        client
    };
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    for notification in std::mem::take(&mut state.buffered_notifications) {
        apply_notification_with_manifest(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
            run.remote_extension_manifest(),
        );
    }

    let mut drained_to_quiet = true;
    for _ in 0..CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS {
        let Some(notification) =
            client.read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
        else {
            break;
        };
        drained_to_quiet = false;
        apply_notification_with_manifest(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
            run.remote_extension_manifest(),
        );
    }
    if !drained_to_quiet {
        drained_to_quiet = client
            .read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
            .map(|notification| {
                state.buffered_notifications.push(notification);
            })
            .is_none();
    }
    // Reconcile once when a turn starts, then only when provider evidence asks
    // for completion recovery. This keeps long healthy managed turns from
    // repeatedly reloading their growing rollout. Pending terminal and legacy
    // signals do not require a quiet drain; quiet final-assistant or completed-
    // tool evidence can re-arm the same bounded gate. `backfill_completed_turn`
    // still requires an authoritative terminal record with settlement evidence.
    let completion_recovery_evidence = codex_turn_recovery_evidence(
        run.endpoint_mode(),
        state.active_turn_id.is_some(),
        &state.turn_tracker,
        drained_to_quiet,
    );
    let authoritative_backfill_due = state.authoritative_backfill_gate.is_due(
        state.active_turn_id.is_some(),
        completion_recovery_evidence,
        std::time::Instant::now(),
    );
    if authoritative_backfill_due {
        backfill_completed_turn(
            &client,
            state,
            run.remote_extension_manifest(),
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        )?;
        if prompt_completed {
            state.turn_tracker.clear_legacy_completion_hint();
            state.authoritative_backfill_gate.reset();
        }
    }
    if drained_to_quiet && !prompt_completed {
        maybe_finalize_terminal_signal(
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
    }

    Ok(CodexPollResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_usage,
    })
}

pub(super) fn codex_turn_recovery_evidence(
    _endpoint_mode: AgentEndpointMode,
    has_active_turn: bool,
    turn_tracker: &super::turn::CodexTurnTracker,
    drained_to_quiet: bool,
) -> Option<u64> {
    let recovery_requested = has_active_turn
        && (turn_tracker.has_pending_terminal()
            || turn_tracker.has_legacy_completion_hint()
            || (drained_to_quiet
                && (turn_tracker
                    .has_quiet_terminal_assistant_evidence(CODEX_MANAGED_BACKFILL_QUIET_GRACE)
                    || turn_tracker
                        .has_quiet_completed_tool_activity(CODEX_MANAGED_BACKFILL_QUIET_GRACE))));
    recovery_requested
        .then(|| turn_tracker.completion_recovery_version())
        .flatten()
}

#[cfg(test)]
pub(super) fn codex_turn_should_backfill(
    endpoint_mode: AgentEndpointMode,
    has_active_turn: bool,
    turn_tracker: &super::turn::CodexTurnTracker,
    drained_to_quiet: bool,
) -> bool {
    codex_turn_recovery_evidence(
        endpoint_mode,
        has_active_turn,
        turn_tracker,
        drained_to_quiet,
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio_tungstenite::tungstenite::{accept, connect, Message};

    use crate::mcp::CharioxMcpServerConfig;
    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        ProviderLaunchResult, ProviderWriteAccessMode, RuntimeMcpBinding, RuntimeProviderRun,
    };

    use super::super::state::CodexRuntimeState;
    use super::drain_codex_events;

    #[test]
    fn read_only_discovery_policy_survives_event_drain_reconstruction_after_turn_start() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Codex websocket fixture");
        let address = listener
            .local_addr()
            .expect("resolve Codex websocket fixture");
        let (turn_start_returned_tx, turn_start_returned_rx) = mpsc::channel();
        let (permission_sent_tx, permission_sent_rx) = mpsc::channel();
        let (response_received_tx, response_received_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept Codex websocket client");
            let mut socket = accept(stream).expect("upgrade Codex websocket fixture");

            let thread_start = read_json_request(&mut socket, "thread/start");
            assert_eq!(thread_start["params"]["config"]["mcp_servers"], json!({}));
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 41,
                        "method": "item/tool/call",
                        "params": {
                            "tool": "workspace_live_sync_write_artifact",
                            "arguments": {
                                "path": "/repo/output",
                                "content": "mutate"
                            }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send thread MCP tool request");
            let tool_response = socket.read().expect("read thread MCP tool response");
            let Message::Text(tool_response) = tool_response else {
                panic!("expected thread MCP tool response text frame");
            };
            let tool_response: Value =
                serde_json::from_str(&tool_response).expect("parse thread MCP tool response");
            assert_eq!(tool_response["id"], json!(41));
            assert_eq!(tool_response["result"]["success"], json!(false));
            assert_eq!(
                tool_response["result"]["contentItems"][0]["text"],
                "MCP tool calls are disabled during read-only discovery"
            );
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": thread_start["id"],
                        "result": {
                            "thread": {"id": "thread-discovery"},
                            "model": "gpt-5.5"
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send thread/start response");

            let turn_start = read_json_request(&mut socket, "turn/start");
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": turn_start["id"],
                        "result": {"turn": {"id": "turn-discovery"}}
                    })
                .to_string()
                .into(),
                ))
                .expect("send turn/start response");
            turn_start_returned_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("client should finish turn/start before permission request");
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 42,
                        "method": "item/permissions/requestApproval",
                        "params": {
                            "permissions": {
                                "network": true,
                                "fileSystem": {
                                    "read": ["README.md"],
                                    "write": ["MUTATE"]
                                }
                            }
                        }
                    })
                .to_string()
                .into(),
                ))
                .expect("send post-turn-start permission request");
            // `drain_codex_events` intentionally performs a bounded
            // nonblocking read. Give the loopback peer time to deliver the
            // frame before releasing the client-side barrier below.
            thread::sleep(Duration::from_millis(50));
            permission_sent_tx
                .send(())
                .expect("notify client that permission request was sent");

            let response = socket.read().expect("read permission response");
            let Message::Text(response) = response else {
                panic!("expected permission response text frame");
            };
            let response: Value =
                serde_json::from_str(&response).expect("parse permission response");
            assert_eq!(response["id"], json!(42));
            assert_eq!(response["result"]["scope"], "turn");
            assert_eq!(response["result"]["permissions"]["network"], true);
            assert_eq!(
                response["result"]["permissions"]["fileSystem"]["read"],
                json!(["README.md"])
            );
            assert!(response["result"]["permissions"]["fileSystem"]
                .get("write")
                .is_none());
            response_received_tx
                .send(())
                .expect("notify client that permission response was received");
        });

        let endpoint = format!("ws://{address}");
        let (mut socket, _) = connect(&endpoint).expect("connect Codex websocket client");
        let launch_mcp = CharioxMcpServerConfig::stdio(
            "mutating-tool",
            "project-mcp",
            vec!["serve".to_string()],
        );
        let mut request = LaunchProviderRequest::new(
            "session-discovery-drain",
            "codex",
            "codex",
            "default",
            "default",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ))
        .with_mcp_servers(vec![launch_mcp])
        .with_provider_config_override("mcp_servers.injected.command", json!("arbitrary-mcp"));
        request.write_access_mode = ProviderWriteAccessMode::WorkspaceLiveSyncTracked;
        let run = RuntimeProviderRun::new(
            "provider-run-discovery-drain",
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
                structured_endpoint: Some(endpoint.clone()),
            },
        );
        let client = super::super::run_config::codex_client_for_run(&run, endpoint.as_str(), None)
            .expect("client should construct")
            .with_read_only_discovery_permissions();
        let mut next_request_id = 1;
        let thread = client
            .thread_start(
                &mut socket,
                &mut next_request_id,
                None,
                None,
                ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
                AgentExecutionMode::Plan,
                AgentPermissionLevel::Required,
                None,
            )
            .expect("thread/start should succeed");
        let mut state = CodexRuntimeState::new(
            endpoint,
            thread.thread.id,
            socket,
            next_request_id,
        );
        state.set_read_only_discovery_permissions(true);
        let thread_id = state.thread_id().to_string();
        client
            .turn_start(
                &mut state.socket,
                &mut state.next_request_id,
                &thread_id,
                None,
                None,
                None,
                ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
                AgentExecutionMode::Plan,
                AgentPermissionLevel::Required,
                None,
                vec![json!({"type": "text", "text": "discover"})],
                &mut state.buffered_notifications,
            )
            .expect("turn/start should succeed");
        turn_start_returned_tx
            .send(())
            .expect("notify server that turn/start returned");
        permission_sent_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("server should send the permission request");

        let poll = drain_codex_events(&run, &mut state, None).expect("event drain should succeed");
        assert!(poll.terminal_failure.is_none());
        assert!(!poll.prompt_completed);
        response_received_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("server should receive the read-only permission response");
        drop(state);
        server.join().expect("join Codex websocket fixture");
    }

    fn read_json_request(
        socket: &mut tokio_tungstenite::tungstenite::WebSocket<std::net::TcpStream>,
        expected_method: &str,
    ) -> Value {
        let message = socket.read().expect("read Codex request");
        let Message::Text(text) = message else {
            panic!("expected Codex request text frame");
        };
        let request: Value = serde_json::from_str(&text).expect("parse Codex request");
        assert_eq!(request["method"], expected_method);
        request
    }
}
