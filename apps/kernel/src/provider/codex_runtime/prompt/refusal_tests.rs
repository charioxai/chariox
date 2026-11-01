//! A Codex app-server's own error answer to a turn request is the provider's
//! refusal; faults of the connection stay protocol faults.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use serde_json::json;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

use super::super::drain::drain_codex_events;
use super::super::state::CodexRuntimeState;
use super::submit_codex_prompt;
use crate::error::DaemonError;
use crate::prompt_assembly::{PromptEnvelope, PromptManifest};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

const REFUSAL: &str = "Selected model is at capacity. Please try a different model.";

fn run() -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "default")
        .with_agent_id("agent");
    RuntimeProviderRun::new(
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
    )
}

/// A Codex app-server that answers the first request after `initialize` with
/// `answer` (a JSON-RPC error body, or `None` to hang up).
fn codex_server(
    expected_method: &'static str,
    answer: Option<serde_json::Value>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind Codex fixture");
    let endpoint = format!("ws://{}", listener.local_addr().expect("fixture address"));
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept Codex client");
        let mut socket = accept(stream).expect("upgrade Codex fixture");
        let Message::Text(text) = socket.read().expect("read Codex request") else {
            panic!("expected a text request");
        };
        let request: serde_json::Value = serde_json::from_str(&text).expect("request payload");
        assert_eq!(request["method"], expected_method);
        match answer {
            Some(error) => {
                socket
                    .send(Message::Text(
                        json!({"id": request["id"], "error": error})
                            .to_string()
                            .into(),
                    ))
                    .expect("send Codex error");
                // Keep the connection until the kernel is done with it.
                while socket.read().is_ok() {}
            }
            None => drop(socket),
        }
    });
    (endpoint, server)
}

fn submit_to_codex(endpoint: &str, steering: bool) -> (Result<(), DaemonError>, CodexRuntimeState) {
    let (socket, _) = connect(endpoint).expect("connect Codex fixture");
    let mut state = CodexRuntimeState::new(endpoint.to_string(), "thread".to_string(), socket, 1);
    state.active_turn_id = steering.then(|| "turn-1".to_string());
    let envelope = PromptEnvelope::new("prompt", "", Vec::new(), PromptManifest::current())
        .with_steering(steering);
    let result = submit_codex_prompt(&run(), &mut state, &envelope);
    (result, state)
}

#[test]
fn a_codex_turn_start_error_is_the_providers_refusal() {
    let (endpoint, server) = codex_server(
        "turn/start",
        Some(json!({"code": -32000, "message": REFUSAL})),
    );
    let (result, mut state) = submit_to_codex(&endpoint, false);
    result.expect("a refused turn reaches the owner as a notification");
    let drained = drain_codex_events(&run(), &mut state, None).expect("drain the refusal");
    assert_eq!(drained.terminal_failure.as_deref(), Some(REFUSAL));
    drop(state);
    server.join().expect("join Codex fixture");
}

#[test]
fn a_codex_steer_error_is_the_providers_refusal() {
    let (endpoint, server) = codex_server(
        "turn/steer",
        Some(json!({"code": -32000, "message": REFUSAL})),
    );
    let (result, state) = submit_to_codex(&endpoint, true);
    let error = result.expect_err("the steer is rejected");
    assert!(
        matches!(&error, DaemonError::ProviderPromptSteerRejected { source }
            if matches!(source.as_ref(), DaemonError::ProviderRefusal { message } if message == REFUSAL)),
        "{error:?}"
    );
    drop(state);
    server.join().expect("join Codex fixture");
}

#[test]
fn a_codex_connection_fault_stays_a_protocol_fault() {
    let (endpoint, server) = codex_server("turn/steer", None);
    let (result, state) = submit_to_codex(&endpoint, true);
    let error = result.expect_err("the steer fails");
    assert!(
        matches!(
            error,
            DaemonError::ProviderProtocol {
                operation: "codex_read",
                ..
            }
        ),
        "{error:?}"
    );
    drop(state);
    server.join().expect("join Codex fixture");
}
