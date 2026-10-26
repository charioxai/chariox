//! MP-08/MP-10: loopback generator control + protocol-3 AEDS stand-in.
//! Uses the SDK's production subscription store/filter; no live credentials.
use chariox_event_protocol::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::BTreeSet, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};
use tokio_tungstenite::tungstenite::Message;

pub(super) const GENERATOR: &str = "dev.chariox.dummy";
pub(super) struct Services {
    pub generator_url: String,
    pub aeds_url: String,
    pub subscriptions: chariox_aegs_sdk::AegsStore,
    pub commands: mpsc::Sender<DeliveryCommand>,
    stop: watch::Sender<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
pub(super) struct DeliveryCommand {
    pub deliveries: Vec<EventDeliveryEnvelope>,
    pub discard_ack: bool,
    pub result: oneshot::Sender<usize>,
}
impl Services {
    pub async fn new(root: &std::path::Path) -> Self {
        let subscriptions = chariox_aegs_sdk::AegsStore::open(root.join("aegs.sqlite")).unwrap();
        let generator = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let aeds = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let generator_url = format!("http://{}", generator.local_addr().unwrap());
        let aeds_url = format!("ws://{}", aeds.local_addr().unwrap());
        let (stop, stopped) = watch::channel(false);
        let mut generator_stop = stopped.clone();
        let store = subscriptions.clone();
        let generator_task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = tokio::select! {
                    _ = generator_stop.changed() => return,
                    accepted = generator.accept() => accepted.unwrap(),
                };
                let response = tokio::time::timeout(Duration::from_secs(5), async {
                    let mut bytes = Vec::new();
                    let mut chunk = [0; 4096];
                    let (headers, body) = loop {
                        let count = stream.read(&mut chunk).await.unwrap();
                        assert!(count > 0 && bytes.len() + count <= 64 * 1024);
                        bytes.extend_from_slice(&chunk[..count]);
                        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                            let headers = std::str::from_utf8(&bytes[..end]).unwrap().to_owned();
                            let length: usize = headers.lines().find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                            }).unwrap();
                            if bytes.len() >= end + 4 + length {
                                break (headers, serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length]).unwrap());
                            }
                        }
                    };
                    assert!(headers.lines().any(|v| v.eq_ignore_ascii_case("authorization: Bearer fixture-only")));
                    let path = headers.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    let response = match path {
                        "/v1/subscriptions/reconcile" => {
                            let request: AegsSubscriptionReconcileRequest = serde_json::from_value(body).unwrap();
                            assert_eq!(request.owner_id, "daemon-test");
                            let accepted_binding_ids = store.reconcile(&request.owner_id, &request.generator_id, &request.subscriptions).unwrap();
                            serde_json::to_value(AegsSubscriptionReconcileResponse { accepted_binding_ids, authoritative: true }).unwrap()
                        }
                        "/v1/connections/query" => {
                            assert_eq!(body["owner_id"], crate::runtime::event_catalog_control::event_connection_owner_id("daemon-test", "local"));
                            json!({"connections":[{"generator_id":GENERATOR,"connection_id":"fixture-connection","status":"ready","metadata":{},"updated_at_ms":1}],"next_cursor":null})
                        }
                        _ => panic!("unexpected fixture management operation"),
                    };
                    let body = serde_json::to_vec(&response).unwrap();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                    stream.write_all(&body).await.unwrap();
                }).await;
                response.expect("bounded generator request");
            }
        });
        let (commands, mut receiver) = mpsc::channel::<DeliveryCommand>(8);
        let mut aeds_stop = stopped;
        let aeds_task = tokio::spawn(async move {
            loop {
                let (stream, _) = tokio::select! {
                    _ = aeds_stop.changed() => return,
                    accepted = aeds.accept() => accepted.unwrap(),
                };
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let frame = socket.next().await.unwrap().unwrap();
                let KernelToAedsMessage::Hello {
                    protocol_version,
                    kernel_id,
                    environments,
                } = serde_json::from_str(frame.to_text().unwrap()).unwrap()
                else {
                    panic!("hello first")
                };
                assert_eq!(protocol_version, 3);
                assert_eq!(kernel_id, "daemon-test");
                assert_eq!(environments.len(), 1);
                assert_eq!(environments[0].environment_id, "evval-fixture");
                let bindings: BTreeSet<_> = environments[0]
                    .routes
                    .iter()
                    .filter(|r| r.active)
                    .map(|r| r.binding_id.clone())
                    .collect();
                send(
                    &mut socket,
                    AedsToKernelMessage::HelloAccepted {
                        protocol_version: 3,
                        heartbeat_interval_ms: 15_000,
                    },
                )
                .await;
                send(
                    &mut socket,
                    AedsToKernelMessage::RoutesReconciled {
                        accepted_binding_ids: bindings.iter().cloned().collect(),
                        conflicts: vec![],
                    },
                )
                .await;
                loop {
                    let command = tokio::select! {
                        _ = aeds_stop.changed() => return,
                        command = receiver.recv() => command.unwrap(),
                        message = socket.next() => {
                            if message.is_none() || message.as_ref().is_some_and(|m| m.is_err() || m.as_ref().unwrap().is_close()) { break; }
                            continue;
                        }
                    };
                    let mut expected = BTreeSet::new();
                    for delivery in command.deliveries {
                        assert!(
                            bindings.contains(&delivery.binding_id),
                            "delivery must have a claimed binding"
                        );
                        expected.insert(delivery.delivery_id.clone());
                        send(&mut socket, AedsToKernelMessage::Delivery { delivery }).await;
                    }
                    let mut acknowledgements = 0;
                    while !expected.is_empty() {
                        let frame =
                            tokio::time::timeout(Duration::from_millis(500), socket.next()).await;
                        let Ok(Some(Ok(frame))) = frame else { break };
                        if let Ok(KernelToAedsMessage::Ack { delivery_id }) =
                            serde_json::from_str(frame.to_text().unwrap())
                        {
                            assert!(expected.remove(&delivery_id));
                            acknowledgements += 1;
                        }
                    }
                    // Model loss between ACK reception and AEDS durable ACK commit.
                    // The test retains the same delivery for replay after kernel restart.
                    command.result.send(acknowledgements).unwrap();
                    if command.discard_ack {
                        break;
                    }
                }
            }
        });
        Self {
            generator_url,
            aeds_url,
            subscriptions,
            commands,
            stop,
            tasks: vec![generator_task, aeds_task],
        }
    }
    /// Fixture source publication: production AEGS reconciliation/filter first,
    /// then the stand-in's per-binding transport fan-out.
    pub async fn generate(
        &self,
        deliveries: Vec<EventDeliveryEnvelope>,
        metadata: &Value,
        discard_ack: bool,
    ) -> usize {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let subscriptions = loop {
            let claims = self
                .subscriptions
                .matching(GENERATOR, "dummy.test", "default")
                .unwrap();
            if !claims.is_empty() {
                break claims;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "AEGS reconciliation must precede publication"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let matching: BTreeSet<_> = subscriptions
            .into_iter()
            .filter(|s| chariox_aegs_sdk::metadata_matches_filter(metadata, &s.filter))
            .map(|s| s.binding_id)
            .collect();
        let deliveries: Vec<_> = deliveries
            .into_iter()
            .filter(|d| matching.contains(&d.binding_id))
            .collect();
        if deliveries.is_empty() {
            0
        } else {
            self.deliver(deliveries, discard_ack).await
        }
    }
    pub async fn deliver(
        &self,
        deliveries: Vec<EventDeliveryEnvelope>,
        discard_ack: bool,
    ) -> usize {
        let (result, received) = oneshot::channel();
        self.commands
            .send(DeliveryCommand {
                deliveries,
                discard_ack,
                result,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), received)
            .await
            .unwrap()
            .unwrap()
    }
    pub async fn stop(mut self) {
        self.stop.send(true).unwrap();
        for task in self.tasks.drain(..) {
            task.await.unwrap();
        }
    }
    /// Actual AEGS filter and distinct-interest publication semantics.
    pub fn publications(&self, metadata: &Value) -> Vec<String> {
        self.subscriptions
            .matching(GENERATOR, "dummy.test", "default")
            .unwrap()
            .into_iter()
            .filter(|s| chariox_aegs_sdk::metadata_matches_filter(metadata, &s.filter))
            .map(|s| s.event_interest_key)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn send(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    message: AedsToKernelMessage,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
}
