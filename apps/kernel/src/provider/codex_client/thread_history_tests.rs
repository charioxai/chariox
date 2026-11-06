use super::*;
use crate::provider::test_codex_connect as connect;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use serde_json::json;
use std::{net::TcpListener, thread, time::Duration};
use tokio_tungstenite::tungstenite::{accept, Message};

fn turn_history_error(message: &str) -> DaemonError {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let message = message.to_string();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let request: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "thread/turns/list");
        assert_eq!(
            request["params"],
            json!({
                "threadId": "thread-current", "limit": 20,
                "itemsView": "summary", "sortDirection": "desc",
            })
        );
        socket
            .send(Message::Text(
                json!({
                    "id": request["id"], "error": {"code": -32600, "message": message}
                })
                .to_string()
                .into(),
            ))
            .unwrap();
        // Unsupported history must fail explicitly, never reload the whole rollout.
        if let Ok(Message::Text(request)) = socket.read() {
            panic!("unexpected history fallback: {}", request.len());
        }
    });
    let (mut socket, _) = connect(&endpoint).unwrap();
    let client = CodexClient::new("run-history", endpoint).unwrap();
    let result = client.thread_turns_list(&mut socket, &mut 1, "thread-current", &mut Vec::new());
    drop(socket);
    server.join().unwrap();
    result.unwrap_err()
}

#[test]
fn unsupported_paginated_history_does_not_reload_the_entire_thread() {
    let error = turn_history_error("list_turns is not supported yet");
    assert!(matches!(
        error,
        DaemonError::ProviderProtocol {
            operation: "thread/turns/list",
            ..
        }
    ));
}

#[test]
fn turn_history_does_not_mask_unrelated_provider_errors() {
    let error = turn_history_error("thread not found");
    assert!(matches!(
        error,
        DaemonError::ProviderProtocol {
            operation: "thread/turns/list",
            ..
        }
    ));
}

// MP-08 / MP-09 / MP-10 / MP-11: old provider history must stay in the app-server.
#[test]
fn resume_excludes_history_larger_than_the_websocket_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = accept(stream).unwrap();
        let request: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "thread/resume");
        let turns = if request["params"]["excludeTurns"] == true {
            json!([])
        } else {
            json!([{"id":"old-turn", "items":[{"text":"h".repeat(65 * 1024 * 1024)}]}])
        };
        let _ = socket.send(Message::Text(json!({"id":request["id"],"result":{"thread":{"id":"thread-large", "turns":turns},"model":"gpt-6-luna"}}).to_string().into()));
    });
    let (mut socket, _) = connect(&endpoint).unwrap();
    let client = CodexClient::new("run-large-resume", endpoint).unwrap();
    let result = client.thread_resume(
        &mut socket,
        &mut 1,
        "thread-large",
        None,
        Some("gpt-6-luna"),
        ProviderWriteAccessMode::Unrestricted,
        AgentExecutionMode::Build,
        AgentPermissionLevel::Yolo,
        None,
        &mut Vec::new(),
    );
    drop(socket);
    server.join().unwrap();
    assert_eq!(
        result
            .expect("resume must not hydrate old history")
            .thread
            .id,
        "thread-large"
    );
}
