use super::*;

// MP-08 / MP-10: the relay has no per-source connection limit, so connections
// that never speak must not delay a kernel registering from the same address.
#[tokio::test(flavor = "multi_thread")]
async fn stalled_connections_from_one_address_do_not_delay_a_kernel_registration() {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server.bind_listener().await.expect("relay listener binds");
    let addr = listener.local_addr().expect("listener has addr");
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server runs");
    });

    let mut stalled = Vec::new();
    for _ in 0..200 {
        stalled.push(TcpStream::connect(addr).await.expect("stalled peer connects"));
    }

    let (mut socket, _) = timeout(
        Duration::from_secs(2),
        connect_async(format!("ws://{addr}")),
    )
    .await
    .expect("the kernel handshake completes while the storm is stalled")
    .expect("kernel connects");
    let register = RelayEnvelope::DaemonRegister {
        registration: test_registration_with_token("daemon-1", "machine-1", "Linux", 1, "secret"),
    };
    socket
        .send(Message::Text(serde_json::to_string(&register).unwrap().into()))
        .await
        .expect("register frame sends");
    timeout(Duration::from_secs(2), async {
        while registry.read().await.daemon("daemon-1").is_none() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the kernel registers while the storm is stalled");

    drop(stalled);
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task joins");
}
