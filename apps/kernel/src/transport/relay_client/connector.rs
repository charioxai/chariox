//! Relay socket connection lifecycle and heartbeat loop.

use super::*;

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
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
    let command_sequence = Arc::new(AtomicU64::new(1));

    loop {
        if *shutdown.borrow() {
            publish_offline_and_set_disconnected(&router, &state, "shutdown before relay connect")
                .await;
            return;
        }

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

        let (relay_url, mut active_relay_token, heartbeat) = {
            let config = router.relay_config_snapshot();
            match (config.relay_url.clone(), config.relay_token.clone()) {
                (Some(relay_url), Some(relay_token)) => (
                    relay_url,
                    relay_token,
                    Duration::from_millis(config.relay_heartbeat_ms),
                ),
                _ => {
                    publish_offline_and_set_disconnected(
                        &router,
                        &state,
                        "relay configuration unavailable",
                    )
                    .await;
                    let wait = sleep(Duration::from_secs(1));
                    tokio::pin!(wait);
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                publish_offline_and_set_disconnected(
                                    &router,
                                    &state,
                                    "shutdown while relay configuration unavailable",
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
        };

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
                publish_offline_and_set_disconnected(&router, &state, "relay connect timed out")
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
                let register = RelayEnvelope::DaemonRegister {
                    registration: router.relay_registration().await,
                };
                if send_outgoing_envelope(&outgoing_tx, register).is_err() {
                    writer_task.abort();
                    clear_remote_inventory_projection(&router);
                    publish_offline_and_set_disconnected(
                        &router,
                        &state,
                        "failed to send relay registration",
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
                set_connected(&state, outgoing_tx.clone()).await;
                publish_cloud_presence(&router, true, "relay registration sent").await;
                let mut last_cloud_presence_publish = Instant::now();
                let mut inventory_refresh_task = Some(spawn_remote_inventory_projection_refresh(
                    Arc::clone(&router),
                ));
                let mut heartbeat_interval = tokio::time::interval(heartbeat);
                heartbeat_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_interval =
                    tokio::time::interval(CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL);
                token_refresh_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut heartbeat_tick: u64 = 0;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                publish_cloud_presence(&router, false, "daemon shutting down").await;
                                let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                publish_offline_and_set_disconnected(&router, &state, "daemon shutting down").await;
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
                                        &payload,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        publish_offline_and_set_disconnected(&router, &state, "relay payload handling failed").await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    publish_offline_and_set_disconnected(&router, &state, "relay close frame received").await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    publish_offline_and_set_disconnected(&router, &state, "relay read failed or ended").await;
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
                            publish_offline_and_set_disconnected(&router, &state, "relay writer ended").await;
                            break;
                        }
                        _ = token_refresh_interval.tick() => {
                            if router.cloud_relay_token_refresh_due() {
                                if let Err(error) = router.ensure_cloud_relay_connection().await {
                                    crate::logging::warn_with_fields(
                                        "daemon.relay_client",
                                        "failed to refresh cloud relay token",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                            "error": error.to_string(),
                                        }),
                                    );
                                }
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
                                    publish_offline_and_set_disconnected(&router, &state, "relay configuration changed").await;
                                    break;
                                }
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            heartbeat_tick = heartbeat_tick.wrapping_add(1);
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
                                    publish_offline_and_set_disconnected(&router, &state, "relay configuration changed").await;
                                    break;
                                }
                            }
                            if heartbeat_tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS)
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
                                publish_offline_and_set_disconnected(&router, &state, "relay heartbeat send failed").await;
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
                                publish_cloud_presence(&router, true, "relay heartbeat").await;
                                last_cloud_presence_publish = Instant::now();
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
                publish_offline_and_set_disconnected(
                    &router,
                    &state,
                    "relay socket connect failed",
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
