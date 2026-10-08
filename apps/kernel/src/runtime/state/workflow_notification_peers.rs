//! Notification peer adapter: existing discovery, owner-bound E2EE transport,
//! bounded picker refresh. No directory, scheduler, or credential ownership.
use super::workflow_notification_router::{source_available, summary};
use super::*;
use crate::durable_state::{
    notification_target::WorkflowNotificationTarget,
    workflow_notifications::{self as store, NotificationOperation, NotificationOutcome},
};
use crate::local::*;
pub(super) const VERSION: u32 = 82;
const TIMEOUT: Duration = Duration::from_secs(3);

impl KernelRuntimeState {
    pub(super) fn ensure_notification_profile_owner(&self, owner: &str) -> Result<(), DaemonError> {
        if self
            .owned
            .config_projection
            .snapshot()
            .cloud_relay
            .as_ref()
            .is_some_and(|profile| profile.user_id != owner)
        {
            return Err(store::error(
                "workflow notifications require the authenticated profile owner",
            ));
        }
        Ok(())
    }

    fn ensure_notification_home_role(&self) -> Result<(), DaemonError> {
        let config = self.owned.config_projection.snapshot();
        if config.kernel_runtime_role != crate::config::KernelRuntimeRole::General
            || config.daemon_id.starts_with("slice:")
        {
            return Err(store::error(
                "workflow notification peers require a home kernel",
            ));
        }
        Ok(())
    }

    pub(crate) fn notification_peer_owner(
        &self,
        identity: &chariox_relay::protocol::RelayCallerIdentity,
    ) -> Option<String> {
        self.ensure_notification_home_role().ok()?;
        if identity.subject.starts_with("slice:") {
            return None;
        }
        let config = self.owned.config_projection.snapshot();
        let profile = config.cloud_relay.as_ref()?;
        if identity.realm_id != profile.realm_id
            || identity.user_id.as_deref() != Some(profile.user_id.as_str())
        {
            return None;
        }
        Some(profile.user_id.clone())
    }

    pub(super) async fn notification_peer(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        self.ensure_notification_home_role()?;
        let config = self.owned.config_projection.snapshot();
        if config
            .cloud_relay
            .as_ref()
            .is_some_and(|profile| profile.user_id != owner)
        {
            return Err(store::error("notification transport profile owner changed"));
        }
        let target = ClientTarget {
            daemon_id: Some(kernel.into()),
            daemon_alias: None,
        };
        // The existing inventory pins a live target key; connected transport
        // retains sender/response correlation and encryption key verification.
        let (_, kernels) = self.owned.notification_inventory_projection.snapshot();
        if let Some(peer) = kernels.iter().find(|p| p.kernel_id == kernel) {
            return crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(&config,&self.owned.relay_state,target,&peer.public_key,request,TIMEOUT).await;
        }
        // Existing peer discovery handles reconnect and targets not in the picker.
        crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
            &config,
            &self.owned.relay_state,
            target,
            request,
            TIMEOUT,
        )
        .await
    }

    pub(super) async fn refresh_notification_sources(
        &self,
        owner: &str,
    ) -> Result<(), DaemonError> {
        self.ensure_notification_home_role()?;
        self.ensure_notification_profile_owner(owner)?;
        let projection = self.owned.notification_inventory_projection.clone();
        // Refresh uses the same Cloud inventory credential as the waiting room.
        // No workflow data is sent to Cloud. A picker failure retains cache offline.
        let refresh = crate::transport::relay_client::refresh_remote_inventory_projection(
            self.owned.config_projection.clone(),
            projection.clone(),
        );
        let fresh = matches!(tokio::time::timeout(TIMEOUT, refresh).await, Ok(Ok(())));
        self.refresh_notification_sources_from_inventory(owner, fresh)
            .await
    }
    pub(crate) async fn refresh_notification_sources_from_inventory(
        &self,
        owner: &str,
        fresh: bool,
    ) -> Result<(), DaemonError> {
        self.ensure_notification_home_role()?;
        self.ensure_notification_profile_owner(owner)?;
        let (_, kernels) = self.owned.notification_inventory_projection.snapshot();
        let home = self.owned.config_projection.snapshot().daemon_id;
        let mut cache = self
            .owned
            .durable_state_store
            .notification_cached_sources(owner)?;
        let mut touched: BTreeSet<String> = cache.iter().map(|s| s.kernel_id.clone()).collect();
        for item in &mut cache {
            item.available = false;
        }
        // At most four outstanding requests, a bounded 128-kernel picker.
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        if fresh {
            for peers in kernels
                .iter()
                .filter(|k| k.kernel_id != home)
                .take(128)
                .collect::<Vec<_>>()
                .chunks(4)
            {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                let mut set = tokio::task::JoinSet::new();
                for peer in peers {
                    let runtime = self.clone();
                    let kernel = peer.kernel_id.clone();
                    let owner = owner.to_owned();
                    set.spawn(async move {
                        let response = tokio::time::timeout_at(
                            deadline,
                            runtime.notification_peer(
                                &owner,
                                &kernel,
                                RelayPeerRequest::ListWorkflowNotificationSources {
                                    protocol_version: VERSION,
                                },
                            ),
                        )
                        .await
                        .ok()
                        .and_then(Result::ok);
                        (kernel, response)
                    });
                }
                while let Some(Ok((kernel, response))) = set.join_next().await {
                    if let Some(RelayPeerResponse::WorkflowNotificationSources {
                        protocol_version: VERSION,
                        sources,
                    }) = response
                    {
                        if sources.len() > 1024
                            || sources.iter().any(|s| {
                                s.kernel_id != kernel || s.fields.len() > 14 || s.name.len() > 512
                            })
                        {
                            continue;
                        }
                        touched.insert(kernel.clone());
                        cache.retain(|s| s.kernel_id != kernel);
                        cache.extend(sources.into_iter().map(|mut s| {
                            s.available = true;
                            s
                        }));
                    }
                }
            }
        }
        let mut groups = BTreeMap::<String, Vec<WorkflowNotificationSourceSummary>>::new();
        for source in cache {
            groups
                .entry(source.kernel_id.clone())
                .or_default()
                .push(source);
        }
        for kernel in touched {
            groups.entry(kernel).or_default();
        }
        self.ensure_notification_profile_owner(owner)?;
        for (kernel, sources) in groups {
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Cache {
                    owner: owner.into(),
                    kernel,
                    sources,
                })?;
        }
        Ok(())
    }

    pub(super) async fn execute_workflow_notification_command(
        &self,
        request: LocalDaemonRequest,
        owner: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_notification_profile_owner(owner)?;
        if matches!(&request, LocalDaemonRequest::ListWorkflowNotifications(_)) {
            self.refresh_notification_sources(owner).await?;
        }
        if let LocalDaemonRequest::DetachWorkflowNotification(request) = &request {
            let (_, subs, _) = self
                .owned
                .durable_state_store
                .notification_inventory(owner)?;
            let sub = subs
                .into_iter()
                .find(|s| {
                    s.subscription_id == request.subscription_id
                        && s.session_id == request.session_id
                        && s.target_kernel_id == self.owned.config_projection.snapshot().daemon_id
                })
                .ok_or_else(|| store::error("notification not attached"))?;
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Detach {
                    subscription_id: sub.subscription_id.clone(),
                    owner: owner.into(),
                    kernel: sub.target_kernel_id.clone(),
                })?;
            if sub.source_kernel_id != sub.target_kernel_id {
                let _ = self
                    .notification_peer(
                        owner,
                        &sub.source_kernel_id,
                        RelayPeerRequest::UnsubscribeWorkflowNotifications {
                            protocol_version: VERSION,
                            subscription_id: sub.subscription_id.clone(),
                        },
                    )
                    .await;
            }
            return Ok(LocalDaemonResponse::WorkflowNotificationDetached {
                subscription_id: sub.subscription_id,
            });
        }
        let result = self.execute_workflow_notification_request(request, owner)?;
        if let LocalDaemonResponse::WorkflowNotificationAttached { subscription } = result {
            if subscription.source_kernel_id != subscription.target_kernel_id {
                let response = self
                    .notification_peer(
                        owner,
                        &subscription.source_kernel_id,
                        RelayPeerRequest::SubscribeWorkflowNotifications {
                            protocol_version: VERSION,
                            source_workflow_ref: subscription.source_id.clone(),
                            target_ref: subscription.clone(),
                        },
                    )
                    .await;
                if !matches!(&response,Ok(RelayPeerResponse::WorkflowNotificationSubscribed {subscription:s}) if *s==subscription)
                {
                    let mut sources = self
                        .owned
                        .durable_state_store
                        .notification_cached_sources(owner)?;
                    sources.retain(|s| s.kernel_id == subscription.source_kernel_id);
                    for source in &mut sources {
                        if source.source_id == subscription.source_id {
                            source.available = false;
                        }
                    }
                    self.owned
                        .durable_state_store
                        .notify(NotificationOperation::Cache {
                            owner: owner.into(),
                            kernel: subscription.source_kernel_id.clone(),
                            sources,
                        })?;
                    return Err(store::error(
                        "source not available; refresh sources and retry attach",
                    ));
                }
            }
            return self
                .attached_within_grant(subscription)
                .map(
                    |subscription| LocalDaemonResponse::WorkflowNotificationAttached {
                        subscription,
                    },
                );
        }
        Ok(result)
    }

    /// Called only after the relay adapter verifies an unexpired, sender-key-bound
    /// kernel or relay-projected machine identity. Ownership is never taken from the request body.
    pub(crate) fn receive_workflow_notification_peer(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        self.ensure_notification_home_role()?;
        self.owned
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let sessions = self.owned.session_store.read();
                let home = self.owned.config_projection.snapshot().daemon_id;
                match request {
                    RelayPeerRequest::ListWorkflowNotificationSources {
                        protocol_version: VERSION,
                    } => {
                        let (sources, _, _) = self
                            .owned
                            .durable_state_store
                            .notification_inventory(owner)?;
                        let sources = sources
                            .into_iter()
                            .filter(|s| s.enabled && source_available(&sessions, s))
                            .map(|s| summary(&s))
                            .collect();
                        Ok(RelayPeerResponse::WorkflowNotificationSources {
                            protocol_version: VERSION,
                            sources,
                        })
                    }
                    RelayPeerRequest::SubscribeWorkflowNotifications {
                        protocol_version: VERSION,
                        source_workflow_ref,
                        target_ref,
                    } => {
                        let (sources, _, _) = self
                            .owned
                            .durable_state_store
                            .notification_inventory(owner)?;
                        let source = sources
                            .into_iter()
                            .find(|s| {
                                s.source_id == source_workflow_ref
                                    && s.enabled
                                    && source_available(&sessions, s)
                            })
                            .ok_or_else(|| store::error("source not available"))?;
                        if target_ref.owner_user_id != owner
                            || target_ref.target_kernel_id != kernel
                            || target_ref.source_kernel_id != home
                            || target_ref.source_id != source.source_id
                        {
                            return Err(store::error("notification owner mismatch"));
                        }
                        match self.owned.durable_state_store.notify(
                            NotificationOperation::RemoteAttach {
                                subscription: target_ref,
                            },
                        )? {
                            NotificationOutcome::Subscription(subscription) => {
                                Ok(RelayPeerResponse::WorkflowNotificationSubscribed {
                                    subscription,
                                })
                            }
                            _ => Err(store::error("notification subscription failed")),
                        }
                    }
                    RelayPeerRequest::UnsubscribeWorkflowNotifications {
                        protocol_version: VERSION,
                        subscription_id,
                    } => {
                        self.owned
                            .durable_state_store
                            .notify(NotificationOperation::Detach {
                                subscription_id,
                                owner: owner.into(),
                                kernel: kernel.into(),
                            })?;
                        Ok(RelayPeerResponse::WorkflowNotificationUnsubscribed)
                    }
                    RelayPeerRequest::DeliverWorkflowNotification {
                        protocol_version: VERSION,
                        subscription_id,
                        envelope,
                    } => {
                        let (_, subs, _) = self
                            .owned
                            .durable_state_store
                            .notification_inventory(owner)?;
                        let sub = subs
                            .into_iter()
                            .find(|s| {
                                s.subscription_id == subscription_id
                                    && s.source_id == envelope.source_id
                                    && s.source_kernel_id == kernel
                                    && s.target_kernel_id == home
                            })
                            .filter(|s| self.owned.notification_grant_live(s))
                            .ok_or_else(|| store::error("notification not attached"))?;
                        let target = WorkflowNotificationTarget::resolve(
                            &sessions,
                            owner,
                            &sub.session_id,
                            &sub.publication_id,
                            Some(&sub.queue_id),
                        )
                        .map_err(|e| store::error(e.to_string()))?;
                        if sessions.get_session(&sub.session_id)?.owner_user_id() != owner
                            || target.target().endpoint_id != sub.endpoint_id
                        {
                            return Err(store::error("notification target changed"));
                        }
                        match self.owned.durable_state_store.notify(
                            NotificationOperation::Accept {
                                subscription: sub,
                                envelope,
                            },
                        )? {
                            NotificationOutcome::Ack(ack) => {
                                Ok(RelayPeerResponse::WorkflowNotificationAccepted { ack })
                            }
                            _ => Err(store::error("notification admission failed")),
                        }
                    }
                    _ => Err(store::error(
                        "workflow notification requires peer protocol 82",
                    )),
                }
            })
    }

    /// Runs within the existing App pump reservation, never a second timer.
    pub(super) async fn route_remote_workflow_notifications(&self) {
        if !self.owned.publication_activation.is_active()
            || self
                .owned
                .durable_state_store
                .require_writer_healthy()
                .is_err()
        {
            return;
        }
        let now = crate::session::unix_epoch_ms();
        let home = self.owned.config_projection.snapshot().daemon_id;
        let Ok(candidates) = self
            .owned
            .durable_state_store
            .notification_candidates(false, now, 8)
        else {
            return;
        };
        let mut deliveries = tokio::task::JoinSet::new();
        for (sub, envelope) in candidates {
            if sub.target_kernel_id == home {
                continue;
            }
            // Rotate before any check, as the local router does, so rows whose
            // source is gone can never starve live deliveries of this window.
            let _ = self
                .owned
                .durable_state_store
                .notify(NotificationOperation::Retry {
                    subscription_id: sub.subscription_id.clone(),
                    source_id: sub.source_id.clone(),
                    occurrence_id: envelope.occurrence_id.clone(),
                    accepted: false,
                    at: now.saturating_add(5000),
                });
            // Deletion/transfer do not rewrite pending delivery records. They expire.
            let valid = {
                let sessions = self.owned.session_store.read();
                self.owned
                    .durable_state_store
                    .notification_inventory(&sub.owner_user_id)
                    .is_ok_and(|(sources, _, _)| {
                        sources.iter().any(|s| {
                            s.source_id == envelope.source_id
                                && s.enabled
                                && source_available(&sessions, s)
                        })
                    })
            };
            if !valid {
                continue;
            }
            let runtime = self.clone();
            deliveries.spawn(async move {
                if let Ok(RelayPeerResponse::WorkflowNotificationAccepted { .. }) = runtime
                    .notification_peer(
                        &sub.owner_user_id,
                        &sub.target_kernel_id,
                        RelayPeerRequest::DeliverWorkflowNotification {
                            protocol_version: VERSION,
                            subscription_id: sub.subscription_id.clone(),
                            envelope: envelope.clone(),
                        },
                    )
                    .await
                {
                    let _ = runtime.owned.durable_state_store.notify(
                        NotificationOperation::Acknowledge {
                            subscription_id: sub.subscription_id,
                            occurrence_id: envelope.occurrence_id,
                        },
                    );
                }
            });
        }
        while deliveries.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
    use crate::{
        app::DaemonApp,
        config::{DaemonConfig, PersistedCloudRelayProfile},
        runtime::router::CommandRouter,
    };
    use chariox_relay::{
        auth::{
            encode_scoped_hmac_token, RelayAction, RelayAuthVerifier, RelaySubjectKind,
            RelayTokenClaims,
        },
        RelayConfig, RelayServer,
    };
    use tokio::sync::{oneshot, watch};

    fn peer(
        f: &mut Fixture,
        url: &str,
        secret: &str,
    ) -> (Arc<CommandRouter>, KernelRuntimeState, DaemonConfig) {
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = f
            .sessions
            .get_session(&f.session)
            .unwrap()
            .host_daemon_id()
            .into();
        config.host_machine_id = format!("machine-{}", config.daemon_id);
        config.user_config.state.path = Some(f.root.join("runtime/state.db").display().to_string());
        config.user_config_path = f.root.join("runtime/config.toml");
        config = config.with_session_history_root(f.root.join("history"));
        config.user_config.history.operational.path =
            Some(f.root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(f.root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(f.root.join("artifacts/index.db").display().to_string());
        config.relay_url = Some(url.into());
        config.relay_heartbeat_ms = 50;
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            realm_id: "wfnotify-fixture".into(),
            user_id: "local".into(),
            api_url: "http://127.0.0.1:1".into(),
            ..Default::default()
        });
        config.relay_token = Some(token(&config, secret, "local"));
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        ));
        let runtime = router.runtime_state();
        f.store = runtime.owned.durable_state_store.clone();
        for session in f.sessions.list_sessions() {
            runtime
                .owned
                .session_store
                .write()
                .restore_session(session.clone());
            f.store
                .persist_workflow_runtime_transition(&session, "notification peer fixture")
                .unwrap();
        }
        (router, runtime, config)
    }
    fn token(config: &DaemonConfig, secret: &str, user: &str) -> String {
        let now = crate::session::unix_epoch_ms();
        encode_scoped_hmac_token(
            &RelayTokenClaims {
                issuer: "fixture".into(),
                subject: config.daemon_id.clone(),
                subject_kind: RelaySubjectKind::Kernel,
                realm_id: "wfnotify-fixture".into(),
                allowed_actions: vec![
                    RelayAction::DaemonRegister,
                    RelayAction::DaemonHeartbeat,
                    RelayAction::PeerRequest,
                    RelayAction::PeerEvent,
                    RelayAction::ClientMetadataRead,
                    RelayAction::PacketRoute,
                ],
                allowed_targets: None,
                issued_at_ms: now,
                expires_at_ms: now + 60_000,
                token_id: format!("fixture-{}", config.daemon_id),
                account_id: None,
                organization_id: None,
                user_id: Some(user.into()),
                device_id: None,
                machine_id: Some(config.host_machine_id.clone()),
                client_id: None,
                session_id: None,
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config.relay_public_key,
                    ),
                ),
                entitlements_version: None,
            },
            secret,
        )
        .unwrap()
    }
    async fn registered(
        registry: &Arc<tokio::sync::RwLock<chariox_relay::server::RelayRegistry>>,
        id: &str,
    ) {
        for _ in 0..100 {
            if registry
                .read()
                .await
                .daemon_in_realm("wfnotify-fixture", id)
                .is_some()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("MP-08 / MP-10: fixture kernel did not register");
    }
    #[test]
    fn local_relay_82_subscribe_offline_retry_ack_loss_dedup_queue_and_unsubscribe() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_stack_size(32 * 1024 * 1024)
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(relay_drill())
            })
            .unwrap()
            .join()
            .unwrap();
    }
    // MP-08/MP-10/MP-11 security F8/F9: owner-wide transport is not worker authority.
    #[test]
    fn security_f8_slice_identity_cannot_read_home_notifications() {
        let mut f = Fixture::for_kernel("notification-home");
        let (_, runtime, _) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
        let identity = chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "wfnotify-fixture".into(),
            subject: "slice:child".into(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms: crate::session::unix_epoch_ms() + 60_000,
            token_id: None,
            user_id: Some("local".into()),
            public_key_thumbprint: None,
        };
        assert!(runtime.notification_peer_owner(&identity).is_none());
    }

    #[test]
    fn security_f8_local_worker_cannot_serve_home_notifications() {
        let mut f = Fixture::for_kernel("notification-worker");
        let (_, runtime, mut config) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
        config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
        runtime.owned.config_projection.update(config);
        assert!(runtime
            .receive_workflow_notification_peer(
                "home-peer",
                "local",
                RelayPeerRequest::ListWorkflowNotificationSources {
                    protocol_version: VERSION
                }
            )
            .is_err());
    }
    #[tokio::test]
    async fn security_f9_member_cannot_refresh_profile_owner_inventory() {
        let mut f = Fixture::for_kernel("notification-home");
        let (_, runtime, _) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
        assert!(runtime
            .refresh_notification_sources_from_inventory("member", false)
            .await
            .is_err());
        assert!(f
            .store
            .notification_cached_sources("member")
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn security_f9_member_cannot_list_profile_owner_inventory() {
        let mut f = Fixture::for_kernel("notification-home");
        let (_, runtime, _) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
        let result = runtime
            .execute_workflow_notification_command(
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: f.session.clone(),
                }),
                "member",
            )
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn security_f9_standalone_member_cannot_list_session_owner_inventory() {
        let mut f = Fixture::for_kernel("notification-home");
        let (_, runtime, mut config) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
        config.cloud_relay = None;
        runtime.owned.config_projection.update(config);
        assert!(runtime
            .execute_workflow_notification_request(
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: f.session.clone()
                }),
                "member"
            )
            .is_err());
    }
    async fn relay_drill() {
        let relay_config = RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        };
        let secret = format!("fixture-{:032x}", rand::random::<u128>());
        let server = Arc::new(RelayServer::with_auth_verifier(
            relay_config,
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
        let mut source = Fixture::for_kernel("wfnotify-source");
        let (a, _, _) = source.workflow("reviewer");
        let mut target = Fixture::for_kernel("wfnotify-target");
        let (b, _, bp) = target.workflow("subscriber");
        let (sr, source_runtime, mut sc) = peer(&mut source, &url, &secret);
        let (tr, target_runtime, mut tc) = peer(&mut target, &url, &secret);
        sc.relay_token = Some(token(&sc, &secret, "local"));
        tc.relay_token = Some(token(&tc, &secret, "local"));
        source_runtime.owned.config_projection.update(sc.clone());
        target_runtime.owned.config_projection.update(tc.clone());
        let (source_tx, source_rx) = watch::channel(false);
        let (target_tx, target_rx) = watch::channel(false);
        let source_connector = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
                sr.clone(),
                source_runtime.owned.relay_state.clone(),
                source_rx,
                url.clone(),
                sc.relay_token.clone().unwrap(),
            ),
        );
        let target_connector = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
                tr.clone(),
                target_runtime.owned.relay_state.clone(),
                target_rx,
                url.clone(),
                tc.relay_token.clone().unwrap(),
            ),
        );
        registered(&registry, &sc.daemon_id).await;
        registered(&registry, &tc.daemon_id).await;
        for _ in 0..100 {
            if source_runtime.owned.relay_state.read().await.connected()
                && target_runtime.owned.relay_state.read().await.connected()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let source_record = source.source(&a);
        let response = target_runtime
            .notification_peer(
                "local",
                &sc.daemon_id,
                RelayPeerRequest::ListWorkflowNotificationSources {
                    protocol_version: VERSION,
                },
            )
            .await
            .unwrap();
        let RelayPeerResponse::WorkflowNotificationSources { sources, .. } = response else {
            panic!("wrong source list")
        };
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_id, source_record.source_id);
        target
            .store
            .notify(NotificationOperation::Cache {
                owner: "local".into(),
                kernel: sc.daemon_id.clone(),
                sources,
            })
            .unwrap();
        let presence = crate::transport::relay_discovery::get_live_kernel(&tc, &sc.daemon_id)
            .await
            .unwrap();
        target_runtime
            .owned
            .notification_inventory_projection
            .update(vec![], vec![presence]);
        target_runtime
            .refresh_notification_sources_from_inventory("local", false)
            .await
            .unwrap();
        assert!(!target.store.notification_cached_sources("local").unwrap()[0].available);
        target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        assert!(target.store.notification_cached_sources("local").unwrap()[0].available);
        let LocalDaemonResponse::WorkflowNotificationAttached { subscription } = target_runtime
            .execute_workflow_notification_command(
                LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
                    delivery_mode: crate::local::NotificationDeliveryMode::Queue,
                    session_id: target.session.clone(),
                    source_id: source_record.source_id.clone(),
                    publication_ref: bp.clone(),
                    queue_ref: None,
                    ttl_days: 7,
                    events: WorkflowNotificationEvents::Both,
                    filters: serde_json::Value::Null,
                }),
                "local",
            )
            .await
            .unwrap()
        else {
            panic!("wrong attach")
        };
        // Same-owner identity is taken from verified relay claims, never the body.
        let identity = chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "wfnotify-fixture".into(),
            subject: tc.daemon_id.clone(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms: crate::session::unix_epoch_ms() + 60_000,
            token_id: None,
            user_id: Some("other-owner".into()),
            public_key_thumbprint: None,
        };
        assert!(source_runtime.notification_peer_owner(&identity).is_none());
        let mut bad = subscription.clone();
        bad.owner_user_id = "other-owner".into();
        assert!(target_runtime
            .notification_peer(
                "local",
                &sc.daemon_id,
                RelayPeerRequest::SubscribeWorkflowNotifications {
                    protocol_version: VERSION,
                    source_workflow_ref: source_record.source_id.clone(),
                    target_ref: bad,
                }
            )
            .await
            .is_err());
        // Target accepts while paused; ACK never means a target run finished.
        target
            .sessions
            .update_workflow_prompt_queue(
                &target.session,
                &b,
                &subscription.queue_id,
                None,
                None,
                Some(false),
            )
            .unwrap();
        target.persist();
        target_runtime
            .owned
            .session_store
            .write()
            .restore_session(target.sessions.get_session(&target.session).unwrap());
        source.complete(
            &a,
            "review-complete",
            None,
            crate::session::WorkflowRunStatus::Completed,
            "MP-08 / MP-10 private fixture output",
        );
        source_runtime
            .owned
            .session_store
            .write()
            .restore_session(source.sessions.get_session(&source.session).unwrap());
        target_tx.send(true).unwrap();
        target_connector.await.unwrap();
        source_runtime.route_remote_workflow_notifications().await;
        assert_eq!(source.candidates(false).len(), 0); // delayed, still durable
        let pending = source
            .store
            .notification_candidates(false, crate::session::unix_epoch_ms() + 6000, 8)
            .unwrap();
        assert_eq!(pending.len(), 1);
        let (target_tx, target_rx) = watch::channel(false);
        let target_connector = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
                tr.clone(),
                target_runtime.owned.relay_state.clone(),
                target_rx,
                url.clone(),
                tc.relay_token.clone().unwrap(),
            ),
        );
        registered(&registry, &tc.daemon_id).await;
        for _ in 0..100 {
            if source_runtime.owned.relay_state.read().await.connected()
                && target_runtime.owned.relay_state.read().await.connected()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let envelope = pending[0].1.clone();
        // Admit once while source still has no ACK (lost response seam).
        assert!(matches!(
            source_runtime
                .notification_peer(
                    "local",
                    &tc.daemon_id,
                    RelayPeerRequest::DeliverWorkflowNotification {
                        protocol_version: VERSION,
                        subscription_id: subscription.subscription_id.clone(),
                        envelope: envelope.clone()
                    }
                )
                .await
                .unwrap(),
            RelayPeerResponse::WorkflowNotificationAccepted {
                ack: WorkflowNotificationAck::Accepted
            }
        ));
        source
            .store
            .notify(NotificationOperation::Retry {
                subscription_id: subscription.subscription_id.clone(),
                source_id: envelope.source_id.clone(),
                occurrence_id: envelope.occurrence_id.clone(),
                accepted: false,
                at: 0,
            })
            .unwrap();
        source_runtime.route_remote_workflow_notifications().await;
        assert!(source.candidates(false).is_empty());
        assert_eq!(target.candidates(true).len(), 1);
        target_runtime.owned.route_workflow_notifications();
        assert_eq!(
            target
                .store
                .notification_candidates(true, crate::session::unix_epoch_ms() + 2000, 8)
                .unwrap()
                .len(),
            1
        );
        target
            .sessions
            .update_workflow_prompt_queue(
                &target.session,
                &b,
                &subscription.queue_id,
                None,
                None,
                Some(true),
            )
            .unwrap();
        target.persist();
        target_runtime
            .owned
            .session_store
            .write()
            .restore_session(target.sessions.get_session(&target.session).unwrap());
        target
            .store
            .notify(NotificationOperation::Retry {
                subscription_id: subscription.subscription_id.clone(),
                source_id: envelope.source_id.clone(),
                occurrence_id: envelope.occurrence_id.clone(),
                accepted: true,
                at: 0,
            })
            .unwrap();
        target_runtime.owned.route_workflow_notifications();
        assert_eq!(
            target_runtime
                .owned
                .session_store
                .read()
                .get_session(&target.session)
                .unwrap()
                .workflow_queued_prompts()
                .len(),
            1
        );
        assert!(target_runtime
            .notification_peer(
                "local",
                &sc.daemon_id,
                RelayPeerRequest::ListWorkflowNotificationSources {
                    protocol_version: 70
                }
            )
            .await
            .is_err());
        let (_, sourcesubs, _) = source.store.notification_inventory("local").unwrap();
        assert_eq!(sourcesubs.len(), 1);
        source_runtime
            .execute_workflow_notification_request(
                LocalDaemonRequest::RegisterWorkflowNotificationSource(
                    RegisterWorkflowNotificationSourceRequest {
                        session_id: source.session.clone(),
                        workflow_ref: a.clone(),
                        enabled: false,
                        output_fields: None,
                    },
                ),
                "local",
            )
            .unwrap();
        // The existing connector clears discovery on disconnect. Restore the
        // relay's live presence before testing an online source's empty answer.
        let presence = crate::transport::relay_discovery::get_live_kernel(&tc, &sc.daemon_id)
            .await
            .unwrap();
        target_runtime
            .owned
            .notification_inventory_projection
            .update(vec![], vec![presence]);
        target_runtime
            .refresh_notification_sources_from_inventory("local", true)
            .await
            .unwrap();
        assert!(
            target
                .store
                .notification_cached_sources("local")
                .unwrap()
                .is_empty(),
            "MP-08 / MP-10: empty list must clear the persistent source cache"
        );
        let LocalDaemonResponse::WorkflowNotifications { subscriptions, .. } = target_runtime
            .execute_workflow_notification_request(
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: target.session.clone(),
                }),
                "local",
            )
            .unwrap()
        else {
            panic!("wrong inventory")
        };
        assert!(!subscriptions[0].source_available);
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
        assert!(source
            .store
            .notification_inventory("local")
            .unwrap()
            .1
            .is_empty());
        // MP-11 F8: authenticated, same-owner relay access does not grant
        // kernel-wide notification authority on a lease worker. These kinds
        // have no lease selector, so all four must fail at the caller boundary.
        sc.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
        source_runtime.owned.config_projection.update(sc.clone());
        for request in [
            RelayPeerRequest::ListWorkflowNotificationSources {
                protocol_version: VERSION,
            },
            RelayPeerRequest::SubscribeWorkflowNotifications {
                protocol_version: VERSION,
                source_workflow_ref: source_record.source_id.clone(),
                target_ref: subscription.clone(),
            },
            RelayPeerRequest::UnsubscribeWorkflowNotifications {
                protocol_version: VERSION,
                subscription_id: subscription.subscription_id.clone(),
            },
            RelayPeerRequest::DeliverWorkflowNotification {
                protocol_version: VERSION,
                subscription_id: subscription.subscription_id.clone(),
                envelope: envelope.clone(),
            },
        ] {
            match target_runtime
                .notification_peer("local", &sc.daemon_id, request)
                .await
            {
                Err(DaemonError::RelayTransport { code, .. }) => {
                    assert_eq!(code, "kernel_runtime_role_denied");
                }
                other => {
                    panic!("MP-11 F8: worker notification authority must be denied: {other:?}")
                }
            }
        }
        source_tx.send(true).unwrap();
        target_tx.send(true).unwrap();
        source_connector.await.unwrap();
        target_connector.await.unwrap();
        server_tx.send(()).unwrap();
        server_task.await.unwrap();
        drop(sr);
        drop(tr);
        drop(source_runtime);
        drop(target_runtime);
        cleanup(source);
        cleanup(target);
    }
}
