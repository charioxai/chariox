//! Relay socket connection lifecycle and heartbeat loop.

use super::*;

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    run_daemon_relay_connector_inner(app, state, &mut shutdown, None).await;
}

pub async fn run_daemon_relay_connector_with_static_relay(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
    relay_url: String,
    relay_token: String,
) {
    run_daemon_relay_connector_inner(
        app,
        state,
        &mut shutdown,
        Some(StaticRelayConfig {
            relay_url,
            relay_token,
        }),
    )
    .await;
}

struct StaticRelayConfig {
    relay_url: String,
    relay_token: String,
}

const CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingRelayConfigDisposition {
    LocalIdle,
    CloudUnavailable,
}

fn should_refresh_cloud_relay(config: &crate::config::DaemonConfig) -> bool {
    config.cloud_relay.is_some()
}

fn missing_relay_config_disposition(
    config: &crate::config::DaemonConfig,
) -> MissingRelayConfigDisposition {
    if config.cloud_relay.is_some() {
        MissingRelayConfigDisposition::CloudUnavailable
    } else {
        MissingRelayConfigDisposition::LocalIdle
    }
}

fn should_start_cloud_presence_publish(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn should_start_cloud_token_refresh(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn spawn_cloud_token_refresh(router: Arc<CommandRouter>, relay_url: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        match timeout(
            CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT,
            router.ensure_cloud_relay_connection(),
        )
        .await
        {
            Ok(Ok(())) => {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "cloud relay token refresh completed",
                    serde_json::json!({
                        "relay_url": relay_url,
                    }),
                );
            }
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to refresh cloud relay token",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "error": error.to_string(),
                    }),
                );
            }
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "cloud relay token refresh timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "timeout_ms": CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT.as_millis(),
                    }),
                );
            }
        }
    })
}

async fn disconnect_relay(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    reason: &str,
    publish_presence: bool,
) {
    if publish_presence {
        publish_offline_and_set_disconnected(router, state, reason).await;
    } else {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "relay socket disconnected",
            serde_json::json!({
                "reason": reason,
            }),
        );
        super::connection_state::set_disconnected(state).await;
    }
}

async fn run_daemon_relay_connector_inner(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    shutdown: &mut watch::Receiver<bool>,
    static_relay: Option<StaticRelayConfig>,
) {
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let event_runtime = match RelayEventRuntime::new(router.relay_event_counter_path()) {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to initialize relay event id allocator",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            return;
        }
    };
    let command_result_cache_path = router
        .relay_event_counter_path()
        .with_file_name("relay-command-results.jsonl");
    let command_result_cache =
        match CommandResultCache::new_with_persistent_path(command_result_cache_path) {
            Ok(cache) => Arc::new(cache),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to initialize relay command result cache",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
    let command_sequence = Arc::new(AtomicU64::new(1));
    let mut missing_relay_config_reported = false;

    loop {
        if *shutdown.borrow() {
            disconnect_relay(
                &router,
                &state,
                "shutdown before relay connect",
                static_relay.is_none(),
            )
            .await;
            return;
        }

        if static_relay.is_none() {
            let config = router.relay_config_snapshot();
            if should_refresh_cloud_relay(&config) {
                let cloud_refresh_started = Instant::now();
                match router.ensure_cloud_relay_connection().await {
                    Ok(()) => {
                        crate::logging::info_with_fields(
                            "daemon.startup",
                            "cloud relay profile hydrated",
                            serde_json::json!({
                                "refresh_ms": cloud_refresh_started.elapsed().as_millis(),
                            }),
                        );
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.relay_client",
                            "failed to refresh cloud relay token",
                            serde_json::json!({
                                "refresh_ms": cloud_refresh_started.elapsed().as_millis(),
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            }
        }

        let (relay_url, mut active_relay_token, heartbeat) = {
            let config = router.relay_config_snapshot();
            if let Some(static_relay) = static_relay.as_ref() {
                (
                    static_relay.relay_url.clone(),
                    static_relay.relay_token.clone(),
                    Duration::from_millis(config.relay_heartbeat_ms),
                )
            } else {
                match (config.relay_url.clone(), config.relay_token.clone()) {
                    (Some(relay_url), Some(relay_token)) => (
                        relay_url,
                        relay_token,
                        Duration::from_millis(config.relay_heartbeat_ms),
                    ),
                    _ => {
                        if !missing_relay_config_reported {
                            match missing_relay_config_disposition(&config) {
                                MissingRelayConfigDisposition::CloudUnavailable => {
                                    disconnect_relay(
                                        &router,
                                        &state,
                                        "relay configuration unavailable",
                                        true,
                                    )
                                    .await;
                                }
                                MissingRelayConfigDisposition::LocalIdle => {
                                    crate::logging::info_with_fields(
                                        "daemon.relay_client",
                                        "relay connector idle",
                                        serde_json::json!({
                                            "reason": "relay configuration unavailable",
                                        }),
                                    );
                                    super::connection_state::set_disconnected(&state).await;
                                }
                            }
                            missing_relay_config_reported = true;
                        } else {
                            super::connection_state::set_disconnected(&state).await;
                        }
                        let wait = sleep(Duration::from_secs(1));
                        tokio::pin!(wait);
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_ok() && *shutdown.borrow() {
                                    disconnect_relay(
                                        &router,
                                        &state,
                                        "shutdown while relay configuration unavailable",
                                        true,
                                    )
                                    .await;
                                    return;
                                }
                            }
                            _ = &mut wait => {}
                        }
                        continue;
                    }
                }
            }
        };
        missing_relay_config_reported = false;

        crate::logging::info_with_fields(
            "daemon.relay_client",
            "attempting relay connection",
            serde_json::json!({
                "relay_url": relay_url,
            }),
        );
        let connect_started = Instant::now();
        match timeout(RELAY_CONNECT_TIMEOUT, connect_async(&relay_url)).await {
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_started.elapsed().as_millis(),
                        "timeout_ms": RELAY_CONNECT_TIMEOUT.as_millis(),
                    }),
                );
                disconnect_relay(
                    &router,
                    &state,
                    "relay connect timed out",
                    static_relay.is_none(),
                )
                .await;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            Ok(Ok((socket, _))) => {
                let connect_ms = connect_started.elapsed().as_millis();
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay socket connected",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_ms,
                    }),
                );
                let registration_started = Instant::now();
                let (mut writer, mut reader) = socket.split();
                let (outgoing_tx, mut outgoing_rx) =
                    mpsc::channel::<RelayEnvelope>(RELAY_OUTGOING_QUEUE_LIMIT);
                let (writer_done_tx, mut writer_done_rx) = oneshot::channel::<()>();
                let writer_task = tokio::spawn(async move {
                    while let Some(envelope) = outgoing_rx.recv().await {
                        let payload = match serde_json::to_string(&envelope) {
                            Ok(payload) => payload,
                            Err(_) => break,
                        };
                        if writer.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    let _ = writer_done_tx.send(());
                });
                let subscription_tasks: RelaySubscriptionTasks =
                    Arc::new(Mutex::new(BTreeMap::new()));
                let daemon_id = router.relay_daemon_id();
                let mut registration = router.relay_registration().await;
                if static_relay.is_some() {
                    registration.auth_token = active_relay_token.clone();
                }
                let register = RelayEnvelope::DaemonRegister { registration };
                if send_outgoing_envelope(&outgoing_tx, register).is_err() {
                    writer_task.abort();
                    clear_remote_inventory_projection(&router);
                    disconnect_relay(
                        &router,
                        &state,
                        "failed to send relay registration",
                        static_relay.is_none(),
                    )
                    .await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay register sent",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_ms,
                        "registration_send_ms": registration_started.elapsed().as_millis(),
                        "connect_to_registration_ms": connect_started.elapsed().as_millis(),
                    }),
                );
                set_connected(&state, outgoing_tx.clone(), relay_url.clone()).await;
                let mut cloud_presence_task = None;
                if static_relay.is_none() {
                    cloud_presence_task = Some(spawn_cloud_presence_publish(
                        Arc::clone(&router),
                        true,
                        "relay registration sent",
                    ));
                }
                let mut last_cloud_presence_publish = Instant::now();
                let mut inventory_refresh_task = static_relay
                    .is_none()
                    .then(|| spawn_remote_inventory_projection_refresh(Arc::clone(&router)));
                let mut heartbeat_interval = tokio::time::interval(heartbeat);
                heartbeat_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_interval =
                    tokio::time::interval(CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL);
                token_refresh_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_task = None;
                let mut heartbeat_tick: u64 = 0;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                if static_relay.is_none() {
                                    let _ = spawn_cloud_presence_publish(
                                        Arc::clone(&router),
                                        false,
                                        "daemon shutting down",
                                    );
                                }
                                let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&router, &state, "daemon shutting down", static_relay.is_none()).await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if handle_incoming_envelope(
                                        &router,
                                        &command_sequence,
                                        &state,
                                        &outgoing_tx,
                                        &subscription_tasks,
                                        &event_runtime,
                                        &command_result_cache,
                                        &payload,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        disconnect_relay(&router, &state, "relay payload handling failed", static_relay.is_none()).await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay close frame received", static_relay.is_none()).await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay read failed or ended", static_relay.is_none()).await;
                                    break;
                                }
                            }
                        }
                        writer_done = &mut writer_done_rx => {
                            let _ = writer_done;
                            abort_inventory_refresh_task(&mut inventory_refresh_task);
                            abort_subscription_tasks(&subscription_tasks).await;
                            writer_task.abort();
                            clear_remote_inventory_projection(&router);
                            disconnect_relay(&router, &state, "relay writer ended", static_relay.is_none()).await;
                            break;
                        }
                        _ = token_refresh_interval.tick() => {
                            if static_relay.is_some() {
                                continue;
                            }
                            if router.cloud_relay_token_refresh_due()
                                && should_start_cloud_token_refresh(token_refresh_task.as_ref())
                            {
                                token_refresh_task = Some(spawn_cloud_token_refresh(
                                    Arc::clone(&router),
                                    relay_url.clone(),
                                ));
                            }
                            match relay_config_continuity(
                                &relay_url,
                                &active_relay_token,
                                &router.relay_config_snapshot(),
                            ) {
                                RelayConfigContinuity::Continue => {}
                                RelayConfigContinuity::TokenRotated(next_token) => {
                                    active_relay_token = next_token;
                                    crate::logging::info_with_fields(
                                        "daemon.relay_client",
                                        "relay token rotated on active socket",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                        }),
                                    );
                                }
                                RelayConfigContinuity::Reconnect(reason) => {
                                    crate::logging::warn_with_fields(
                                        "daemon.relay_client",
                                        "relay socket reconnect requested",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                            "reason": reason,
                                            "phase": "token_refresh",
                                        }),
                                    );
                                    let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                        reason: "relay configuration changed".to_string(),
                                    });
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay configuration changed", true).await;
                                    break;
                                }
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            heartbeat_tick = heartbeat_tick.wrapping_add(1);
                            if static_relay.is_none() {
                                match relay_config_continuity(
                                    &relay_url,
                                    &active_relay_token,
                                    &router.relay_config_snapshot(),
                                ) {
                                    RelayConfigContinuity::Continue => {}
                                    RelayConfigContinuity::TokenRotated(next_token) => {
                                        active_relay_token = next_token;
                                        crate::logging::info_with_fields(
                                            "daemon.relay_client",
                                            "relay token rotated on active socket",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                            }),
                                        );
                                    }
                                    RelayConfigContinuity::Reconnect(reason) => {
                                        crate::logging::warn_with_fields(
                                            "daemon.relay_client",
                                            "relay socket reconnect requested",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                                "reason": reason,
                                                "phase": "heartbeat",
                                            }),
                                        );
                                        let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                            reason: "relay configuration changed".to_string(),
                                        });
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        disconnect_relay(&router, &state, "relay configuration changed", true).await;
                                        break;
                                    }
                                }
                            }
                            if static_relay.is_none()
                                && heartbeat_tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS)
                                && inventory_refresh_task
                                    .as_ref()
                                    .is_none_or(|task| task.is_finished())
                            {
                                inventory_refresh_task = Some(
                                    spawn_remote_inventory_projection_refresh(
                                        Arc::clone(&router),
                                    )
                                );
                            }
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                                registration: None,
                            };
                            if send_outgoing_envelope(&outgoing_tx, heartbeat_frame).is_err() {
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&router, &state, "relay heartbeat send failed", static_relay.is_none()).await;
                                break;
                            }
                            let _ = timeout(
                                RELAY_HEARTBEAT_APP_WORK_TIMEOUT,
                                pump_leased_projection_events(&router, &outgoing_tx),
                            )
                            .await;
                            if last_cloud_presence_publish.elapsed()
                                >= CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL
                            {
                                if static_relay.is_none()
                                    && should_start_cloud_presence_publish(
                                        cloud_presence_task.as_ref(),
                                    )
                                {
                                    cloud_presence_task = Some(spawn_cloud_presence_publish(
                                        Arc::clone(&router),
                                        true,
                                        "relay heartbeat",
                                    ));
                                    last_cloud_presence_publish = Instant::now();
                                }
                            }
                        }
                    }
                }
            }
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect failed",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_started.elapsed().as_millis(),
                        "error": error.to_string(),
                    }),
                );
                clear_remote_inventory_projection(&router);
                disconnect_relay(
                    &router,
                    &state,
                    "relay socket connect failed",
                    static_relay.is_none(),
                )
                .await;
                let reconnect_delay = sleep(Duration::from_secs(1));
                tokio::pin!(reconnect_delay);
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_ok() && *shutdown.borrow() {
                            return;
                        }
                    }
                    _ = &mut reconnect_delay => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_relay_config_without_cloud_profile_is_local_idle() {
        let config = crate::config::DaemonConfig::for_tests();

        assert!(!should_refresh_cloud_relay(&config));
        assert_eq!(
            missing_relay_config_disposition(&config),
            MissingRelayConfigDisposition::LocalIdle
        );
    }

    #[test]
    fn missing_relay_config_with_cloud_profile_is_cloud_unavailable() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "account".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: None,
            machine_alias: None,
            machine_credential: Some("machine-credential".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });

        assert!(should_refresh_cloud_relay(&config));
        assert_eq!(
            missing_relay_config_disposition(&config),
            MissingRelayConfigDisposition::CloudUnavailable
        );
    }

    #[tokio::test]
    async fn cloud_presence_publish_gate_skips_running_task() {
        let (_tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = rx.await;
        });

        assert!(!should_start_cloud_presence_publish(Some(&task)));

        task.abort();
    }

    #[tokio::test]
    async fn cloud_presence_publish_gate_allows_finished_or_missing_task() {
        let task = tokio::spawn(async {});
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }

        assert!(should_start_cloud_presence_publish(None));
        assert!(should_start_cloud_presence_publish(Some(&task)));
    }

    #[tokio::test]
    async fn cloud_token_refresh_gate_skips_running_task() {
        let (_tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = rx.await;
        });

        assert!(!should_start_cloud_token_refresh(Some(&task)));

        task.abort();
    }

    #[tokio::test]
    async fn cloud_token_refresh_gate_allows_finished_or_missing_task() {
        let task = tokio::spawn(async {});
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }

        assert!(should_start_cloud_token_refresh(None));
        assert!(should_start_cloud_token_refresh(Some(&task)));
    }
}
