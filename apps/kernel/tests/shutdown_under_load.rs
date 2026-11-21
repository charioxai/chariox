//! A clean kernel exit while Room subscribers still hold their connections.
#![cfg(unix)]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, request};

#[tokio::test]
async fn sigterm_exits_with_agents_and_live_event_subscriptions() {
    let root = std::env::var_os("CHARIOX_SHUTDOWN_DRILL_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let scratch = root.join(format!("shutdown-{:016x}", rand::random::<u64>()));
    let mut kernel = real_kernel::start(&scratch, &[], None);
    let mut subscriptions = Vec::new();
    for index in 0..3 {
        let mut socket = connect(&kernel).await;
        let session = create_session(&mut socket, &scratch).await;
        let spawned = request(
            &mut socket,
            json!({"SpawnAgent": {"session_id": session, "alias": format!("worker-{index}"),
                "provider": "dev-stub", "model": "terminal-echo-a", "no_subscribe": true}}),
        )
        .await
        .unwrap();
        assert!(spawned.get("AgentSpawned").is_some());
        let attachment = request(
            &mut socket,
            json!({"AttachToSession": {"session_id": session, "client_id": format!("subscriber-{index}"),
                "capability_level": "FullTerminal"}}),
        )
        .await
        .unwrap();
        let subscription = json!({"type": "subscribe", "request_id": "subscribe",
            "session_id": session, "attachment_id": attachment["SessionAttached"]["attachment"]["id"]});
        socket
            .send(Message::Text(subscription.to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["request_id"] == "subscribe" {
                        assert_eq!(frame["response"]["ok"], true, "{frame}");
                    }
                    if frame["event"]["event"] == "heartbeat" {
                        break;
                    }
                }
            }
        })
        .await
        .expect("an active subscriber receives events");
        subscriptions.push(socket);
    }

    let status = kernel.terminate_within(Duration::from_secs(10)).await;
    let log = std::fs::read_to_string(scratch.join("kernel.log")).unwrap();
    assert!(status.success(), "kernel exited with {status}");
    assert!(!log.contains("panicked"), "kernel panicked during shutdown");
    assert!(
        !log.contains("shutdown budget exhausted"),
        "kernel abandoned shutdown"
    );
    drop(subscriptions);
}
