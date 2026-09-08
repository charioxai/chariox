use super::*;
use std::{net::TcpListener, thread};
use tokio_tungstenite::tungstenite::{accept, Message};

#[test]
fn refresh_deadline_bounds_stalled_handshake_initialize_and_reload_io() {
    for phase in ["handshake", "initialize", "reload"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept fixture: {error}"),
                }
            };
            // Accepted sockets inherit nonblocking mode on macOS. This
            // synchronous fixture uses explicit I/O deadlines on both hosts.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            if phase == "handshake" {
                // Receive the real request, never send upgrade headers, and
                // observe the timed-out client's actual socket close.
                let mut bytes = [0; 4096];
                assert!(stream.read(&mut bytes).unwrap() > 0);
                loop {
                    match stream.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => break,
                        Err(error) => panic!("stalled handshake socket was retained: {error}"),
                    }
                }
                return;
            }
            let mut socket = accept(stream).unwrap();
            let init: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(init["method"], "initialize");
            if phase == "reload" {
                socket
                    .send(Message::Text(
                        json!({"id":0,"result":{}}).to_string().into(),
                    ))
                    .unwrap();
                let initialized: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(initialized["method"], "initialized");
                let reload: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(reload["method"], "config/mcpServer/reload");
            }
            let error = socket.read().unwrap_err();
            if let tokio_tungstenite::tungstenite::Error::Io(error) = error {
                assert!(
                    !matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ),
                    "refresh retained its socket after deadline"
                );
            }
        });
        let started = Instant::now();
        let result = CodexClient::new("fixed-native-run", endpoint)
            .unwrap()
            .reload_mcp_servers_until(started + Duration::from_millis(150));
        assert!(result.is_err(), "stalled {phase} unexpectedly completed");
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "{phase} exceeded bounded slack"
        );
        server.join().unwrap();
    }
}

#[test]
fn refresh_does_not_resolve_external_endpoints_or_start_after_deadline() {
    for endpoint in [
        "ws://example.invalid",
        "wss://127.0.0.1:1",
        "ws://127.0.0.1:1/?token=secret",
    ] {
        assert!(CodexClient::new("fixed", endpoint)
            .unwrap()
            .reload_mcp_servers_until(Instant::now() + Duration::from_secs(1))
            .is_err());
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    assert!(CodexClient::new("fixed", endpoint)
        .unwrap()
        .reload_mcp_servers_until(Instant::now())
        .is_err());
    assert!(matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}
#[test]
fn native_mcp_reload_uses_only_the_official_hook_and_propagates_rejection() {
    for rejected in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "fixture client did not connect");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut socket = accept(stream).unwrap();
            let init: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(init["method"], "initialize");
            socket
                .send(Message::Text(
                    json!({"jsonrpc":"2.0","id":init["id"],"result":{}})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            let initialized: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(initialized["method"], "initialized");
            let request: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(request["method"], "config/mcpServer/reload");
            assert!(request.get("params").is_none());
            let reply = if rejected {
                json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"unsupported hook"}})
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"result":{}})
            };
            socket
                .send(Message::Text(reply.to_string().into()))
                .unwrap();
            // Refresh must not create a thread, submit a prompt or retarget
            // the terminal. This connection ends after the one hook.
            assert!(socket.read().is_err());
        });
        let result = CodexClient::new("unchanged-native-run", endpoint)
            .unwrap()
            .reload_mcp_servers_until(Instant::now() + Duration::from_secs(3));
        assert_eq!(result.is_err(), rejected);
        server.join().unwrap();
    }
}
