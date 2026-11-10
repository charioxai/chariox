//! MP-08 / MP-10: a notification trigger follows the reachability of its source kernel within
//! seconds, without waiting for the Cloud inventory to change.
use super::reconnect::{both_connected, connect};
use super::*;
use crate::session::{AgentTriggerNotificationSource, AgentTriggerSource};

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

/// A source kernel and a target kernel behind a real relay; the target follows one source of the
/// source kernel with an agent notification trigger saved the way the product saves it.
struct Drill {
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
    workflow: String,
    trigger_id: String,
}

impl Drill {
    async fn start() -> Self {
        let secret = format!("fixture-{:032x}", rand::random::<u128>());
        let server = Arc::new(RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::scoped_hmac(
                BTreeMap::from([("fixture".into(), secret.clone())]),
                None,
            ),
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
        let (workflow, _, _) = source.workflow("reviewer");
        let mut target = Fixture::for_kernel("wfavail-target");
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
        source.source(&workflow);
        let mut drill = Self {
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
            workflow,
            trigger_id: String::new(),
        };
        drill.refresh_cache().await;
        drill.follow().await;
        drill
    }

    /// The inventory refresh: the target caches what the source kernel lists.
    async fn refresh_cache(&self) {
        let RelayPeerResponse::WorkflowNotificationSources { sources, .. } = self
            .target_runtime
            .notification_peer(
                "local",
                &self.source_config.daemon_id,
                RelayPeerRequest::ListWorkflowNotificationSources {
                    protocol_version: VERSION,
                },
            )
            .await
            .unwrap()
        else {
            panic!("wrong source list")
        };
        self.target
            .store
            .notify(NotificationOperation::Cache {
                owner: "local".into(),
                kernel: self.source_config.daemon_id.clone(),
                sources,
            })
            .unwrap();
    }

    async fn follow(&mut self) {
        let source_id = self.source.source(&self.workflow).source_id;
        let subscription_id = "sub-availability";
        let trigger = self
            .target_runtime
            .owned
            .session_store
            .write()
            .create_agent_notification_trigger(
                &self.target.session,
                "agent",
                "local",
                AgentTriggerNotificationSource::Workflow {
                    source_id: source_id.clone(),
                },
                subscription_id.into(),
                "reviewer".into(),
                None,
            )
            .unwrap();
        self.trigger_id = trigger.id().to_owned();
        self.target_runtime
            .subscribe_agent_notification(
                &self.target_config.daemon_id,
                WorkflowNotificationSubscription {
                    subscription_id: subscription_id.into(),
                    source_id,
                    owner_user_id: "local".into(),
                    created_by_user_id: "local".into(),
                    target_kernel_id: self.target_config.daemon_id.clone(),
                    target_kind: WorkflowNotificationTargetKind::Agent,
                    session_id: self.target.session.clone(),
                    workflow_id: String::new(),
                    publication_id: self.trigger_id.clone(),
                    endpoint_id: "agent".into(),
                    queue_id: "agent".into(),
                    source_available: true,
                    source_kernel_id: self.source_config.daemon_id.clone(),
                    agent_id: Some("agent".into()),
                    trigger_id: Some(self.trigger_id.clone()),
                },
            )
            .await
            .unwrap();
    }

    fn listening(&self) -> bool {
        let session = self
            .target_runtime
            .owned
            .session_snapshot(&self.target.session)
            .unwrap();
        match session
            .agent_triggers()
            .iter()
            .find(|t| t.id() == self.trigger_id)
            .unwrap()
            .source()
        {
            AgentTriggerSource::Notification {
                source_available, ..
            } => *source_available,
            other => panic!("not a notification trigger: {other:?}"),
        }
    }

    /// The owner enables or disables the followed source on the source kernel.
    fn set_source_enabled(&self, enabled: bool) {
        self.source.set_source_enabled(&self.workflow, enabled);
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
        seed_presence(
            &self.target_runtime,
            &self.target_config,
            &self.source_config.daemon_id,
        )
        .await;
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

#[test]
fn a_notification_trigger_follows_the_reachability_of_its_source_kernel() {
    on_a_big_stack(reachability_drill());
}

async fn reachability_drill() {
    let mut drill = Drill::start().await;
    drill
        .target_runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(
        drill.listening(),
        "a reachable source leaves the trigger available"
    );

    // The source kernel stops answering. The inventory is not refreshed (it changes every few
    // minutes); the probe alone has to notice, and one lost answer is not enough.
    drill.stop_source().await;
    let t0 = crate::session::unix_epoch_ms();
    drill.target_runtime.probe_notification_sources(t0).await;
    assert!(drill.listening(), "one lost answer changes nothing");
    drill
        .target_runtime
        .probe_notification_sources(t0 + 5_000)
        .await;
    assert!(
        drill.listening(),
        "a source kernel is asked once per interval"
    );
    drill
        .target_runtime
        .probe_notification_sources(t0 + 10_000)
        .await;
    assert!(
        !drill.listening(),
        "the second lost answer marks the source offline within the interval window"
    );

    // The source kernel returns: the next probe brings the trigger back.
    drill.start_source().await;
    drill
        .target_runtime
        .probe_notification_sources(t0 + 20_000)
        .await;
    assert!(
        drill.listening(),
        "an answering source makes the trigger available again"
    );

    // The kernel's own pump claims the probe; it asks nobody until the interval has passed.
    drill
        .target_runtime
        .route_remote_workflow_notifications()
        .await;
    assert!(
        drill
            .target_runtime
            .claim_due_notification_source_probes(crate::session::unix_epoch_ms())
            .is_empty(),
        "the pump started the probe"
    );
    drill.finish().await;
}

// MP-08 / MP-10: a source the owner disables on a kernel that stays online drops out of that
// kernel's list. The kernel keeps being probed while the trigger exists, so enabling the source
// again brings the trigger back without any inventory refresh.
#[test]
fn a_notification_trigger_recovers_when_its_remote_source_is_enabled_again() {
    on_a_big_stack(re_enable_drill());
}

async fn re_enable_drill() {
    let drill = Drill::start().await;
    drill
        .target_runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(drill.listening());

    let t0 = crate::session::unix_epoch_ms();
    drill.set_source_enabled(false);
    drill.target_runtime.probe_notification_sources(t0).await;
    assert!(
        !drill.listening(),
        "a disabled source no longer reaches the trigger"
    );

    drill.set_source_enabled(true);
    drill
        .target_runtime
        .probe_notification_sources(t0 + 10_000)
        .await;
    assert!(
        drill.listening(),
        "the source kernel is still probed, so the enabled source brings the trigger back"
    );
    drill.finish().await;
}
