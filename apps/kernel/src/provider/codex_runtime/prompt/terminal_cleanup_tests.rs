//! MP-08 / MP-10: cleanup belongs to the provider thread, never an OS PID.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

#[test]
fn mp08_terminal_cleanup_retains_cancellation_on_error_and_cleans_idle_thread() {
    for active in [false, true] {
        for failed in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("ws://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut socket = accept(stream).unwrap();
                if active {
                    let request: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                    assert_eq!(request["method"], "turn/interrupt");
                    socket.send(Message::Text(json!({"id":request["id"],"result":{}}).to_string().into())).unwrap();
                }
                let request: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(request["method"], "thread/backgroundTerminals/clean");
                assert_eq!(request["params"], json!({"threadId":"owned-thread"}));
                let response = if failed {
                    json!({"id":request["id"],"error":{"code":-32601,"message":"unsupported cleanup"}})
                } else { json!({"id":request["id"],"result":{}}) };
                socket.send(Message::Text(response.to_string().into())).unwrap();
            });
            let (socket, _) = connect(&endpoint).unwrap();
            let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() else { unreachable!() };
            let close = stream.try_clone().unwrap();
            let mut state = CodexRuntimeState::new(endpoint, "owned-thread".into(), socket, 1);
            if active { state.active_turn_id = Some("active".into()); }
            let result = abort_codex_turn("owned-run", &mut state);
            close.shutdown(Shutdown::Both).unwrap();
            server.join().unwrap();
            assert_eq!(result.is_err(), failed);
            assert_eq!(state.active_turn_id.is_some(), active && failed);
        }
    }
}

#[test]
fn mp08_terminal_cleanup_ack_cannot_hide_successor_start() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut socket = accept(stream).unwrap();
        for (index, method) in ["turn/interrupt", "thread/backgroundTerminals/clean", "turn/interrupt", "thread/backgroundTerminals/clean"].iter().enumerate() {
            let request: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(request["method"], *method);
            if index == 0 { assert_eq!(request["params"]["turnId"], "submitted"); }
            if index == 2 { assert_eq!(request["params"]["turnId"], "successor"); }
            if index == 1 {
                socket.send(Message::Text(json!({"method":"turn/started","params":{"turn":{"id":"successor"}}}).to_string().into())).unwrap();
            }
            socket.send(Message::Text(json!({"id":request["id"],"result":{}}).to_string().into())).unwrap();
        }
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() else { unreachable!() };
    let close = stream.try_clone().unwrap();
    let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
    state.active_turn_id = Some("submitted".into());
    let result = abort_codex_turn("run", &mut state);
    close.shutdown(Shutdown::Both).unwrap();
    server.join().unwrap();
    result.unwrap();
    assert!(state.active_turn_id.is_none());
}
