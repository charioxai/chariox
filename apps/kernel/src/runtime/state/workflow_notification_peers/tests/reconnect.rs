//! MP-08 / MP-10 / MP-11: a detach the source kernel could not answer is reconciled when the
//! source reconnects.
use super::*;

#[test]
fn a_source_kernel_that_reconnects_drops_the_follow_a_detach_could_not_reach() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(32 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(reconnect_drill())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn connect(
    router: &Arc<CommandRouter>,
    runtime: &KernelRuntimeState,
    config: &DaemonConfig,
    url: &str,
) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let (stop, stopped) = watch::channel(false);
    let connector = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            router.clone(),
            runtime.owned.relay_state.clone(),
            stopped,
            url.into(),
            config.relay_token.clone().unwrap(),
        ),
    );
    (stop, connector)
}

async fn both_connected(a: &KernelRuntimeState, b: &KernelRuntimeState) {
    for _ in 0..100 {
        if a.owned.relay_state.read().await.connected()
            && b.owned.relay_state.read().await.connected()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("MP-08 / MP-10: fixture kernels did not connect");
}

async fn reconnect_drill() {
    let secret = format!("fixture-{:032x}", rand::random::<u128>());
    let server = Arc::new(RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(BTreeMap::from([("fixture".into(), secret.clone())]), None),
    ));
    let listener = server.bind_listener().await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let registry = server.registry();
    let (server_tx, server_rx) = oneshot::channel();
    let server_copy = server.clone();
    let server_task = tokio::spawn(async move {
        server_copy
            .run_listener_until(listener, async {
                let _ = server_rx.await;
            })
            .await
            .unwrap();
    });
    let mut source = Fixture::for_kernel("wfreconnect-source");
    let (a, _, _) = source.workflow("reviewer");
    let mut target = Fixture::for_kernel("wfreconnect-target");
    let (_, _, bp) = target.workflow("subscriber");
    let (sr, source_runtime, mut sc) = peer(&mut source, &url, &secret);
    let (tr, target_runtime, mut tc) = peer(&mut target, &url, &secret);
    sc.relay_token = Some(token(&sc, &secret, "local"));
    tc.relay_token = Some(token(&tc, &secret, "local"));
    source_runtime.owned.config_projection.update(sc.clone());
    target_runtime.owned.config_projection.update(tc.clone());
    let (source_stop, source_connector) = connect(&sr, &source_runtime, &sc, &url).await;
    let (target_stop, target_connector) = connect(&tr, &target_runtime, &tc, &url).await;
    registered(&registry, &sc.daemon_id).await;
    registered(&registry, &tc.daemon_id).await;
    both_connected(&source_runtime, &target_runtime).await;
    seed_presence(&target_runtime, &tc, &sc.daemon_id).await;
    seed_presence(&source_runtime, &sc, &tc.daemon_id).await;
    let source_record = source.source(&a);
    let RelayPeerResponse::WorkflowNotificationSources { sources, .. } = target_runtime
        .notification_peer(
            "local",
            &sc.daemon_id,
            RelayPeerRequest::ListWorkflowNotificationSources {
                protocol_version: VERSION,
            },
        )
        .await
        .unwrap()
    else {
        panic!("wrong source list")
    };
    target
        .store
        .notify(NotificationOperation::Cache {
            owner: "local".into(),
            kernel: sc.daemon_id.clone(),
            sources,
        })
        .unwrap();
    target_runtime
        .refresh_notification_sources_from_inventory("local", true)
        .await
        .unwrap();
    let attach = || {
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            session_id: target.session.clone(),
            source_id: source_record.source_id.clone(),
            publication_ref: bp.clone(),
        })
    };
    let LocalDaemonResponse::WorkflowNotificationAttached { subscription } = target_runtime
        .execute_workflow_notification_command(attach(), "local")
        .await
        .unwrap()
    else {
        panic!("wrong attach")
    };
    let held_by_source = |source: &Fixture| {
        source
            .store
            .notification_inventory("local")
            .unwrap()
            .1
            .contains(&subscription)
    };
    assert!(held_by_source(&source), "the source installed the follow");

    // The source kernel goes away; the detach cannot be answered, so the source keeps its row.
    source_stop.send(true).unwrap();
    source_connector.await.unwrap();
    target_runtime
        .execute_workflow_notification_command(
            LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                session_id: target.session.clone(),
                subscription_id: subscription.subscription_id.clone(),
            }),
            "local",
        )
        .await
        .unwrap();
    assert!(
        !target
            .store
            .notification_inventory("local")
            .unwrap()
            .1
            .contains(&subscription),
        "the detached follow is still active here"
    );
    assert!(
        held_by_source(&source),
        "the source never heard of the detach"
    );
    let later = crate::session::unix_epoch_ms() + 3_600_000;
    target_runtime
        .reconcile_remote_notification_installs(later)
        .await;
    assert_eq!(
        target.store.unconfirmed_detaches(),
        1,
        "an unreachable source stays pending"
    );

    // The source stays away past every retry window: the entry is parked, not forgotten.
    for minute in 1..=40u64 {
        target
            .store
            .notify(NotificationOperation::ClaimRemoteReconciles {
                now: later + minute * 60_000,
                limit: 8,
            })
            .unwrap();
    }
    assert_eq!(target.store.unconfirmed_detaches(), 1);
    target_runtime
        .owned
        .notification_inventory_projection
        .update(vec![], vec![]);

    // The source comes back and drops the row.
    let (source_stop, source_connector) = connect(&sr, &source_runtime, &sc, &url).await;
    registered(&registry, &sc.daemon_id).await;
    both_connected(&source_runtime, &target_runtime).await;
    seed_presence(&target_runtime, &tc, &sc.daemon_id).await;
    // Its return is the only thing that makes the parked entry due: a single pass asks it.
    for attempt in 1..=10u64 {
        target_runtime
            .reconcile_remote_notification_installs(later + (40 + attempt) * 60_000)
            .await;
        if !held_by_source(&source) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        target.store.unconfirmed_detaches(),
        0,
        "the source answered, so nothing is left to reconcile"
    );
    assert!(
        !held_by_source(&source),
        "the reconciliation left the detached follow on the source"
    );

    source_stop.send(true).unwrap();
    target_stop.send(true).unwrap();
    let _ = source_connector.await;
    let _ = target_connector.await;
    let _ = server_tx.send(());
    let _ = server_task.await;
    drop((source_runtime, target_runtime));
    cleanup(source);
    cleanup(target);
}
