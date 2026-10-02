//! MP-08/MP-10: resumed native sessions must follow kernel permission changes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::{submit_opencode_prompt, tests::test_run_with_endpoint, OpenCodeRuntimeState};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest, OpenCodeEventSubscription,
};

fn server(
    turns: usize,
    reject_permission: bool,
) -> (String, thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        let mut submitted = 0;
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("Content-Length: ")?.parse::<usize>().ok()
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            let (headers, body) = text.split_once("\r\n\r\n").unwrap();
            let request = headers.lines().next().unwrap().to_string();
            let body = serde_json::from_str(body).unwrap_or(Value::Null);
            let patch = request.starts_with("PATCH ");
            let post = request.starts_with("POST ");
            requests.push((request, body));
            let (status, payload) = if patch && reject_permission {
                ("500 Internal Server Error", "{}")
            } else if post {
                ("204 No Content", "")
            } else if patch {
                ("200 OK", "{}")
            } else {
                ("200 OK", "[]")
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
            if post {
                submitted += 1;
            }
            if submitted == turns || (patch && reject_permission) {
                break;
            }
        }
        requests
    });
    (endpoint, handle)
}

fn envelope() -> crate::prompt_assembly::PromptEnvelope {
    crate::prompt_assembly::PromptEnvelope::new(
        "run the native shell",
        "",
        Vec::new(),
        Default::default(),
    )
}

#[test]
fn resumed_session_permission_changes_before_next_native_prompt() {
    let (endpoint, received) = server(2, false);
    let mut run = test_run_with_endpoint(
        LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
            .with_permission_level(AgentPermissionLevel::Yolo),
        false,
        &endpoint,
    );
    let (_tx, rx) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        endpoint,
        "existing-session".into(),
        OpenCodeEventSubscription::for_tests(rx),
    );
    submit_opencode_prompt(&run, &mut state, &envelope()).unwrap();
    run.set_execution_config(AgentExecutionMode::Build, AgentPermissionLevel::Required);
    submit_opencode_prompt(&run, &mut state, &envelope()).unwrap();
    state.stop();
    let requests = received.join().unwrap();
    let updates = requests
        .iter()
        .filter(|(line, _)| line.starts_with("PATCH /session/existing-session "))
        .collect::<Vec<_>>();
    assert_eq!(
        updates.len(),
        2,
        "each native turn must sync its kernel-owned permission policy"
    );
    for (update, action) in updates.iter().zip(["allow", "ask"]) {
        let rules = update.1["permission"].as_array().unwrap();
        assert_eq!(
            rules.iter().find(|r| r["permission"] == "bash").unwrap()["action"],
            json!(action)
        );
    }
    for (index, (line, _)) in requests.iter().enumerate() {
        if line.starts_with("POST ") {
            assert!(requests[..index]
                .iter()
                .rev()
                .any(|(line, _)| line.starts_with("PATCH ")));
        }
    }
}

#[test]
fn native_prompt_is_not_submitted_when_permission_update_fails() {
    let (endpoint, received) = server(1, true);
    let run = test_run_with_endpoint(
        LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
            .with_permission_level(AgentPermissionLevel::Required),
        false,
        &endpoint,
    );
    let (_tx, rx) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        endpoint,
        "existing-session".into(),
        OpenCodeEventSubscription::for_tests(rx),
    );
    let result = submit_opencode_prompt(&run, &mut state, &envelope());
    state.stop();
    let requests = received.join().unwrap();
    assert!(
        result.is_err(),
        "permission sync errors must block provider execution"
    );
    assert!(!requests.iter().any(|(line, _)| line.starts_with("POST ")));
}
