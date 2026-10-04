//! Codex utility-prompt execution.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, ProviderUtilityExecutionPolicy,
    ProviderWriteAccessMode, RuntimeProviderRun,
};
use crate::terminal::TerminalOutputKind;

use super::input::codex_input;
use super::prompt::{abort_codex_turn, codex_turn_id_from_start_response};
use super::run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
use super::{drain_codex_events, CodexRuntimeState};

const CODEX_UTILITY_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn run_codex_utility_prompt(
    run: &RuntimeProviderRun,
    prompt: &str,
    hidden_system_context: &str,
    timeout: Duration,
    policy: ProviderUtilityExecutionPolicy,
) -> Result<String, DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_endpoint_missing",
            message: "codex utility requires a structured provider endpoint".to_string(),
        })?
        .to_string();
    let client = codex_client_for_run(run, &endpoint, None)?;
    let client = if policy.is_read_only_discovery() {
        client
            .with_write_access_mode(ProviderWriteAccessMode::WorkspaceLiveSyncTracked)
            .with_read_only_discovery_permissions()
    } else {
        client
    };
    let client = if policy.is_metadata_only() {
        client.with_metadata_only_discovery()
    } else {
        client
    };
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let (write_access_mode, execution_mode, permission_level) = if policy.is_read_only_discovery() {
        // Plan plus the tracked live-sync mode is Codex's enforced
        // read-only sandbox on every host platform, including macOS. Do
        // not inherit a provider run's ordinary build/yolo capability for
        // discovery.
        (
            ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
            AgentExecutionMode::Plan,
            AgentPermissionLevel::Required,
        )
    } else {
        (
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
        )
    };
    let thread = client.thread_start(
        &mut socket,
        &mut next_request_id,
        cwd.as_deref(),
        model.as_deref(),
        write_access_mode,
        execution_mode,
        permission_level,
        hidden_context_for_provider(hidden_system_context),
    )?;
    let mut state = CodexRuntimeState::new(endpoint, thread.thread.id, socket, next_request_id);
    if policy.is_read_only_discovery() {
        state.set_read_only_discovery_permissions(true);
    }
    state.ephemeral = policy.is_metadata_only();
    // MP-08 / MP-10 / MP-11: A fresh utility thread settles through its own
    // notifications; it never needs durable history reconciliation.
    state.notification_only = true;
    let input = codex_input(prompt, &[]);
    let thread_id = state.thread_id().to_string();
    let response = client.turn_start(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        cwd.as_deref(),
        model.as_deref(),
        effort.as_deref(),
        write_access_mode,
        execution_mode,
        permission_level,
        hidden_context_for_provider(hidden_system_context),
        input,
        &mut state.buffered_notifications,
    )?;
    if let Some(turn_id) = codex_turn_id_from_start_response(&response) {
        state.active_turn_id = Some(turn_id);
    }

    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut completed = false;
    while Instant::now() < deadline {
        let poll = drain_codex_events(run, &mut state, None)?;
        for chunk in poll.chunks {
            if chunk.kind == TerminalOutputKind::ProviderOutput {
                output.push_str(&String::from_utf8_lossy(&chunk.bytes));
            }
        }
        if let Some(failure) = poll.terminal_failure {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "codex_utility_failed",
                message: failure,
            });
        }
        if poll.prompt_completed {
            completed = true;
            break;
        }
        sleep(CODEX_UTILITY_POLL_INTERVAL);
    }
    if !completed {
        let _ = abort_codex_turn(run.id(), &mut state);
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_timeout",
            message: format!(
                "codex utility did not complete within {} ms",
                timeout.as_millis()
            ),
        });
    }
    let output = if policy.is_read_only_discovery() {
        output.trim().to_string()
    } else {
        clean_codex_utility_output(&output)
    };
    if output.is_empty() {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_empty_output",
            message: "codex utility returned no assistant text".to_string(),
        });
    }
    Ok(output)
}

fn hidden_context_for_provider(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn clean_codex_utility_output(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
    use serde_json::{json, Value};
    use std::{collections::BTreeMap, net::TcpListener, thread};
    use tokio_tungstenite::tungstenite::{accept, Message};

    #[test]
    fn mp08_mp10_mp11_fresh_read_only_utility_uses_notifications_and_preserves_json() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let expected = "{\n  \"schema_version\": 1,\n  \"validation_commands\": []\n}";
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket = accept(stream).unwrap();
            let mut methods = Vec::new();
            while let Ok(message) = socket.read() {
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                let method = request["method"].as_str().unwrap_or("");
                if method == "initialized" {
                    continue;
                }
                methods.push(method.to_string());
                let result = match method {
                    "initialize" => json!({"userAgent":"fixture"}),
                    "thread/start" => json!({"thread":{"id":"utility-thread"},"model":"fixture"}),
                    "turn/start" => json!({"turn":{"id":"utility-turn"}}),
                    // Fail explicitly like the official harness, rather than hanging.
                    "thread/turns/list" => {
                        socket.send(Message::Text(json!({"id": request["id"], "error":{"code":-32600,"message":"list_turns is not supported yet"}}).to_string().into())).unwrap();
                        break;
                    }
                    _ => panic!("unexpected utility method {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                if method == "turn/start" {
                    for notification in [
                        json!({"method":"item/completed","params":{"threadId":"utility-thread","turnId":"utility-turn","item":{"id":"output","type":"agentMessage","text":expected}}}),
                        json!({"method":"turn/completed","params":{"threadId":"utility-thread","turn":{"id":"utility-turn","status":"completed","items":[]}}}),
                    ] {
                        socket
                            .send(Message::Text(notification.to_string().into()))
                            .unwrap();
                    }
                }
            }
            methods
        });
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "fixture");
        let run = RuntimeProviderRun::new(
            "utility-run",
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
                structured_endpoint: Some(endpoint),
            },
        );
        let output = run_codex_utility_prompt(
            &run,
            "Generate definition",
            "",
            Duration::from_secs(3),
            ProviderUtilityExecutionPolicy::ReadOnlyDiscovery,
        );
        let methods = server.join().unwrap();
        assert_eq!(methods, ["initialize", "thread/start", "turn/start"]);
        assert_eq!(output.unwrap(), expected);
    }
}
