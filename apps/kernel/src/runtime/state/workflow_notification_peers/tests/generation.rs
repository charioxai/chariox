//! MP-08 / MP-10 / MP-11: remote work claimed before a newer attach or detach must not undo it.
//! The source kernel is a real kernel behind a real relay.
use super::reconnect::{both_connected, connect};
use super::*;
use crate::durable_state::workflow_notifications::remote_install::Generation;

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
    source_workflow: String,
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
        source_workflow: a,
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
    /// A run of the followed workflow ends on the source kernel.
    fn complete_run(&mut self, run: &str) {
        let workflow = self.source_workflow.clone();
        self.source.complete(
            &workflow,
            run,
            None,
            crate::session::WorkflowRunStatus::Completed,
            "done",
        );
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

fn claim(
    pair: &Pair,
    now: u64,
) -> Vec<crate::durable_state::workflow_notifications::remote_install::RemoteReconcile> {
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
        resume_cleanup(&pair, due[0].clone()).await;
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

/// What a claimed cleanup does when it finally runs.
async fn resume_cleanup(
    pair: &Pair,
    item: crate::durable_state::workflow_notifications::remote_install::RemoteReconcile,
) {
    pair.target_runtime.ask_remote_reconcile(item).await;
}

fn begin_install(pair: &Pair, subscription: &WorkflowNotificationSubscription) -> Generation {
    match pair
        .target
        .store
        .notify(NotificationOperation::AttachToRemoteSource {
            subscription: subscription.clone(),
        })
        .unwrap()
    {
        NotificationOutcome::RemoteInstalling { generation, .. } => generation,
        _ => panic!("a remote attach answers with the install it left to do"),
    }
}

async fn resume_install(
    pair: &Pair,
    subscription: &WorkflowNotificationSubscription,
    generation: Generation,
) {
    pair.target_runtime
        .install_on_source(subscription, generation)
        .await
        .unwrap();
}

/// A request that was already on its way when the state moved on, and lands afterwards.
async fn land_late(pair: &Pair, request: RelayPeerRequest) {
    let _ = pair
        .target_runtime
        .notification_peer("local", &pair.source_config.daemon_id, request)
        .await;
}

fn unsubscribe(
    subscription: &WorkflowNotificationSubscription,
    generation: Generation,
) -> RelayPeerRequest {
    RelayPeerRequest::UnsubscribeWorkflowNotifications {
        protocol_version: VERSION,
        subscription_id: subscription.subscription_id.clone(),
        generation: generation as u64,
    }
}

fn subscribe(
    subscription: &WorkflowNotificationSubscription,
    generation: Generation,
) -> RelayPeerRequest {
    RelayPeerRequest::SubscribeWorkflowNotifications {
        protocol_version: VERSION,
        source_id: subscription.source_id.clone(),
        target_ref: subscription.clone(),
        generation: generation as u64,
    }
}

fn later() -> u64 {
    crate::session::unix_epoch_ms() + 3_600_000
}

#[test]
fn a_stale_install_that_lands_after_a_detach_is_ignored_at_the_source() {
    on_a_big_stack(async {
        let pair = pair("wfgen-lateinstall").await;
        let subscription = pair.attach_now().await.unwrap();
        let generation = begin_install(&pair, &subscription);
        pair.detach_now(&subscription).await;
        // The install was already sent: it lands after the detach, and the source ignores it.
        land_late(&pair, subscribe(&subscription, generation)).await;
        assert!(
            !pair.source_holds(),
            "the source installed a follow the detach had already ended"
        );
        pair.target
            .store
            .notify(NotificationOperation::RemoteInstallConfirmed {
                owner: "local".into(),
                subscription_id: subscription.subscription_id.clone(),
                generation,
            })
            .unwrap();
        assert!(
            !pair.destination_holds(),
            "the stale answer restored the follow"
        );
        assert_eq!(
            pair.target.store.unconfirmed_detaches(),
            1,
            "the source may hold a follow nobody wants: it is asked to drop it"
        );
        pair.target_runtime
            .reconcile_remote_notification_installs(later())
            .await;
        assert!(!pair.source_holds());
        assert_eq!(pair.target.store.unconfirmed_installs(), 0);
        pair.finish().await;
    });
}

#[test]
fn a_stale_cleanup_answer_after_a_newer_attach_leaves_the_follow_on_both_kernels() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-latecleanup").await;
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        pair.start_source().await;
        pair.target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        let mut due = claim(&pair, later());
        let item = due.remove(0);
        let subscription = pair.attach_now().await.unwrap();
        // The cleanup was already sent: it lands after the newer attach, and the source ignores it.
        land_late(&pair, unsubscribe(&subscription, item.generation)).await;
        assert!(pair.source_holds());
        pair.target
            .store
            .notify(NotificationOperation::RemoteReconcileSettled {
                owner: item.owner.clone(),
                subscription_id: item.subscription_id.clone(),
                generation: item.generation,
                matched: true,
            })
            .unwrap();
        assert!(pair.destination_holds());
        pair.target_runtime
            .reconcile_remote_notification_installs(later())
            .await;
        assert!(
            pair.source_holds(),
            "the source was left without the follow"
        );
        assert!(pair.destination_holds());
        assert_eq!(pair.target.store.unconfirmed_installs(), 0);
        pair.finish().await;
    });
}

#[test]
fn a_second_attach_keeps_the_state_the_first_one_replaced() {
    on_a_big_stack(async {
        let pair = pair("wfgen-twoattaches").await;
        let subscription = pair.attach_now().await.unwrap();
        pair.detach_now(&subscription).await;
        // Two attaches are in flight together; the later one is refused.
        let first = begin_install(&pair, &subscription);
        let second = begin_install(&pair, &subscription);
        assert!(second > first);
        let ids = ("local".to_string(), subscription.subscription_id.clone());
        pair.target
            .store
            .notify(NotificationOperation::RemoteInstallConfirmed {
                owner: ids.0.clone(),
                subscription_id: ids.1.clone(),
                generation: first,
            })
            .unwrap();
        assert!(
            pair.destination_holds(),
            "the later attach is still in flight"
        );
        pair.target
            .store
            .notify(NotificationOperation::RemoteInstallFailed {
                owner: ids.0.clone(),
                subscription_id: ids.1.clone(),
                generation: second,
            })
            .unwrap();
        assert!(
            !pair.destination_holds(),
            "the refused attach restored the first attach's attempt instead of the detached state"
        );
        let due = claim(&pair, later());
        assert_eq!(due.len(), 1);
        assert!(
            due[0].desired.is_none(),
            "the source is asked to hold nothing"
        );
        pair.finish().await;
    });
}

#[test]
fn a_claimed_ask_survives_a_restart_and_a_reconnect_reset_without_double_effect() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-restart").await;
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        let now = later();
        let first = claim(&pair, now).remove(0);
        assert!(
            claim(&pair, now + 1_000).is_empty(),
            "claimed work waits for its window"
        );
        // The kernel restarted before the claimed ask was sent: nothing is in memory, and the
        // entry is asked again with the generation it was claimed under.
        let again = claim(&pair, now + 31_000).remove(0);
        assert_eq!(again.generation, first.generation);
        // The source reconnects while that ask is in flight: the reset makes it due again.
        pair.target
            .store
            .notify(NotificationOperation::RemoteKernelsConnected {
                kernels: vec![again.source_kernel_id.clone()],
            })
            .unwrap();
        let twice = claim(&pair, now + 32_000).remove(0);
        assert_eq!(twice.generation, again.generation);
        for item in [again, twice] {
            pair.target
                .store
                .notify(NotificationOperation::RemoteReconcileSettled {
                    owner: item.owner,
                    subscription_id: item.subscription_id,
                    generation: item.generation,
                    matched: true,
                })
                .unwrap();
        }
        assert_eq!(
            pair.target.store.unconfirmed_installs(),
            0,
            "the same ask answered twice left work behind"
        );
        pair.finish().await;
    });
}

#[test]
fn a_detach_the_source_did_not_confirm_does_not_replace_a_newer_attach() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-detachrace").await;
        let subscription = pair.attach_now().await.unwrap();
        pair.stop_source().await;
        // The detach committed here and waits for the source; a newer attach follows it.
        let detached = match pair
            .target
            .store
            .notify(NotificationOperation::Detach {
                subscription_id: subscription.subscription_id.clone(),
                owner: "local".into(),
                kernel: subscription.target_kernel_id.clone(),
            })
            .unwrap()
        {
            NotificationOutcome::Detached {
                generation: Some(generation),
            } => generation,
            _ => panic!("a detach answers with its generation"),
        };
        let generation = begin_install(&pair, &subscription);
        pair.target
            .store
            .notify(NotificationOperation::RemoteInstallConfirmed {
                owner: "local".into(),
                subscription_id: subscription.subscription_id.clone(),
                generation,
            })
            .unwrap();
        pair.target
            .store
            .notify(NotificationOperation::RemoteDetachSettled {
                owner: "local".into(),
                subscription_id: subscription.subscription_id.clone(),
                source_kernel: subscription.source_kernel_id.clone(),
                generation: detached,
                confirmed: false,
            })
            .unwrap();
        assert!(
            pair.destination_holds(),
            "the late detach answer ended the newer attach"
        );
        let due = claim(&pair, later());
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].desired.as_ref(),
            Some(&subscription),
            "the source is asked to hold the current follow, not to drop it"
        );
        pair.finish().await;
    });
}

#[test]
fn a_cleanup_in_transit_that_lands_after_a_newer_attach_is_ignored_at_the_source() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-transit").await;
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        pair.start_source().await;
        pair.target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        let item = claim(&pair, later()).remove(0);
        let subscription = pair.attach_now().await.unwrap();
        // The cleanup was sent before the newer attach and lands after it.
        land_late(&pair, unsubscribe(&subscription, item.generation)).await;
        assert!(
            pair.source_holds(),
            "the source disabled the follow the newer attach installed"
        );
        assert!(pair.destination_holds());
        // A run ends inside the interval the stale cleanup would have left the follow disabled.
        pair.complete_run("inside-the-window");
        assert_eq!(
            pair.source.store.receipts(),
            1,
            "the occurrence completed after the late cleanup has no receipt"
        );
        pair.finish().await;
    });
}

#[test]
fn a_repeat_attach_that_fails_keeps_the_reinstall_the_source_still_owes() {
    on_a_big_stack(async {
        let mut pair = pair("wfgen-repeat").await;
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        pair.start_source().await;
        pair.target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        let item = claim(&pair, later()).remove(0);
        let subscription = pair.attach_now().await.unwrap();
        // The old cleanup's answer arrives late: the current follow is owed to the source again.
        pair.target
            .store
            .notify(NotificationOperation::RemoteReconcileSettled {
                owner: item.owner,
                subscription_id: item.subscription_id,
                generation: item.generation,
                matched: true,
            })
            .unwrap();
        assert_eq!(pair.target.store.unconfirmed_installs(), 1);
        // The same attach is repeated and its request goes unanswered.
        pair.stop_source().await;
        assert!(pair.attach_now().await.is_err());
        assert_eq!(
            pair.target.store.unconfirmed_installs(),
            1,
            "the failed repeat deleted the reinstall nobody had confirmed"
        );
        // The entry survives a reconnect and reaches the source.
        pair.start_source().await;
        pair.target_runtime
            .reconcile_remote_notification_installs(later())
            .await;
        assert!(pair.source_holds() && pair.destination_holds());
        assert_eq!(pair.target.store.unconfirmed_installs(), 0);
        let _ = subscription;
        pair.finish().await;
    });
}

#[test]
fn a_detach_whose_request_never_left_this_kernel_is_still_owed_to_the_source() {
    on_a_big_stack(async {
        let pair = pair("wfgen-detachowed").await;
        let subscription = pair.attach_now().await.unwrap();
        assert!(pair.source_holds());
        // The kernel stopped between the local detach and its request to the source.
        pair.target
            .store
            .notify(NotificationOperation::Detach {
                subscription_id: subscription.subscription_id.clone(),
                owner: "local".into(),
                kernel: subscription.target_kernel_id.clone(),
            })
            .unwrap();
        assert!(!pair.destination_holds());
        assert_eq!(
            pair.target.store.unconfirmed_detaches(),
            1,
            "nothing remembers that the source must still drop the follow"
        );
        pair.target_runtime
            .reconcile_remote_notification_installs(later())
            .await;
        assert!(!pair.source_holds());
        assert_eq!(pair.target.store.unconfirmed_installs(), 0);
        pair.finish().await;
    });
}

#[test]
fn a_detach_the_source_confirmed_leaves_nothing_owed() {
    on_a_big_stack(async {
        let pair = pair("wfgen-detachdone").await;
        let subscription = pair.attach_now().await.unwrap();
        pair.detach_now(&subscription).await;
        assert!(!pair.source_holds() && !pair.destination_holds());
        assert_eq!(pair.target.store.unconfirmed_installs(), 0);
        pair.finish().await;
    });
}
