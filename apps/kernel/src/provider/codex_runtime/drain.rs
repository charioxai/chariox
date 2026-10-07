use std::time::Duration;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderNativeInteractionBridge, RuntimeProviderRun};

use super::events::{apply_notification_with_manifest, backfill_completed_turn};
use super::run_config::codex_client_for_run;
use super::turn::maybe_finalize_terminal_signal;
use super::{CodexPollResult, CodexRuntimeState};

const CODEX_EVENT_DRAIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
pub(super) const CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS: usize = 64;
pub(super) const CODEX_EVENT_DRAIN_TIME_BUDGET: Duration = Duration::from_millis(10);
const CODEX_MANAGED_BACKFILL_QUIET_GRACE: Duration = Duration::from_millis(250);

pub fn drain_codex_events(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexPollResult, DaemonError> {
    if state.active_turn_id.is_none() {
        state.native_approval_origin = None;
    }
    let client = codex_client_for_run(run, state.endpoint(), native_interaction_bridge)?
        .with_native_approval_context(
            state.native_approval_origin.clone(),
            state.active_turn_id.clone(),
        );
    let client = if state.read_only_discovery_permissions() || run.read_only_discovery() {
        client.with_read_only_discovery_permissions()
    } else {
        client
    };
    let client = if state.ephemeral {
        client.with_metadata_only_discovery()
    } else {
        client
    };
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    // MP-08 / MP-10: a queued abort shares this actor with output polls.
    // Bound both buffered and live work; keep unread notifications in order.
    let deadline = std::time::Instant::now() + CODEX_EVENT_DRAIN_TIME_BUDGET;
    let mut buffered = std::mem::take(&mut state.buffered_notifications).into_iter();
    let mut drained_to_quiet = false;
    for _ in 0..CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let notification = if let Some(notification) = buffered.next() {
            notification
        } else if let Some(notification) =
            client.read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
        {
            notification
        } else {
            drained_to_quiet = true;
            break;
        };
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
    state.buffered_notifications.extend(buffered);
    if !drained_to_quiet
        && state.buffered_notifications.is_empty()
        && std::time::Instant::now() < deadline
    {
        drained_to_quiet = client
            .read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
            .map(|notification| state.buffered_notifications.push(notification))
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
    // MP-08 / MP-10 / MP-11: Ephemeral metadata threads have no durable turn list.
    // Settle only their native item/completion notifications.
    if authoritative_backfill_due && !state.ephemeral {
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
    fn mp08_buffered_output_poll_yields_and_preserves_notification_order() {
        use crate::provider::CodexNotification;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let (done_tx, done_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _socket = accept(stream).unwrap();
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let (socket, _) = connect(&endpoint).unwrap();
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "default");
        let run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: BTreeMap::new(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some(endpoint.clone()),
            },
        );
        let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
        state.ephemeral = true;
        state.active_turn_id = Some("turn".into());
        for index in 0..128 {
            state
                .buffered_notifications
                .push(CodexNotification::CommandExecutionOutputDelta {
                    item_id: "tool".into(),
                    delta: format!("{index},"),
                });
        }
        state
            .buffered_notifications
            .push(CodexNotification::TurnCompleted {
                turn_id: "turn".into(),
                status: "completed".into(),
                error_message: None,
                items: vec![],
            });
        let first = drain_codex_events(&run, &mut state, None).unwrap();
        assert!(!first.prompt_completed);
        assert!(
            state.buffered_notifications.len() >= 65,
            "MP-08 buffered output must yield to queued controls"
        );
        let mut completed = false;
        for _ in 0..128 {
            let poll = drain_codex_events(&run, &mut state, None).unwrap();
            completed |= poll.prompt_completed;
            if state.buffered_notifications.is_empty() {
                break;
            }
        }
        assert!(completed);
        assert!(state.buffered_notifications.is_empty());
        assert_eq!(
            state.tool_items["tool"].streamed_output,
            (0..128)
                .map(|index| format!("{index},"))
                .collect::<String>()
        );
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn mp08_mp10_mp11_ephemeral_metadata_turn_settles_without_durable_backfill() {
        use crate::provider::CodexNotification;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let (done_tx, done_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = accept(stream).unwrap();
            done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            assert!(
                socket.read().is_err(),
                "ephemeral thread requested durable backfill"
            );
        });
        let (socket, _) = connect(&endpoint).unwrap();
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "default");
        let run = RuntimeProviderRun::new(
            "ephemeral-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: BTreeMap::new(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some(endpoint.clone()),
            },
        );
        let mut state = CodexRuntimeState::new(endpoint, "ephemeral-thread".into(), socket, 1);
        state.ephemeral = true;
        state.set_read_only_discovery_permissions(true);
        state.active_turn_id = Some("turn".into());
        state.buffered_notifications = vec![
            CodexNotification::ItemCompleted {
                item: json!({"id":"item", "type":"agentMessage", "text":"{\n  \"schema_version\": 1\n}"}),
            },
            CodexNotification::TurnCompleted {
                turn_id: "turn".into(),
                status: "completed".into(),
                error_message: None,
                items: vec![],
            },
        ];
        let poll = drain_codex_events(&run, &mut state, None).unwrap();
        assert!(poll.prompt_completed);
        assert!(poll.terminal_failure.is_none());
        assert_eq!(
            poll.chunks
                .iter()
                .flat_map(|chunk| chunk.bytes.clone())
                .collect::<Vec<_>>(),
            b"{\n  \"schema_version\": 1\n}"
        );
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn read_only_discovery_policy_survives_event_drain_reconstruction_after_turn_start() {
        struct DiscoveryWorkspace(std::path::PathBuf);
        impl Drop for DiscoveryWorkspace {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let workspace = DiscoveryWorkspace(std::env::temp_dir().join(format!(
            "chariox-discovery-drain-{:032x}",
            rand::random::<u128>()
        )));
        std::fs::create_dir(&workspace.0).expect("create discovery workspace");
        std::fs::write(workspace.0.join("README.md"), b"project evidence")
            .expect("write project-local read fixture");
        let canonical_read = workspace.0.join("README.md").canonicalize().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Codex websocket fixture");
        let address = listener
            .local_addr()
            .expect("resolve Codex websocket fixture");
        let (turn_start_returned_tx, turn_start_returned_rx) = mpsc::channel();
        let (permission_sent_tx, permission_sent_rx) = mpsc::channel();
        let (response_received_tx, response_received_rx) = mpsc::channel();
        let (drain_finished_tx, drain_finished_rx) = mpsc::channel();
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
                .recv_timeout(Duration::from_secs(10))
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
            assert!(response["result"]["permissions"].get("network").is_none());
            assert_eq!(
                response["result"]["permissions"]["fileSystem"]["read"],
                json!([canonical_read])
            );
            assert!(response["result"]["permissions"]["fileSystem"]
                .get("write")
                .is_none());
            response_received_tx
                .send(())
                .expect("notify client that permission response was received");
            drain_finished_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("client must finish draining before the server closes");
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
                working_directory: Some(workspace.0.clone()),
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
        let mut state = CodexRuntimeState::new(endpoint, thread.thread.id, socket, next_request_id);
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
            .recv_timeout(Duration::from_secs(10))
            .expect("server should send the permission request");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let poll =
                drain_codex_events(&run, &mut state, None).expect("event drain should succeed");
            assert!(poll.terminal_failure.is_none());
            assert!(!poll.prompt_completed);
            match response_received_rx.try_recv() {
                Ok(()) => break,
                Err(mpsc::TryRecvError::Empty) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                result => {
                    panic!("server should receive the read-only permission response: {result:?}")
                }
            }
        }
        drain_finished_tx
            .send(())
            .expect("release the mock server after draining");
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
