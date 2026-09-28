//! `connections.list` / `connections.action` (protocol 359) through a real
//! worker's SDK channel, against a fake event generator.
use super::*;
use crate::durable_state::app_connections::ConnectionGrantCommand;
use crate::runtime::app_connection_broker::AppConnectionBroker;
use chariox_app_runtime::worker_peer::{Broker, BrokerFuture, BrokerRequest};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::Arc;

struct Connections(AppConnectionBroker);
impl Broker for Connections {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}

/// A generator that accepts every action and records the requests it got.
/// With `answer` false it acts and then drops the connection unanswered.
fn fake_generator(answer: bool) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let server_received = received.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            let body = loop {
                let count = stream.read(&mut buffer).unwrap_or(0);
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= length || count == 0 {
                        break body.to_owned();
                    }
                }
            };
            let request: Value = serde_json::from_str(&body).unwrap();
            let response = json!({
                "action_id": request["action_id"],
                "idempotency_key": request["idempotency_key"],
                "accepted": true,
                "result": {"ts": "1.3"},
            })
            .to_string();
            server_received.lock().unwrap().push(request);
            if !answer {
                continue;
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (format!("http://{address}"), received)
}

#[test]
fn an_app_acts_only_through_granted_connections_with_declared_actions() {
    let fixture = Fixture::new(Mode::Ready);
    let (url, received) = fake_generator(true);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.event_generator_management_targets = std::collections::BTreeMap::from([(
        "dev.chariox.slack".to_string(),
        crate::config::EventGeneratorManagementTarget {
            url,
            token: "token".into(),
            expires_at_ms: None,
            owner_ids: None,
            owner_scoped: None,
        },
    )]);
    let daemon_id = config.daemon_id.clone();
    let event_config: crate::runtime::app_lifecycle::EventConfig = Default::default();
    assert!(event_config
        .set(crate::runtime::projection::DaemonConfigProjectionStore::new(config))
        .is_ok());
    let installation = fixture.catalog.installation_id().to_owned();
    let broker = AppConnectionBroker::new(
        fixture.store.clone(),
        "alice".into(),
        installation.clone(),
        vec![chariox_app_package::ConnectionAccess {
            generator: "dev.chariox.slack".into(),
            actions: vec!["notification.reply".into()],
        }],
        fixture.admission.clone(),
        event_config,
    );
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Connections(broker)));
        let reply = json!({"connectionId": "connection-1", "action": "notification.reply",
            "input": {"text": "On it"}, "context": {"channel_id": "C1", "message_ts": "1.2"}});
        // Not granted yet.
        peer.send("early", "connections.action", reply.clone())
            .await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "CONNECTION_NOT_GRANTED"
        );
        let store = fixture.store.clone();
        let owner_installation = installation.clone();
        tokio::task::spawn_blocking(move || {
            store.app_connection_grant(ConnectionGrantCommand::Grant {
                owner: "alice".into(),
                installation: owner_installation,
                generator_id: "dev.chariox.slack".into(),
                connection_id: "connection-1".into(),
                now_ms: 1,
            })
        })
        .await
        .unwrap()
        .unwrap();
        peer.send("list", "connections.list", json!({})).await;
        assert_eq!(
            peer.response().await.1.unwrap(),
            json!({"connections": [{"generatorId": "dev.chariox.slack",
                "connectionId": "connection-1", "actions": ["notification.reply"]}]})
        );
        // An action the manifest does not declare.
        peer.send(
            "delete",
            "connections.action",
            json!({"connectionId": "connection-1", "action": "slack.message.delete"}),
        )
        .await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "CAPABILITY_REQUIRED"
        );
        for (id, key) in [
            ("reply", None),
            ("again", None),
            ("keyed", Some("k")),
            ("retry", Some("k")),
        ] {
            let mut reply = reply.clone();
            if let Some(key) = key {
                reply["idempotencyKey"] = json!(key);
            }
            peer.send(id, "connections.action", reply).await;
            let answer = peer.response().await.1.unwrap();
            assert_eq!(answer["accepted"], true);
            assert_eq!(answer["result"], json!({"ts": "1.3"}));
        }
        peer.close().await;
    });
    let received = received.lock().unwrap();
    assert_eq!(
        received.len(),
        4,
        "only granted, declared actions reach the generator"
    );
    assert_eq!(
        received[0]["owner_id"],
        crate::runtime::event_catalog_control::event_connection_owner_id(&daemon_id, "alice")
    );
    assert_eq!(received[0]["action_id"], "notification.reply");
    assert_eq!(received[0]["context"]["channel_id"], "C1");
    assert!(received[0]["idempotency_key"]
        .as_str()
        .unwrap()
        .starts_with(&format!("app:{installation}:")));
    assert_ne!(
        received[0]["idempotency_key"], received[1]["idempotency_key"],
        "without a key, a repeated call is a new action"
    );
    assert_eq!(
        received[2]["idempotency_key"],
        format!("app:{installation}:k")
    );
    assert_eq!(
        received[2]["idempotency_key"], received[3]["idempotency_key"],
        "a retry with the App's key reaches the generator with the same key"
    );
}

#[test]
fn a_lost_generator_reply_is_uncertain_and_never_replayed() {
    let fixture = Fixture::new(Mode::Ready);
    let (url, received) = fake_generator(false);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.event_generator_management_targets = std::collections::BTreeMap::from([(
        "dev.chariox.slack".to_string(),
        crate::config::EventGeneratorManagementTarget {
            url,
            token: "token".into(),
            expires_at_ms: None,
            owner_ids: None,
            owner_scoped: None,
        },
    )]);
    let event_config: crate::runtime::app_lifecycle::EventConfig = Default::default();
    assert!(event_config
        .set(crate::runtime::projection::DaemonConfigProjectionStore::new(config))
        .is_ok());
    let installation = fixture.catalog.installation_id().to_owned();
    fixture
        .store
        .app_connection_grant(ConnectionGrantCommand::Grant {
            owner: "alice".into(),
            installation: installation.clone(),
            generator_id: "dev.chariox.slack".into(),
            connection_id: "connection-1".into(),
            now_ms: 1,
        })
        .unwrap();
    let broker = AppConnectionBroker::new(
        fixture.store.clone(),
        "alice".into(),
        installation,
        vec![chariox_app_package::ConnectionAccess {
            generator: "dev.chariox.slack".into(),
            actions: vec!["notification.reply".into()],
        }],
        fixture.admission.clone(),
        event_config,
    );
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Connections(broker)));
        let reply = json!({"connectionId": "connection-1", "action": "notification.reply",
            "input": {"text": "On it"}, "context": {"channel_id": "C1", "message_ts": "1.2"}});
        // Without the App's key a retry would be a second action.
        peer.send("unkeyed", "connections.action", reply.clone())
            .await;
        let failure = peer.response().await.1.unwrap_err();
        assert_eq!(failure.code, "APP_CONNECTION_OUTCOME_UNCERTAIN");
        assert_eq!(failure.retryable, Some(false));
        let mut keyed = reply;
        keyed["idempotencyKey"] = json!("k");
        peer.send("keyed", "connections.action", keyed).await;
        let failure = peer.response().await.1.unwrap_err();
        assert_eq!(failure.code, "APP_CONNECTION_OUTCOME_UNCERTAIN");
        assert_eq!(failure.retryable, Some(true));
        peer.close().await;
    });
    assert_eq!(
        received.lock().unwrap().len(),
        2,
        "the kernel never replays an action whose outcome is unknown"
    );
}
