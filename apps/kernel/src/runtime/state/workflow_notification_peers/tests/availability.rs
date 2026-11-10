//! MP-08 / MP-10: a notification trigger follows the reachability of its source kernel within
//! seconds, without waiting for the Cloud inventory to change.
use super::reconnect::{both_connected, connect};
use super::*;
use crate::session::{AgentTriggerNotificationSource, AgentTriggerSource};

#[test]
fn a_notification_trigger_follows_the_reachability_of_its_source_kernel() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(32 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(availability_drill())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn availability_drill() {
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
    let mut source = Fixture::for_kernel("wfavail-source");
    let (a, _, _) = source.workflow("reviewer");
    let mut target = Fixture::for_kernel("wfavail-target");
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
    let source_id = source.source(&a).source_id;
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
    let trigger_id = target_runtime
        .owned
        .session_store
        .write()
        .create_agent_notification_trigger(
            &target.session,
            "agent",
            "local",
            AgentTriggerNotificationSource::Workflow {
                source_id: source_id.clone(),
            },
            "sub-availability".into(),
            "reviewer".into(),
            None,
        )
        .unwrap()
        .id()
        .to_owned();
    let listening = || {
        let session = target_runtime
            .owned
            .session_snapshot(&target.session)
            .unwrap();
        match session
            .agent_triggers()
            .iter()
            .find(|t| t.id() == trigger_id)
            .unwrap()
            .source()
        {
            AgentTriggerSource::Notification {
                source_available, ..
            } => *source_available,
            other => panic!("not a notification trigger: {other:?}"),
        }
    };
    target_runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(
        listening(),
        "a reachable source leaves the trigger available"
    );

    // The source kernel stops answering. The inventory is not refreshed (it changes every few
    // minutes); the probe alone has to notice, and one lost answer is not enough.
    source_stop.send(true).unwrap();
    source_connector.await.unwrap();
    let t0 = crate::session::unix_epoch_ms();
    target_runtime.probe_notification_sources(t0).await;
    assert!(listening(), "one lost answer changes nothing");
    target_runtime.probe_notification_sources(t0 + 5_000).await;
    assert!(listening(), "a source kernel is asked once per interval");
    target_runtime.probe_notification_sources(t0 + 10_000).await;
    assert!(
        !listening(),
        "the second lost answer marks the source offline within the interval window"
    );

    // The source kernel returns: the next probe brings the trigger back.
    let (source_stop, source_connector) = connect(&sr, &source_runtime, &sc, &url).await;
    registered(&registry, &sc.daemon_id).await;
    both_connected(&source_runtime, &target_runtime).await;
    seed_presence(&target_runtime, &tc, &sc.daemon_id).await;
    target_runtime.probe_notification_sources(t0 + 20_000).await;
    assert!(
        listening(),
        "an answering source makes the trigger available again"
    );

    // The kernel's own pump claims the probe; it asks nobody until the interval has passed.
    target_runtime.route_remote_workflow_notifications().await;
    assert!(
        target_runtime
            .claim_due_notification_source_probes(crate::session::unix_epoch_ms())
            .is_empty(),
        "the pump started the probe"
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
