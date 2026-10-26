//! MP-08/MP-10/MP-11: a fresh assessment owns one turn and its terminal evidence.
use super::*;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    net::{Shutdown, TcpListener},
    thread,
};
use tokio_tungstenite::tungstenite::{accept, Message};

#[test]
fn amfix_r13_utility_terminal_result_survives_connection_reset() {
    let result = reset_after_utility_output(Some(("utility-turn", "completed")));
    assert_eq!(
        result.unwrap(),
        "{\"status\":\"achieved\",\"final_answer\":\"Verified\",\"evidence\":[\"public-result\"]}"
    );
}

#[test]
fn amfix_r13_utility_failed_terminal_survives_connection_reset_as_failure() {
    let error = reset_after_utility_output(Some(("utility-turn", "failed"))).unwrap_err();
    assert!(
        matches!(
            error,
            DaemonError::ProviderProtocol {
                operation: "codex_utility_failed",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn amfix_r13_utility_reset_without_matching_terminal_never_succeeds() {
    for terminal in [None, Some(("foreign-turn", "completed"))] {
        assert!(
            reset_after_utility_output(terminal).is_err(),
            "a result requires its exact turn completion"
        );
    }
}

fn reset_after_utility_output(
    terminal: Option<(&'static str, &'static str)>,
) -> Result<String, DaemonError> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        loop {
            let request: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            let method = request["method"].as_str().unwrap();
            if method == "initialized" {
                continue;
            }
            let result = match method {
                "initialize" => json!({}),
                "thread/start" => json!({"thread":{"id":"utility-thread"},"model":"fixture"}),
                "turn/start" => json!({"turn":{"id":"utility-turn"}}),
                _ => panic!("unexpected utility request {method}"),
            };
            socket
                .send(Message::Text(
                    json!({"id":request["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            if method != "turn/start" {
                continue;
            }
            socket.send(Message::Text(json!({"method":"item/completed","params":{"threadId":"utility-thread","turnId":"utility-turn","item":{"id":"answer","type":"agentMessage","text":"{\"status\":\"achieved\",\"final_answer\":\"Verified\",\"evidence\":[\"public-result\"]}"}}}).to_string().into())).unwrap();
            if let Some((id, status)) = terminal {
                socket.send(Message::Text(json!({"method":"turn/completed","params":{"threadId":"utility-thread","turn":{"id":id,"status":status,"items":[],"error":{"message":"bounded failure"}}}}).to_string().into())).unwrap();
            }
            // Actual TCP closure, without a WebSocket close handshake, after
            // buffered output and (only in the positive case) exact completion.
            socket.get_mut().shutdown(Shutdown::Both).unwrap();
            break;
        }
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
    let result = run_codex_utility_prompt(
        &run,
        "Assess public evidence",
        "",
        Duration::from_secs(3),
        ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
    );
    server.join().unwrap();
    result
}
