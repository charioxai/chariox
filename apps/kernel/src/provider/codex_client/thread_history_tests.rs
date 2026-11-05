use super::*;
use crate::provider::test_codex_connect as connect;
use serde_json::json;
use std::{net::TcpListener, thread, time::Duration};
use tokio_tungstenite::tungstenite::{accept, Message};

fn list_with_legacy_server(error: &str) -> Result<Value, DaemonError> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let error = error.to_string();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let request: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "thread/turns/list");
        socket
            .send(Message::Text(
                json!({"id":request["id"],"error":{"code":-32600,"message":error}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        if let Ok(message) = socket.read() {
            if let Ok(text) = message.to_text() {
                let request: Value = serde_json::from_str(text).unwrap();
                assert_eq!(request["method"], "thread/read");
                assert_eq!(
                    request["params"],
                    json!({"threadId":"thread-legacy","includeTurns":true})
                );
                socket.send(Message::Text(json!({"id":request["id"],"result":{"thread":{"id":"thread-legacy","turns":[{"id":"turn-1","status":"failed","error":{"message":"usage limit"},"items":[]}]}}}).to_string().into())).unwrap();
            }
        }
    });
    let (mut socket, _) = connect(&endpoint).unwrap();
    let client = CodexClient::new("run-history", endpoint).unwrap();
    let result = client.thread_turns_list(&mut socket, &mut 1, "thread-legacy", &mut Vec::new());
    drop(socket);
    server.join().unwrap();
    result
}

#[test]
fn legacy_codex_turn_history_preserves_terminal_failure() {
    let result = list_with_legacy_server("list_turns is not supported yet").unwrap();
    assert_eq!(result["data"][0]["id"], "turn-1");
    assert_eq!(result["data"][0]["status"], "failed");
    assert_eq!(result["data"][0]["error"]["message"], "usage limit");
}

#[test]
fn turn_history_does_not_mask_unrelated_provider_errors() {
    let error = list_with_legacy_server("thread not found").unwrap_err();
    assert!(matches!(
        error,
        DaemonError::ProviderProtocol {
            operation: "thread/turns/list",
            ..
        }
    ));
}
