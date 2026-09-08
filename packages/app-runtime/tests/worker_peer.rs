use chariox_app_runtime::{
    wire::{Channel, Message, Outcome, Sender, Success, WIRE_VERSION},
    worker_peer::{Broker, BrokerFuture, BrokerRequest, PeerError, PeerLimits, WorkerPeer},
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{mpsc, oneshot, Mutex},
    time::timeout,
};

const BUDGET: Duration = Duration::from_secs(2);
struct Handler<F>(F);
impl<F> Broker for Handler<F>
where
    F: Fn(BrokerRequest) -> BrokerFuture + Send + Sync + 'static,
{
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        (self.0)(request)
    }
}
fn broker(f: impl Fn(BrokerRequest) -> BrokerFuture + Send + Sync + 'static) -> Arc<dyn Broker> {
    Arc::new(Handler(f))
}
fn null_broker() -> Arc<dyn Broker> {
    broker(|_| Box::pin(async { Ok(Value::Null) }))
}
fn worker(stream: DuplexStream) -> Channel<DuplexStream> {
    Channel::new(stream, "1".into(), Sender::Supervisor).unwrap()
}
async fn receive(worker: &mut Channel<DuplexStream>) -> Message {
    timeout(BUDGET, worker.receive(BUDGET))
        .await
        .unwrap()
        .unwrap()
}
fn deadline() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 30_000
}
fn request(id: &str, method: &str) -> Message {
    Message::Request {
        version: WIRE_VERSION,
        generation: "1".into(),
        id: id.into(),
        method: method.into(),
        params: json!({}),
        deadline_ms: deadline(),
        context: None,
    }
}
fn response(id: &str, result: Value) -> Message {
    Message::Response {
        version: WIRE_VERSION,
        generation: "1".into(),
        id: id.into(),
        outcome: Outcome::Success(Success { result }),
    }
}
fn id(message: &Message) -> String {
    match message {
        Message::Request { id, .. } | Message::Response { id, .. } | Message::Cancel { id, .. } => {
            id.clone()
        }
        _ => panic!("correlated message required"),
    }
}
fn result(message: Message) -> Value {
    match message {
        Message::Response {
            outcome: Outcome::Success(success),
            ..
        } => success.result,
        _ => panic!("successful response required"),
    }
}
fn error(message: Message) -> String {
    match message {
        Message::Response {
            outcome: Outcome::Failure(failure),
            ..
        } => failure.error.code,
        _ => panic!("error response required"),
    }
}

#[tokio::test]
async fn tool_call_can_await_broker_work_on_the_same_channel_and_control_events_are_separate() {
    let (host, stream) = tokio::io::duplex(64 * 1024);
    let (seen_tx, mut seen) = mpsc::channel(4);
    let handler = broker(move |request| {
        let seen = seen_tx.clone();
        Box::pin(async move {
            seen.send(request.method.clone()).await.unwrap();
            if request.method == "state.get" {
                Ok(json!({"value":"stored", "version":1}))
            } else {
                Err(chariox_app_runtime::wire::RemoteError {
                    code: "NOT_READY".into(),
                    message: "Trusted readiness policy has not accepted this worker".into(),
                    retryable: Some(false),
                })
            }
        })
    });
    let (peer, mut events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        handler,
        PeerLimits::default(),
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({"name":"echo","input":{}}), None));
    let mut worker = worker(stream);
    let tool = receive(&mut worker).await;
    worker
        .send(&request("sdk-state", "state.get"), BUDGET)
        .await
        .unwrap();
    assert_eq!(
        result(receive(&mut worker).await),
        json!({"value":"stored","version":1})
    );
    worker
        .send(
            &Message::Event {
                version: WIRE_VERSION,
                generation: "1".into(),
                name: "worker.progress".into(),
                data: json!({"phase":"working"}),
            },
            BUDGET,
        )
        .await
        .unwrap();
    assert_eq!(
        timeout(BUDGET, events.recv()).await.unwrap().unwrap().name,
        "worker.progress"
    );
    worker
        .send(&response(&id(&tool), json!({"text":"done"})), BUDGET)
        .await
        .unwrap();
    assert_eq!(
        result(timeout(BUDGET, call).await.unwrap().unwrap().unwrap()),
        json!({"text":"done"})
    );
    worker
        .send(&request("sdk-ready", "worker.ready"), BUDGET)
        .await
        .unwrap();
    assert_eq!(error(receive(&mut worker).await), "NOT_READY");
    assert_eq!(seen.recv().await.unwrap(), "state.get");
    assert_eq!(seen.recv().await.unwrap(), "worker.ready");
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn reservations_bound_admission_and_do_not_accept_forged_identity() {
    let (host, stream) = tokio::io::duplex(4096);
    let limits = PeerLimits {
        pending_calls: 1,
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        limits,
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    assert!(matches!(peer.reserve(BUDGET), Err(PeerError::Busy)));
    let mut forged = request("client-chosen", "tools.invoke");
    if let Message::Request { deadline_ms, .. } = &mut forged {
        *deadline_ms = slot.deadline_ms();
    }
    assert_eq!(slot.send(forged).await.unwrap_err(), PeerError::Invalid);
    let slot = peer.reserve(BUDGET).unwrap();
    assert_eq!(
        slot.request(
            "tools.invoke",
            json!({"unsafe":9_007_199_254_740_992_u64}),
            None
        )
        .await
        .unwrap_err(),
        PeerError::Invalid
    );
    let slot = peer.reserve(BUDGET).unwrap();
    drop(slot);
    let mut worker = worker(stream);
    assert!(timeout(Duration::from_millis(20), worker.receive(BUDGET))
        .await
        .is_err());
    assert!(!peer.is_closed());
    assert!(peer.reserve(Duration::from_secs(31)).is_err());
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn concurrent_calls_correlate_reversed_replies_under_a_one_frame_writer_queue() {
    let (host, stream) = tokio::io::duplex(4096);
    let limits = PeerLimits {
        pending_calls: 16,
        queued_frames: 1,
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        limits,
    )
    .unwrap();
    let mut calls = Vec::new();
    for index in 0..16 {
        let slot = peer.reserve(BUDGET).unwrap();
        calls.push(tokio::spawn(slot.request(
            "tools.invoke",
            json!({"index":index}),
            None,
        )));
    }
    let mut worker = worker(stream);
    let mut requests = Vec::new();
    for _ in 0..16 {
        requests.push(receive(&mut worker).await);
    }
    let names: std::collections::BTreeSet<_> = requests.iter().map(id).collect();
    assert_eq!(names.len(), 16);
    for request in requests.into_iter().rev() {
        let Message::Request { params, .. } = &request else {
            panic!("request")
        };
        worker
            .send(&response(&id(&request), params["index"].clone()), BUDGET)
            .await
            .unwrap();
    }
    for (index, call) in calls.into_iter().enumerate() {
        assert_eq!(
            result(timeout(BUDGET, call).await.unwrap().unwrap().unwrap()),
            json!(index)
        );
    }
    assert!(!peer.is_closed());
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn submit_enqueues_before_response_polling_and_preserves_correlation() {
    let (host, stream) = tokio::io::duplex(4096);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let mut message = request(slot.id(), "tools.invoke");
    if let Message::Request { deadline_ms, .. } = &mut message {
        *deadline_ms = slot.deadline_ms();
    }
    let response_future = slot.submit(message).unwrap();
    // No spawn or poll of the response future occurs before the worker sees
    // this request. A lifecycle guard can therefore end at successful submit.
    let mut worker = worker(stream);
    let sent = receive(&mut worker).await;
    worker
        .send(&response(&id(&sent), json!({"done":true})), BUDGET)
        .await
        .unwrap();
    assert_eq!(
        result(timeout(BUDGET, response_future).await.unwrap().unwrap()),
        json!({"done":true})
    );
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn dropping_unpolled_submitted_response_cancels_the_owned_request() {
    let (host, stream) = tokio::io::duplex(4096);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let mut message = request(slot.id(), "tools.invoke");
    if let Message::Request { deadline_ms, .. } = &mut message {
        *deadline_ms = slot.deadline_ms();
    }
    let response_future = slot.submit(message).unwrap();
    let mut worker = worker(stream);
    let sent = receive(&mut worker).await;
    drop(response_future);
    assert!(
        matches!(receive(&mut worker).await, Message::Cancel { id: cancelled, .. } if cancelled == id(&sent))
    );
    worker
        .send(&response(&id(&sent), Value::Null), BUDGET)
        .await
        .unwrap();
    assert!(!peer.is_closed());
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn closure_observer_wakes_on_remote_eof_and_remembers_closed_state() {
    let (host, stream) = tokio::io::duplex(4096);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let observed = peer.clone();
    let waiting = tokio::spawn(async move {
        observed.closed().await;
    });
    drop(stream);
    timeout(BUDGET, waiting).await.unwrap().unwrap();
    timeout(BUDGET, peer.closed()).await.unwrap();
    assert!(peer.is_closed());
    // Abrupt remote EOF is an I/O failure, even though every closure observer
    // must wake promptly and the already-closed notification remains readable.
    assert_eq!(
        timeout(BUDGET, task.join()).await.unwrap(),
        Err(PeerError::Io)
    );
}

#[tokio::test]
async fn timeout_and_caller_drop_send_cancel_and_late_unknown_replies_cannot_revive_calls() {
    let (host, stream) = tokio::io::duplex(4096);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let slot = peer.reserve(Duration::from_millis(30)).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({}), None));
    let mut worker = worker(stream);
    let original = receive(&mut worker).await;
    assert_eq!(
        timeout(BUDGET, call).await.unwrap().unwrap().unwrap_err(),
        PeerError::Deadline
    );
    assert!(
        matches!(receive(&mut worker).await,Message::Cancel{id:cancel_id,..} if cancel_id==id(&original))
    );
    worker
        .send(&response(&id(&original), json!("late")), BUDGET)
        .await
        .unwrap();
    worker
        .send(&response("host-999999", json!("unrequested")), BUDGET)
        .await
        .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({}), None));
    let original = receive(&mut worker).await;
    call.abort();
    let _ = call.await;
    assert!(
        matches!(receive(&mut worker).await,Message::Cancel{id:cancel_id,..} if cancel_id==id(&original))
    );
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({}), None));
    let original = receive(&mut worker).await;
    worker
        .send(&response(&id(&original), json!("current")), BUDGET)
        .await
        .unwrap();
    assert_eq!(
        result(timeout(BUDGET, call).await.unwrap().unwrap().unwrap()),
        json!("current")
    );
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn cancelled_inbound_handler_keeps_its_slot_until_it_actually_finishes() {
    let (host, stream) = tokio::io::duplex(4096);
    let (started_tx, mut started) = mpsc::channel(4);
    let (release_tx, release_rx) = oneshot::channel();
    let release = Arc::new(Mutex::new(Some(release_rx)));
    let handler = broker(move |request| {
        let started = started_tx.clone();
        let release = release.clone();
        Box::pin(async move {
            started.send(request.id.clone()).await.unwrap();
            if request.id == "first" {
                release.lock().await.take().unwrap().await.unwrap();
                assert!(request.cancellation.is_cancelled());
            }
            Ok(Value::Null)
        })
    });
    let limits = PeerLimits {
        broker_handlers: 1,
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        handler,
        limits,
    )
    .unwrap();
    let mut worker = worker(stream);
    worker
        .send(&request("first", "state.transaction"), BUDGET)
        .await
        .unwrap();
    assert_eq!(
        timeout(BUDGET, started.recv()).await.unwrap().unwrap(),
        "first"
    );
    worker
        .send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "first".into(),
            },
            BUDGET,
        )
        .await
        .unwrap();
    assert_eq!(error(receive(&mut worker).await), "CANCELLED");
    worker
        .send(&request("second", "state.transaction"), BUDGET)
        .await
        .unwrap();
    assert_eq!(error(receive(&mut worker).await), "BUSY");
    assert!(started.try_recv().is_err());
    release_tx.send(()).unwrap();
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    worker
        .send(&request("third", "state.transaction"), BUDGET)
        .await
        .unwrap();
    assert_eq!(result(receive(&mut worker).await), Value::Null);
    assert_eq!(started.recv().await.unwrap(), "third");
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn remote_deadline_is_clamped_and_shutdown_waits_for_actual_broker_completion() {
    let (host, stream) = tokio::io::duplex(4096);
    let (started_tx, mut started) = mpsc::channel(1);
    let (release_tx, release_rx) = oneshot::channel();
    let release = Arc::new(Mutex::new(Some(release_rx)));
    let handler = broker(move |request| {
        let started = started_tx.clone();
        let release = release.clone();
        Box::pin(async move {
            assert!(request.deadline <= tokio::time::Instant::now() + Duration::from_millis(30));
            started.send(()).await.unwrap();
            release.lock().await.take().unwrap().await.unwrap();
            assert!(request.cancellation.is_cancelled());
            Ok(Value::Null)
        })
    });
    let limits = PeerLimits {
        max_deadline: Duration::from_millis(30),
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        handler,
        limits,
    )
    .unwrap();
    let mut worker = worker(stream);
    worker
        .send(&request("long", "state.transaction"), BUDGET)
        .await
        .unwrap();
    timeout(BUDGET, started.recv()).await.unwrap().unwrap();
    assert_eq!(error(receive(&mut worker).await), "DEADLINE_EXCEEDED");
    peer.close();
    let mut joined = tokio::spawn(task.join());
    assert!(timeout(Duration::from_millis(20), &mut joined)
        .await
        .is_err());
    release_tx.send(()).unwrap();
    timeout(BUDGET, joined).await.unwrap().unwrap().unwrap();
}

async fn raw_send(stream: &mut DuplexStream, message: &Message) {
    let bytes = serde_json::to_vec(message).unwrap();
    stream.write_u32(bytes.len() as u32).await.unwrap();
    stream.write_all(&bytes).await.unwrap();
}
async fn raw_receive(stream: &mut DuplexStream) -> Message {
    let size = stream.read_u32().await.unwrap() as usize;
    assert!(size <= 1024 * 1024);
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await.unwrap();
    chariox_app_runtime::wire::decode(&bytes, "1", Sender::Supervisor).unwrap()
}

#[tokio::test]
async fn partial_incoming_frame_is_not_cancelled_by_an_outgoing_command() {
    let (host, mut stream) = tokio::io::duplex(4096);
    let (peer, mut events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let event = Message::Event {
        version: WIRE_VERSION,
        generation: "1".into(),
        name: "worker.progress".into(),
        data: json!({}),
    };
    let bytes = serde_json::to_vec(&event).unwrap();
    let header = (bytes.len() as u32).to_be_bytes();
    stream.write_all(&header[..2]).await.unwrap();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("lifecycle.dispatch", json!({}), None));
    let outbound = timeout(BUDGET, raw_receive(&mut stream)).await.unwrap();
    stream.write_all(&header[2..]).await.unwrap();
    stream.write_all(&bytes).await.unwrap();
    raw_send(&mut stream, &response(&id(&outbound), Value::Null)).await;
    assert_eq!(events.recv().await.unwrap().name, "worker.progress");
    assert_eq!(
        result(timeout(BUDGET, call).await.unwrap().unwrap().unwrap()),
        Value::Null
    );
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn partial_frame_timeout_closes_peer_and_settles_pending_call() {
    let (host, mut stream) = tokio::io::duplex(4096);
    let limits = PeerLimits {
        frame_timeout: Duration::from_millis(30),
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        limits,
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({}), None));
    timeout(BUDGET, raw_receive(&mut stream)).await.unwrap();
    stream.write_all(&[0, 0]).await.unwrap();
    assert_eq!(
        timeout(BUDGET, call).await.unwrap().unwrap().unwrap_err(),
        PeerError::Closed
    );
    assert_eq!(
        timeout(BUDGET, task.join()).await.unwrap().unwrap_err(),
        PeerError::Io
    );
    assert!(peer.is_closed());
    assert!(matches!(peer.reserve(BUDGET), Err(PeerError::Closed)));
}

#[tokio::test]
async fn occurrence_events_and_worker_context_spoofing_close_the_peer_without_broker_authority() {
    for spoof in [false, true] {
        let (host, mut stream) = tokio::io::duplex(4096);
        let handler =
            broker(|_| Box::pin(async { panic!("untrusted frame must not reach broker") }));
        let (peer, _events, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            handler,
            PeerLimits::default(),
        )
        .unwrap();
        let message = if spoof {
            let mut request = request("spoof", "state.get");
            if let Message::Request { context, .. } = &mut request {
                *context = Some(json!({"owner":"other"}));
            }
            request
        } else {
            Message::Event {
                version: WIRE_VERSION,
                generation: "1".into(),
                name: "events.emit".into(),
                data: json!({"automationId":"unapproved"}),
            }
        };
        raw_send(&mut stream, &message).await;
        assert!(timeout(BUDGET, task.join()).await.unwrap().is_err());
        assert!(peer.is_closed());
    }
}

#[tokio::test]
async fn duplicate_active_request_closes_peer_but_still_drains_the_original_callback() {
    let (host, stream) = tokio::io::duplex(4096);
    let (started_tx, started_rx) = oneshot::channel();
    let started = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release = Arc::new(Mutex::new(Some(release_rx)));
    let handler = broker(move |request| {
        let started = started.clone();
        let release = release.clone();
        Box::pin(async move {
            started
                .lock()
                .await
                .take()
                .expect("only one callback")
                .send(())
                .unwrap();
            release.lock().await.take().unwrap().await.unwrap();
            assert!(request.cancellation.is_cancelled());
            Ok(Value::Null)
        })
    });
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        handler,
        PeerLimits::default(),
    )
    .unwrap();
    let mut worker = worker(stream);
    worker
        .send(&request("same", "state.get"), BUDGET)
        .await
        .unwrap();
    timeout(BUDGET, started_rx).await.unwrap().unwrap();
    worker
        .send(&request("same", "state.get"), BUDGET)
        .await
        .unwrap();
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(peer.is_closed());
    let mut joined = tokio::spawn(task.join());
    assert!(timeout(Duration::from_millis(20), &mut joined)
        .await
        .is_err());
    release_tx.send(()).unwrap();
    assert_eq!(
        timeout(BUDGET, joined).await.unwrap().unwrap().unwrap_err(),
        PeerError::Protocol
    );
}

#[tokio::test]
async fn stalled_partial_write_closes_both_halves_and_releases_waiting_callers() {
    let (host, _unread_worker) = tokio::io::duplex(16);
    let limits = PeerLimits {
        frame_timeout: Duration::from_millis(30),
        ..PeerLimits::default()
    };
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        limits,
    )
    .unwrap();
    let slot = peer.reserve(BUDGET).unwrap();
    let call = tokio::spawn(slot.request("tools.invoke", json!({"text":"x".repeat(4096)}), None));
    assert_eq!(
        timeout(BUDGET, call).await.unwrap().unwrap().unwrap_err(),
        PeerError::Closed
    );
    assert_eq!(
        timeout(BUDGET, task.join()).await.unwrap().unwrap_err(),
        PeerError::Io
    );
    assert!(peer.is_closed());
}
