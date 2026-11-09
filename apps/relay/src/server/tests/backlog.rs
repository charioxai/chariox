use super::*;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const REPLY_BYTES: usize = 8 * 1024;
// More replies than the client's socket buffers and its whole outgoing queue can hold.
const REPLIES: usize = 6_000;

async fn connect_client(url: &str, daemon_id: &str) -> Socket {
    let (mut client, _) = connect_async_with_retry(url)
        .await
        .expect("client should connect");
    client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(daemon_id.to_string()),
                    daemon_alias: None,
                },
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    match client.next().await {
        Some(Ok(Message::Text(text))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).unwrap(),
            RelayEnvelope::ClientConnected { .. }
        )),
        other => panic!("unexpected client connect response: {other:?}"),
    }
    client
}

/// A loopback relay with `daemon-a` registered on it.
async fn relay_with_kernel() -> (
    String,
    Socket,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut daemon, _) = connect_async_with_retry(&url).await.unwrap();
    daemon
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-a", "machine-1", "Linux", 10),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;
    (url, daemon, shutdown_tx, server_task)
}

fn client_request(index: usize) -> Message {
    Message::Text(
        serde_json::to_string(&RelayEnvelope::ClientRequest {
            request_id: format!("client-request-{index}"),
            target: ClientTarget {
                daemon_id: Some("daemon-a".to_string()),
                daemon_alias: None,
            },
            encrypted_request: EncryptedRelayPayload {
                sender_public_key: "client-public".to_string(),
                nonce: "nonce".to_string(),
                ciphertext: "ciphertext".to_string(),
            },
        })
        .unwrap()
        .into(),
    )
}

async fn next_daemon_request(daemon: &mut Socket) -> Option<String> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), daemon.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(RelayEnvelope::DaemonRequest {
                    relay_request_id, ..
                }) = serde_json::from_str::<RelayEnvelope>(&text)
                {
                    return Some(relay_request_id);
                }
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
            _ => return None,
        }
    }
}

fn daemon_reply(relay_request_id: String) -> Message {
    Message::Text(
        serde_json::to_string(&RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: Some(EncryptedRelayPayload {
                sender_public_key: "daemon-public".to_string(),
                nonce: "abcdefghijklmnop".to_string(),
                ciphertext: base64::engine::general_purpose::STANDARD
                    .encode(vec![0xfe; REPLY_BYTES]),
            }),
            error: None,
        })
        .unwrap()
        .into(),
    )
}

// MP-08 / MP-09 / MP-10 / MP-11 (relayflood): one client that stops reading
// while a kernel answers its bulk requests must not cost the kernel its relay
// connection, and with it every other client of that kernel.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_client_does_not_disconnect_the_kernel_that_answers_it() {
    let (url, mut daemon, shutdown_tx, server_task) = relay_with_kernel().await;

    // The stalled client sends its requests and never reads a reply.
    let (mut stalled_tx, stalled_rx) = connect_client(&url, "daemon-a").await.split();
    let sender = tokio::spawn(async move {
        for index in 0..REPLIES {
            stalled_tx.send(client_request(index)).await.unwrap();
        }
        stalled_tx
    });
    for _ in 0..REPLIES {
        let relay_request_id = next_daemon_request(&mut daemon)
            .await
            .expect("the kernel lost its relay connection while answering a stalled client");
        if daemon.send(daemon_reply(relay_request_id)).await.is_err() {
            panic!("the kernel lost its relay connection while answering a stalled client");
        }
    }
    let _stalled_tx = sender.await.unwrap();

    // Another client of the same kernel is still served.
    let mut other = connect_client(&url, "daemon-a").await;
    other.send(client_request(REPLIES)).await.unwrap();
    let relay_request_id = next_daemon_request(&mut daemon)
        .await
        .expect("the kernel must still receive requests from other clients");
    daemon.send(daemon_reply(relay_request_id)).await.unwrap();
    match tokio::time::timeout(Duration::from_secs(10), other.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).unwrap(),
            RelayEnvelope::ClientResponse { .. }
        )),
        other => panic!("the other client did not get its reply: {other:?}"),
    }
    drop(stalled_rx);
    let _ = shutdown_tx.send(());
    server_task.await.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11 (relayflood): a receiver that is too far behind to catch up is
// told to come back later (close 1013) instead of the relay silently dropping its replies.
#[tokio::test(flavor = "multi_thread")]
async fn a_backlogged_receiver_is_closed_and_counted() {
    let (url, mut daemon, shutdown_tx, server_task) = relay_with_kernel().await;
    let (mut stalled_tx, mut stalled_rx) = connect_client(&url, "daemon-a").await.split();
    let sender = tokio::spawn(async move {
        for index in 0..REPLIES {
            stalled_tx.send(client_request(index)).await.unwrap();
        }
        stalled_tx
    });
    for _ in 0..REPLIES {
        let relay_request_id = next_daemon_request(&mut daemon)
            .await
            .expect("the kernel lost its relay connection while answering a stalled client");
        daemon
            .send(daemon_reply(relay_request_id))
            .await
            .expect("the kernel lost its relay connection while answering a stalled client");
    }
    let _stalled_tx = sender.await.unwrap();

    // The client starts reading again: what it had queued, then the close.
    let close = loop {
        match tokio::time::timeout(Duration::from_secs(10), stalled_rx.next()).await {
            Ok(Some(Ok(Message::Close(frame)))) => break frame,
            Ok(Some(Ok(_))) => {}
            other => panic!("the stalled client was never told to come back later: {other:?}"),
        }
    };
    assert_eq!(
        close.map(|frame| u16::from(frame.code)),
        Some(1013),
        "a backlogged receiver is closed with 'try again later'"
    );
    let _ = shutdown_tx.send(());
    server_task.await.unwrap();
}

async fn live_machine_ids(url: &str) -> Vec<String> {
    let (mut client, _) = connect_async_with_retry(url).await.unwrap();
    client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientMetadataRequest {
                request_id: "machines".to_string(),
                auth_token: "secret".to_string(),
                query: RelayMetadataQuery::ListLiveMachines,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    match tokio::time::timeout(Duration::from_secs(10), client.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text).unwrap() {
            RelayEnvelope::ClientMetadataResponse {
                machines: Some(machines),
                ..
            } => machines.into_iter().map(|m| m.machine_id).collect(),
            other => panic!("unexpected metadata response: {other:?}"),
        },
        other => panic!("no metadata response: {other:?}"),
    }
}

// MP-08 / MP-09 / MP-10 / MP-11 (relayflood): a kernel renews its kernel list over the
// socket it already has instead of opening a new one per renewal. A metadata token the
// relay refuses on that socket is an answer, not the end of the kernel's connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_metadata_request_on_a_kernel_socket_keeps_the_kernel_connected() {
    let (url, mut daemon, shutdown_tx, server_task) = relay_with_kernel().await;
    assert_eq!(live_machine_ids(&url).await, vec!["machine-1".to_string()]);

    daemon
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientMetadataRequest {
                request_id: "kernel-list".to_string(),
                auth_token: "expired-metadata-token".to_string(),
                query: RelayMetadataQuery::ListLiveMachines,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    match tokio::time::timeout(Duration::from_secs(2), daemon.next()).await {
        Err(_) => {}
        Ok(Some(Ok(Message::Text(text)))) => assert!(
            matches!(
                serde_json::from_str::<RelayEnvelope>(&text).unwrap(),
                RelayEnvelope::ClientMetadataResponse { error: Some(_), .. }
            ),
            "the relay must answer a refused metadata request, not close the kernel: {text}"
        ),
        Ok(other) => {
            panic!("the relay dropped the kernel over a refused metadata request: {other:?}")
        }
    }
    assert_eq!(
        live_machine_ids(&url).await,
        vec!["machine-1".to_string()],
        "the kernel must stay connected"
    );
    let _ = shutdown_tx.send(());
    server_task.await.unwrap();
}
