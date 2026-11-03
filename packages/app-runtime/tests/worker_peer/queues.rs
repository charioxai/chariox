use super::*;

#[tokio::test]
async fn sixty_four_running_sixteen_queued_then_busy_and_cancel_waits_for_ack() {
    let (host, stream) = tokio::io::duplex(256 * 1024);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits::default(),
    )
    .unwrap();
    let mut worker = worker(stream);
    let mut calls = Vec::new();
    for _ in 0..80 {
        let slot = peer
            .reserve(BUDGET)
            .expect("64 running plus 16 waiting admitted");
        let message = Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: slot.id().into(),
            method: "tools.invoke".into(),
            params: json!({}),
            deadline_ms: slot.deadline_ms(),
            context: None,
        };
        calls.push(slot.submit(message).unwrap());
    }
    assert!(matches!(peer.reserve(BUDGET), Err(PeerError::Busy)));
    let mut running = Vec::new();
    for _ in 0..64 {
        running.push(receive(&mut worker).await);
    }
    assert!(timeout(Duration::from_millis(30), worker.receive(BUDGET))
        .await
        .is_err());
    drop(calls.remove(0));
    let cancelled = receive(&mut worker).await;
    assert!(matches!(cancelled, Message::Cancel { .. }));
    assert_eq!(id(&cancelled), id(&running[0]));
    assert!(
        matches!(peer.reserve(BUDGET), Err(PeerError::Busy)),
        "cancelled work retains total admission until its terminal reply"
    );
    assert!(
        timeout(Duration::from_millis(30), worker.receive(BUDGET))
            .await
            .is_err(),
        "a cancelled guest still owns its execution slot until it answers"
    );
    worker
        .send(&response(&id(&cancelled), Value::Null), BUDGET)
        .await
        .unwrap();
    let promoted = receive(&mut worker).await;
    assert!(matches!(promoted, Message::Request { .. }));
    assert_eq!(id(&promoted), "host-65");
    peer.close();
    drop(calls);
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn sixty_four_broker_handlers_then_bounded_queue_and_busy() {
    let (host, stream) = tokio::io::duplex(256 * 1024);
    let (started_tx, mut started) = mpsc::channel(80);
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let permits = release.clone();
    let handler = broker(move |request| {
        let started = started_tx.clone();
        let release = permits.clone();
        Box::pin(async move {
            started.send(request.id).await.unwrap();
            let _permit = release.acquire().await.unwrap();
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
    for index in 0..81 {
        worker
            .send(&request(&format!("b-{index}"), "state.get"), BUDGET)
            .await
            .unwrap();
    }
    for _ in 0..64 {
        timeout(BUDGET, started.recv()).await.unwrap().unwrap();
    }
    assert!(started.try_recv().is_err());
    let busy = receive(&mut worker).await;
    assert_eq!(id(&busy), "b-80");
    assert_eq!(error(busy), "BUSY");
    release.add_permits(1);
    for _ in 0..80 {
        assert_eq!(result(receive(&mut worker).await), Value::Null);
    }
    for index in 64..80 {
        assert_eq!(started.recv().await.unwrap(), format!("b-{index}"));
    }
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn waiting_calls_cancel_and_expire_without_entering_the_worker() {
    let (host, stream) = tokio::io::duplex(4096);
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        null_broker(),
        PeerLimits {
            pending_calls: 1,
            queued_calls: 2,
            ..PeerLimits::default()
        },
    )
    .unwrap();
    let submit = |budget| {
        let slot = peer.reserve(budget).unwrap();
        let message = Message::Request {
            version: WIRE_VERSION,
            generation: slot.generation().into(),
            id: slot.id().into(),
            method: "tools.invoke".into(),
            params: json!({}),
            deadline_ms: slot.deadline_ms(),
            context: None,
        };
        slot.submit(message).unwrap()
    };
    let first = submit(BUDGET);
    let mut worker = worker(stream);
    let running = receive(&mut worker).await;
    let cancelled = submit(BUDGET);
    let expired = submit(Duration::from_millis(30));
    drop(cancelled);
    assert_eq!(
        timeout(BUDGET, expired).await.unwrap().unwrap_err(),
        PeerError::Deadline
    );
    let next = submit(BUDGET);
    worker
        .send(&response(&id(&running), Value::Null), BUDGET)
        .await
        .unwrap();
    first.await.unwrap();
    let promoted = receive(&mut worker).await;
    assert_eq!(
        id(&promoted),
        "host-4",
        "neither cancelled nor timed-out queued work reaches the guest"
    );
    worker
        .send(&response(&id(&promoted), Value::Null), BUDGET)
        .await
        .unwrap();
    next.await.unwrap();
    peer.close();
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}

struct QueueProbe {
    started: mpsc::UnboundedSender<BrokerRequest>,
    stopped: std::sync::Mutex<Vec<(String, bool)>>,
    release: Arc<tokio::sync::Semaphore>,
}
impl Broker for QueueProbe {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        self.started.send(request).unwrap();
        let release = self.release.clone();
        Box::pin(async move {
            release.acquire().await.unwrap().forget();
            Ok(Value::Null)
        })
    }
    fn request_not_dispatched(&self, request: &BrokerRequest) {
        assert!(request.cancellation.is_cancelled());
        self.stopped.lock().unwrap().push((
            request.id.clone(),
            request.cancellation.cancelled_by_deadline(),
        ));
    }
}

#[tokio::test]
async fn broker_queue_cancellation_deadline_and_shutdown_never_start_effects() {
    let (host, stream) = tokio::io::duplex(4096);
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let probe = Arc::new(QueueProbe {
        started: started_tx,
        stopped: Default::default(),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
    });
    let (peer, mut events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        probe.clone(),
        PeerLimits {
            broker_handlers: 1,
            queued_broker_requests: 2,
            ..PeerLimits::default()
        },
    )
    .unwrap();
    let mut worker = worker(stream);
    worker
        .send(&request("running", "state.transaction"), BUDGET)
        .await
        .unwrap();
    let running = timeout(BUDGET, started.recv()).await.unwrap().unwrap();
    worker
        .send(&request("cancel", "state.transaction"), BUDGET)
        .await
        .unwrap();
    let mut expiring = request("expire", "state.transaction");
    if let Message::Request { deadline_ms, .. } = &mut expiring {
        *deadline_ms = deadline() - 30_000 + 30;
    }
    worker.send(&expiring, BUDGET).await.unwrap();
    worker
        .send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "cancel".into(),
            },
            BUDGET,
        )
        .await
        .unwrap();
    assert_eq!(error(receive(&mut worker).await), "CANCELLED");
    assert_eq!(error(receive(&mut worker).await), "DEADLINE_EXCEEDED");
    assert_eq!(
        *probe.stopped.lock().unwrap(),
        [("cancel".into(), false), ("expire".into(), true)]
    );
    worker
        .send(&request("shutdown", "state.transaction"), BUDGET)
        .await
        .unwrap();
    worker
        .send(
            &Message::Event {
                version: WIRE_VERSION,
                generation: "1".into(),
                name: "worker.barrier".into(),
                data: Value::Null,
            },
            BUDGET,
        )
        .await
        .unwrap();
    events.recv().await.unwrap();
    peer.close();
    peer.closed().await;
    // Teardown holds real callback ownership even though queued work is discarded.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(running.cancellation.is_cancelled());
    assert!(started.try_recv().is_err());
    assert_eq!(probe.stopped.lock().unwrap().last().unwrap().0, "shutdown");
    probe.release.add_permits(1);
    timeout(BUDGET, task.join()).await.unwrap().unwrap();
}
