//! A Cloud that answers a presence heartbeat slowly (home Wi-Fi, dead IPv6
//! first) still counts as published; real socket regression, not hosted
//! acceptance.
use super::support::*;
use crate::transport::relay_client::connection_state::{
    publish_cloud_presence, CloudPresencePublishOutcome,
};
use std::io::{Read, Write};

const CLOUD_ANSWER_DELAY: Duration = Duration::from_millis(1500);

fn slow_presence_cloud() -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let cloud = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
        }
        std::thread::sleep(CLOUD_ANSWER_DELAY);
        let body = b"{}";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        String::from_utf8_lossy(&request)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    });
    (api_url, cloud)
}

#[test]
fn cloud_presence_publish_survives_a_slow_cloud_answer() {
    run_async_with_large_test_stack("cloud-presence-slow", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let (api_url, cloud) = slow_presence_cloud();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url,
            machine_id: Some("presence-machine".into()),
            machine_credential: Some("synthetic-machine-credential".into()),
            ..Default::default()
        });
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap())),
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
        ));

        let outcome = publish_cloud_presence(&router, true, "slow cloud drill").await;

        assert_eq!(outcome, CloudPresencePublishOutcome::Published);
        assert_eq!(cloud.join().unwrap(), "POST /kernels/presence HTTP/1.1");
    });
}

/// Cloud cannot be reached, so the expired relay token cannot be renewed: the
/// kernel must wait and retry the renewal, not present the dead token to the
/// relay every few seconds.
#[test]
fn expired_cloud_relay_token_is_not_presented_to_the_relay() {
    run_async_with_large_test_stack("cloud-expired-token", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let relay = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let relay_url = format!("ws://{}", relay.local_addr().unwrap());
        let unreachable_cloud = {
            let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", closed.local_addr().unwrap())
        };
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(relay_url.clone());
        config.relay_token = Some("expired-token".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: unreachable_cloud,
            relay_url,
            machine_id: Some("expired-machine".into()),
            machine_credential: Some("synthetic-machine-credential".into()),
            token_expires_at_ms: Some(crate::session::unix_epoch_ms() - 60_000),
            ..Default::default()
        });
        let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector = tokio::spawn(run_daemon_relay_connector(app, state, shutdown_rx));

        tokio::time::sleep(Duration::from_millis(2500)).await;

        assert!(
            relay.accept().is_err(),
            "the kernel opened a relay connection with an expired token"
        );
        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_secs(5), connector)
            .await
            .expect("connector should stop")
            .unwrap();
    });
}
