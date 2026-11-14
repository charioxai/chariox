//! What a kernel with no client sends: nothing the relay and Cloud do not need to keep it listed.
//! Real sockets (fake relay and Cloud ends), not hosted acceptance.
use super::support::*;
use crate::transport::relay_client::connection_state::{
    publish_cloud_presence, CloudPresencePublishOutcome,
};
use futures_util::StreamExt;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio_tungstenite::tungstenite::Message;

const HEARTBEAT_TICK: Duration = Duration::from_millis(100);
const OBSERVED: Duration = Duration::from_millis(3000);

/// The 5 s tick drives local work; the relay only needs a frame to know the kernel is alive, and it
/// pings every 30 s on its own, so a bare heartbeat goes out once per six ticks.
#[test]
fn an_idle_kernel_sends_a_bare_heartbeat_once_per_six_ticks() {
    run_async_with_large_test_stack("idle-heartbeat-pace", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}", relay.local_addr().unwrap()));
        config.relay_token = Some("idle-heartbeat-fixture".into());
        config.relay_heartbeat_ms = HEARTBEAT_TICK.as_millis() as u64;
        let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector = tokio::spawn(run_daemon_relay_connector(app, state, shutdown_rx));

        let (stream, _) = timeout(Duration::from_secs(5), relay.accept())
            .await
            .expect("the kernel should connect")
            .unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut heartbeats = 0;
        let deadline = tokio::time::Instant::now() + OBSERVED;
        while let Ok(Some(Ok(message))) = tokio::time::timeout_at(deadline, socket.next()).await {
            if let Message::Text(text) = message {
                if matches!(
                    serde_json::from_str::<RelayEnvelope>(&text),
                    Ok(RelayEnvelope::DaemonHeartbeat { .. })
                ) {
                    heartbeats += 1;
                }
            }
        }

        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_secs(5), connector)
            .await
            .expect("connector should stop")
            .unwrap();
        // 3 s of 100 ms ticks is 30 ticks: five heartbeats at one per six ticks, not thirty.
        assert!(
            (3..=7).contains(&heartbeats),
            "an idle kernel sent {heartbeats} heartbeats in {OBSERVED:?}"
        );
    });
}

/// A Cloud that keeps its connections open: each request answered with a length and no close.
fn keep_alive_cloud() -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (counted, stopped) = (connections.clone(), stop.clone());
    std::thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            counted.fetch_add(1, Ordering::SeqCst);
            stream.set_nonblocking(false).unwrap();
            std::thread::spawn(move || {
                let mut buffered = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    while !buffered.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(read) => buffered.extend_from_slice(&chunk[..read]),
                        }
                    }
                    let head_end = buffered.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let head = String::from_utf8_lossy(&buffered[..head_end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while buffered.len() < head_end + length {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(read) => buffered.extend_from_slice(&chunk[..read]),
                        }
                    }
                    buffered.drain(..head_end + length);
                    let body = b"{}";
                    if write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                        body.len()
                    )
                    .and_then(|()| stream.write_all(body))
                    .is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    (api_url, connections, stop)
}

/// Every presence refresh used to open a new TCP and TLS connection to Cloud (about 4.6 KB on the
/// wire for a 568-byte request); the refreshes share one connection.
#[test]
fn presence_refreshes_share_one_cloud_connection() {
    run_async_with_large_test_stack("idle-presence-connection", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let (api_url, connections, stop) = keep_alive_cloud();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url,
            machine_id: Some("idle-presence-machine".into()),
            machine_credential: Some("synthetic-machine-credential".into()),
            ..Default::default()
        });
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap())),
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
        ));

        for round in 0..3 {
            let outcome = publish_cloud_presence(&router, true, "idle drill").await;
            assert_eq!(outcome, CloudPresencePublishOutcome::Published, "{round}");
        }

        stop.store(true, Ordering::SeqCst);
        assert_eq!(
            connections.load(Ordering::SeqCst),
            1,
            "three presence refreshes opened more than one Cloud connection"
        );
    });
}
