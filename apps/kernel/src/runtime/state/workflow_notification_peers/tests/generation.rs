//! MP-08 / MP-10 / MP-11: remote work claimed before a newer attach or detach must not undo it.
//! The source kernel is a real kernel behind a real relay.
use super::reconnect::{both_connected, connect};
use super::*;

struct Pair {
    source: Fixture,
    target: Fixture,
    source_runtime: KernelRuntimeState,
    target_runtime: KernelRuntimeState,
    source_connection: Option<(watch::Sender<bool>, tokio::task::JoinHandle<()>)>,
    target_connection: (watch::Sender<bool>, tokio::task::JoinHandle<()>),
    source_router: Arc<CommandRouter>,
    source_config: DaemonConfig,
    target_config: DaemonConfig,
    registry: Arc<tokio::sync::RwLock<chariox_relay::server::RelayRegistry>>,
    url: String,
    server_tx: oneshot::Sender<()>,
    server_task: tokio::task::JoinHandle<()>,
    source_id: String,
    publication: String,
}

fn on_a_big_stack<F: std::future::Future<Output = ()> + Send + 'static>(drill: F) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(32 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(drill)
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn pair(name: &str) -> Pair {
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
    let mut source = Fixture::for_kernel(&format!("{name}-source"));
    let (a, _, _) = source.workflow("reviewer");
    let mut target = Fixture::for_kernel(&format!("{name}-target"));
    let (_, _, publication) = target.workflow("subscriber");
    let (source_router, source_runtime, mut sc) = peer(&mut source, &url, &secret);
    let (tr, target_runtime, mut tc) = peer(&mut target, &url, &secret);
    sc.relay_token = Some(token(&sc, &secret, "local"));
    tc.relay_token = Some(token(&tc, &secret, "local"));
    source_runtime.owned.config_projection.update(sc.clone());
    target_runtime.owned.config_projection.update(tc.clone());
    let source_connection = connect(&source_router, &source_runtime, &sc, &url).await;
    let target_connection = connect(&tr, &target_runtime, &tc, &url).await;
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
    Pair {
        source,
        target,
        source_runtime,
        target_runtime,
        source_connection: Some(source_connection),
        target_connection,
        source_router,
        source_config: sc,
        target_config: tc,
        registry,
        url,
        server_tx,
        server_task,
        source_id: source_record.source_id,
        publication,
    }
}

impl Pair {
    fn attach(&self) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            session_id: self.target.session.clone(),
            source_id: self.source_id.clone(),
            publication_ref: self.publication.clone(),
        })
    }
    async fn attach_now(&self) -> Result<WorkflowNotificationSubscription, DaemonError> {
        match self
            .target_runtime
            .execute_workflow_notification_command(self.attach(), "local")
            .await?
        {
            LocalDaemonResponse::WorkflowNotificationAttached { subscription } => Ok(subscription),
            _ => panic!("wrong attach"),
        }
    }
    async fn detach_now(&self, subscription: &WorkflowNotificationSubscription) {
        self.target_runtime
            .execute_workflow_notification_command(
                LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                    session_id: self.target.session.clone(),
                    subscription_id: subscription.subscription_id.clone(),
                }),
                "local",
            )
            .await
            .unwrap();
    }
    fn source_holds(&self) -> bool {
        self.source
            .store
            .notification_inventory("local")
            .unwrap()
            .1
            .iter()
            .any(|s| s.source_id == self.source_id)
    }
    fn destination_holds(&self) -> bool {
        self.target
            .store
            .notification_inventory("local")
            .unwrap()
            .1
            .iter()
            .any(|s| s.source_id == self.source_id)
    }
    async fn stop_source(&mut self) {
        let (stop, connector) = self.source_connection.take().unwrap();
        stop.send(true).unwrap();
        connector.await.unwrap();
    }
    async fn start_source(&mut self) {
        self.source_connection = Some(
            connect(
                &self.source_router,
                &self.source_runtime,
                &self.source_config,
                &self.url,
            )
            .await,
        );
        registered(&self.registry, &self.source_config.daemon_id).await;
        both_connected(&self.source_runtime, &self.target_runtime).await;
        seed_presence(&self.target_runtime, &self.target_config, &self.source_config.daemon_id).await;
    }
    async fn finish(mut self) {
        if let Some((stop, connector)) = self.source_connection.take() {
            stop.send(true).unwrap();
            let _ = connector.await;
        }
        self.target_connection.0.send(true).unwrap();
        let _ = self.target_connection.1.await;
        let _ = self.server_tx.send(());
        let _ = self.server_task.await;
        drop((self.source_runtime, self.target_runtime));
        cleanup(self.source);
        cleanup(self.target);
    }
}

fn claim(pair: &Pair, now: u64) -> Vec<crate::durable_state::workflow_notifications::remote_install::RemoteReconcile> {
    match pair
        .target
        .store
        .notify(NotificationOperation::ClaimRemoteReconciles { now, limit: 8 })
        .unwrap()
    {
        NotificationOutcome::Reconciles(due) => due,
        _ => panic!("a claim answers with the due reconciliations"),
    }
}

#[test]
fn a_claimed_cleanup_does_not_undo_a_newer_attach() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-cleanup").await;
        // The first attachment goes unanswered: the pump will clean up what the source may hold.
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        pair.start_source().await;
        let due = claim(&pair, crate::session::unix_epoch_ms() + 3_600_000);
        assert_eq!(due.len(), 1);
        assert!(due[0].desired.is_none());
        // A newer attach of the same source and destination succeeds before the cleanup sends.
        pair.target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        let subscription = pair.attach_now().await.unwrap();
        assert!(pair.source_holds() && pair.destination_holds());
        resume_cleanup(&pair, &due[0]).await;
        assert!(
            pair.destination_holds(),
            "the destination lost the follow it was just given"
        );
        assert!(
            pair.source_holds(),
            "the old cleanup removed the follow the newer attach installed on the source"
        );
        let _ = subscription;
        pair.finish().await;
    });
}

#[test]
fn a_claimed_install_does_not_reinstall_a_detached_follow() {
    on_a_big_stack(async {
        let pair = pair("wfgen-install").await;
        let subscription = pair.attach_now().await.unwrap();
        assert!(pair.source_holds());
        // A repeat attach commits and is about to install; the user detaches before it sends.
        let token = begin_install(&pair, &subscription);
        pair.detach_now(&subscription).await;
        assert!(!pair.source_holds() && !pair.destination_holds());
        resume_install(&pair, &subscription, token).await;
        assert!(
            !pair.source_holds(),
            "the stale install put a detached follow back on the source"
        );
        assert!(!pair.destination_holds());
        pair.finish().await;
    });
}

/// What a claimed cleanup does when it finally runs: it asks the source, then reports.
async fn resume_cleanup(
    pair: &Pair,
    item: &crate::durable_state::workflow_notifications::remote_install::RemoteReconcile,
) {
    let _ = pair
        .target_runtime
        .notification_peer(
            &item.owner,
            &item.source_kernel_id,
            RelayPeerRequest::UnsubscribeWorkflowNotifications {
                protocol_version: VERSION,
                subscription_id: item.subscription_id.clone(),
            },
        )
        .await;
    pair.target
        .store
        .notify(NotificationOperation::RemoteReconciled {
            owner: item.owner.clone(),
            subscription_id: item.subscription_id.clone(),
        })
        .unwrap();
}

fn begin_install(pair: &Pair, subscription: &WorkflowNotificationSubscription) {
    pair.target
        .store
        .notify(NotificationOperation::AttachToRemoteSource {
            subscription: subscription.clone(),
        })
        .unwrap();
}

async fn resume_install(pair: &Pair, subscription: &WorkflowNotificationSubscription, _token: ()) {
    let response = pair
        .target_runtime
        .notification_peer(
            &subscription.owner_user_id,
            &subscription.source_kernel_id,
            RelayPeerRequest::SubscribeWorkflowNotifications {
                protocol_version: VERSION,
                source_id: subscription.source_id.clone(),
                target_ref: subscription.clone(),
            },
        )
        .await;
    assert!(matches!(
        response,
        Ok(RelayPeerResponse::WorkflowNotificationSubscribed { .. })
    ));
    pair.target
        .store
        .notify(NotificationOperation::RemoteInstallConfirmed {
            owner: subscription.owner_user_id.clone(),
            subscription_id: subscription.subscription_id.clone(),
        })
        .unwrap();
}
