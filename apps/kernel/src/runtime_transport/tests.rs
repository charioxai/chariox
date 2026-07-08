use super::*;

use tokio::sync::oneshot;
use tokio::time::{timeout, Instant as TokioInstant};
use tokio_tungstenite::connect_async;

#[test]
fn kernel_event_writer_coalesces_event_lane_with_stable_deadline() {
    let now = TokioInstant::now();
    let mut coalescer = EventWriteCoalescer::new(33);

    assert!(coalescer.push_event("event-1", now).is_none());
    assert_eq!(coalescer.ready_at(), Some(now + Duration::from_millis(33)));
    assert!(coalescer
        .push_event("event-2", now + Duration::from_millis(10))
        .is_none());
    assert_eq!(coalescer.ready_at(), Some(now + Duration::from_millis(33)));

    assert_eq!(coalescer.drain_ready(), vec!["event-1", "event-2"]);
    assert_eq!(coalescer.ready_at(), None);
}

#[test]
fn kernel_event_writer_can_disable_event_coalescing_for_tests() {
    let now = TokioInstant::now();
    let mut coalescer = EventWriteCoalescer::new(0);

    assert_eq!(coalescer.push_event("event-1", now), Some("event-1"));
    assert_eq!(coalescer.ready_at(), None);
    assert!(coalescer.drain_ready().is_empty());
}

#[tokio::test]
async fn kernel_websocket_replies_to_ping_frames() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(app, listener, async {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let (mut socket, _) = connect_async(format!("ws://{addr}"))
        .await
        .expect("client should connect");
    socket
        .send(Message::Ping(Vec::from("probe").into()))
        .await
        .expect("ping should send");

    let pong = timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(payload))) => break payload.to_vec(),
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("websocket read failed: {error}"),
                None => panic!("websocket closed before pong"),
            }
        }
    })
    .await
    .expect("pong should arrive");

    assert_eq!(pong, b"probe");

    let _ = socket.close(None).await;
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server should stop")
        .expect("server task should finish")
        .expect("server should exit cleanly");
}
