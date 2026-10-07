//! MP-08 / MP-10: continuous command output cannot starve cancellation.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, protocol::Role, Message, WebSocket};

#[test]
fn mp08_interrupt_backlogged_command_output_sends_before_timeout() {
    for delta_bytes in [16 * 1024, 256 * 1024] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel::<Instant>();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(6)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut socket = accept(stream).unwrap();
            let writer_stream = socket.get_ref().try_clone().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let writer_stop = stop.clone();
            let writer = thread::spawn(move || {
                let mut output = WebSocket::from_raw_socket(writer_stream, Role::Server, None);
                let delta = Message::Text(
                    json!({
                        "method":"item/commandExecution/outputDelta",
                        "params":{"itemId":"noisy-command","delta":"x".repeat(delta_bytes)}
                    })
                    .to_string()
                    .into(),
                );
                output.send(delta.clone()).unwrap();
                ready_tx.send(()).unwrap();
                let mut sent = 1;
                // The producer stops only when the reader receives interrupt
                // (or the client closes on failure), never after a fixed batch.
                while !writer_stop.load(Ordering::Acquire) {
                    if output.send(delta.clone()).is_err() {
                        break;
                    }
                    sent += 1;
                }
                sent
            });
            let started = start_rx.recv().unwrap();
            let request = socket.read().ok().and_then(|message| {
                message
                    .into_text()
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            });
            let elapsed = started.elapsed();
            stop.store(true, Ordering::Release);
            let sent = writer.join().unwrap();
            if let Some(request) = request.as_ref() {
                assert_eq!(request["method"], "turn/interrupt");
                assert_eq!(request["params"]["turnId"], "submitted");
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":{}}).to_string().into(),
                    ))
                    .unwrap();
            }
            let clean = socket.read().ok().and_then(|message| {
                message
                    .into_text()
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            });
            if let Some(clean) = clean.as_ref() {
                assert_eq!(clean["method"], "thread/backgroundTerminals/clean");
                assert_eq!(clean["params"], json!({"threadId":"thread"}));
                socket
                    .send(Message::Text(
                        json!({"id":clean["id"],"result":{}}).to_string().into(),
                    ))
                    .unwrap();
            }
            (request.is_some(), elapsed, sent, clean.is_some())
        });
        let (socket, _) = connect(&endpoint).unwrap();
        let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) =
            socket.get_ref()
        else {
            unreachable!()
        };
        let close = stream.try_clone().unwrap();
        ready_rx.recv().unwrap();
        // Let the producer fill the TCP receive queue before cancellation.
        thread::sleep(Duration::from_millis(100));
        let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
        state.active_turn_id = Some("submitted".into());
        start_tx.send(Instant::now()).unwrap();
        let result = abort_codex_turn("run", &mut state);
        close.shutdown(Shutdown::Both).unwrap();
        let (interrupted, elapsed, sent, cleaned) = server.join().unwrap();
        println!("MP-08 / MP-10 delta_bytes={delta_bytes} interrupt_received={interrupted} elapsed={elapsed:?} deltas_sent={sent} result={result:?}");
        assert!(
            interrupted,
            "MP-08 command output must not prevent the first interrupt"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "MP-08 first interrupt must arrive promptly: {elapsed:?}"
        );
        if delta_bytes == 16 * 1024 {
            assert!(
                sent > 64,
                "MP-08 fixture must exceed a normal polling batch"
            );
        } else {
            // Large frames exercise the time budget before reaching 64 reads.
            assert!(
                sent * delta_bytes >= 1024 * 1024,
                "MP-08 large-frame fixture must have backlogged output"
            );
        }
        assert!(
            cleaned,
            "MP-08 interrupt ACK must be followed by provider-owned terminal cleanup"
        );
        result.unwrap();
        assert!(state.active_turn_id.is_none());
    }
}
