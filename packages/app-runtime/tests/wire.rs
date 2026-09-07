use chariox_app_runtime::wire::{Channel, Message, Sender, WireError, MAX_FRAME_BYTES};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{duplex, AsyncWriteExt};

fn request() -> Value {
    json!({"version":1,"generation":"generation-1","kind":"request",
        "id":"sdk-1","method":"state.get","params":{"key":"tasks"},
        "deadline_ms":1700000030000_u64})
}

async fn feed(raw: Value) -> Result<Message, WireError> {
    let data = serde_json::to_vec(&raw).unwrap();
    let (mut sender, receiver) = duplex(4096);
    sender.write_u32(data.len() as u32).await.unwrap();
    sender.write_all(&data).await.unwrap();
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    channel.receive(Duration::from_secs(1)).await
}

#[tokio::test]
async fn null_response_is_valid_but_ambiguous_response_is_not() {
    let mut value = json!({"version":1,"generation":"generation-1","kind":"response",
        "id":"host-1","result":null});
    assert!(feed(value.clone()).await.is_ok());
    value["error"] = json!({"code":"failed","message":"bad"});
    assert!(feed(value).await.is_err());
}

#[tokio::test]
async fn installation_generation_and_caller_come_from_supervisor() {
    assert!(feed(request()).await.is_ok());
    let mut stale = request();
    stale["generation"] = json!("previous-generation");
    assert!(matches!(feed(stale).await, Err(WireError::StaleGeneration)));
    let mut impersonated = request();
    impersonated["context"] = json!({"actor":"human","approved":true});
    assert!(feed(impersonated).await.is_err());
    let mut extra = request();
    extra["installation_id"] = json!("someone-else");
    assert!(feed(extra).await.is_err());
}

#[tokio::test]
async fn oversized_header_rejected_without_waiting_for_body() {
    let (mut sender, receiver) = duplex(16);
    sender
        .write_u32((MAX_FRAME_BYTES + 1) as u32)
        .await
        .unwrap();
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    assert!(matches!(
        channel.receive(Duration::from_secs(1)).await,
        Err(WireError::FrameLimit)
    ));
    assert!(channel.is_closed());
}

#[tokio::test(start_paused = true)]
async fn incomplete_frame_deadline_permanently_closes_channel() {
    let (mut sender, receiver) = duplex(16);
    sender.write_u32(100).await.unwrap();
    sender.write_all(b"{").await.unwrap();
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    assert!(matches!(
        channel.receive(Duration::from_secs(1)).await,
        Err(WireError::Timeout)
    ));
    assert!(matches!(
        channel.receive(Duration::from_secs(1)).await,
        Err(WireError::Closed)
    ));
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_after_partial_read_cannot_reuse_stream() {
    let (mut sender, receiver) = duplex(16);
    sender.write_u32(100).await.unwrap();
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    assert!(tokio::time::timeout(
        Duration::from_millis(1),
        channel.receive(Duration::from_secs(30))
    )
    .await
    .is_err());
    assert!(channel.is_closed());
}

#[tokio::test(start_paused = true)]
async fn blocked_write_has_backpressure_and_bounded_deadline() {
    let (sender, _receiver) = duplex(4);
    let mut channel = Channel::new(sender, "generation-1".into(), Sender::Supervisor).unwrap();
    let message: Message = serde_json::from_value(request()).unwrap();
    assert!(matches!(
        channel.send(&message, Duration::from_secs(1)).await,
        Err(WireError::Timeout)
    ));
    assert!(channel.is_closed());
}

#[tokio::test]
async fn fragmented_consecutive_frames_keep_boundaries() {
    let (mut sender, receiver) = duplex(16);
    let writer = tokio::spawn(async move {
        for id in ["sdk-1", "sdk-2"] {
            let mut value = request();
            value["id"] = json!(id);
            let bytes = serde_json::to_vec(&value).unwrap();
            let mut framed = (bytes.len() as u32).to_be_bytes().to_vec();
            framed.extend_from_slice(&bytes);
            for byte in framed {
                sender.write_all(&[byte]).await.unwrap();
            }
        }
    });
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    for expected in ["sdk-1", "sdk-2"] {
        let message = channel.receive(Duration::from_secs(1)).await.unwrap();
        match message {
            Message::Request { id, .. } => assert_eq!(id, expected),
            _ => panic!("expected request"),
        }
    }
    writer.await.unwrap();
}

#[tokio::test]
async fn invalid_utf8_and_unsupported_version_are_rejected() {
    let mut value = request();
    value["version"] = json!(2);
    assert!(feed(value).await.is_err());
    let (mut sender, receiver) = duplex(16);
    sender.write_u32(3).await.unwrap();
    sender.write_all(&[b'"', 255, b'"']).await.unwrap();
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    assert!(channel.receive(Duration::from_secs(1)).await.is_err());
    assert!(channel.is_closed());
}

#[tokio::test]
async fn supervisor_can_write_while_waiting_for_worker_traffic() {
    let (supervisor, worker) = duplex(4096);
    let channel = Channel::new(supervisor, "generation-1".into(), Sender::Worker).unwrap();
    let (mut reader, mut writer) = channel.split();
    let mut worker = Channel::new(worker, "generation-1".into(), Sender::Supervisor).unwrap();
    let peer = tokio::spawn(async move {
        let message = worker.receive(Duration::from_secs(1)).await.unwrap();
        worker.send(&message, Duration::from_secs(1)).await.unwrap();
    });
    let message: Message = serde_json::from_value(request()).unwrap();
    let (received, sent) = tokio::join!(
        reader.receive(Duration::from_secs(1)),
        writer.send(&message, Duration::from_secs(1))
    );
    assert!(sent.is_ok());
    assert_eq!(received.unwrap(), message);
    peer.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn writer_failure_wakes_an_already_blocked_reader() {
    let (supervisor, _worker) = duplex(4);
    let channel = Channel::new(supervisor, "generation-1".into(), Sender::Worker).unwrap();
    let (mut reader, mut writer) = channel.split();
    let message: Message = serde_json::from_value(request()).unwrap();
    let (received, sent) = tokio::join!(
        reader.receive(Duration::from_secs(30)),
        writer.send(&message, Duration::from_millis(1))
    );
    assert!(matches!(sent, Err(WireError::Timeout)));
    assert!(matches!(received, Err(WireError::Closed)));
}

#[tokio::test(start_paused = true)]
async fn healthy_idle_worker_can_exchange_frames_after_the_frame_budget() {
    let (supervisor, worker) = duplex(4096);
    let mut channel = Channel::new(supervisor, "generation-1".into(), Sender::Worker).unwrap();
    let mut worker = Channel::new(worker, "generation-1".into(), Sender::Supervisor).unwrap();
    let receiving = tokio::spawn(async move {
        let message = channel.receive(Duration::from_millis(10)).await.unwrap();
        channel
            .send(&message, Duration::from_millis(10))
            .await
            .unwrap();
        channel
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(!receiving.is_finished());
    let message: Message = serde_json::from_value(request()).unwrap();
    worker
        .send(&message, Duration::from_millis(10))
        .await
        .unwrap();
    assert_eq!(
        worker.receive(Duration::from_millis(10)).await.unwrap(),
        message
    );
    assert!(!receiving.await.unwrap().is_closed());
}

#[tokio::test(start_paused = true)]
async fn cancelling_an_idle_read_preserves_the_next_frame() {
    let (mut sender, receiver) = duplex(4096);
    let mut channel = Channel::new(receiver, "generation-1".into(), Sender::Worker).unwrap();
    assert!(tokio::time::timeout(
        Duration::from_secs(1),
        channel.receive(Duration::from_millis(10))
    )
    .await
    .is_err());
    assert!(!channel.is_closed());
    let bytes = serde_json::to_vec(&request()).unwrap();
    sender.write_u32(bytes.len() as u32).await.unwrap();
    sender.write_all(&bytes).await.unwrap();
    assert!(channel.receive(Duration::from_millis(10)).await.is_ok());
}
