//! MD-DISPLAY-04: caller-bound registration on the existing relay event route.
use super::subscriptions::{RelaySubscriptionTask, RelaySubscriptionTasks};
use super::*;
use chariox_relay::protocol::RelayCallerIdentity;
// MD-DISPLAY-04: delivery registration only; policy stays in the host service.
pub(super) struct DisplaySubscription {
    pub(super) request_id: String,
    pub(super) relay_id: String,
    pub(super) display_id: String,
    pub(super) generation: String,
    pub(super) identity: Option<RelayCallerIdentity>,
    pub(super) public_key: String,
    pub(super) resume: Option<u64>,
    pub(super) recover: bool,
}

// Room admission makes a peer round trip. Queue it in the connection's existing
// subscription registry so its reader can receive that response; unsubscribe and
// connection teardown can cancel an admission before it registers delivery.
pub(super) async fn enqueue_subscribe(
    router: &Arc<CommandRouter>,
    outgoing: &RelayOutgoingSender,
    tasks: &RelaySubscriptionTasks,
    events: &Arc<RelayEventRuntime>,
    subscription: DisplaySubscription,
) -> Result<(), DaemonError> {
    let router = Arc::clone(router);
    let writer = outgoing.clone();
    let owned_tasks = Arc::clone(tasks);
    let events = Arc::clone(events);
    let request_id = subscription.request_id.clone();
    let queued = super::display_admission::enqueue(
        tasks,
        subscription.relay_id.clone(),
        subscription.public_key.clone(),
        async move {
            if super::subscriptions::handle_relay_subscribe(
                &router,
                &writer,
                &owned_tasks,
                &events,
                subscription.request_id,
                subscription.relay_id,
                subscription.display_id,
                subscription.generation,
                subscription.identity,
                subscription.public_key,
                Some(
                    if subscription.recover {
                        chariox_relay::frame_transport::DISPLAY_RESUME_SCOPE
                    } else {
                        chariox_relay::frame_transport::DISPLAY_EVENT_SCOPE
                    }
                    .into(),
                ),
                subscription.resume,
                None,
            )
            .await
            .is_err()
            {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "display admission response could not be delivered",
                    serde_json::json!({}),
                );
            }
        },
    )
    .await;
    if !queued {
        return send_outgoing_envelope(
            outgoing,
            RelayEnvelope::DaemonResponse {
                relay_request_id: request_id,
                encrypted_response: None,
                error: Some(super::request_errors::map_relay_error(
                    &DaemonError::LocalTransport {
                        operation: "kernel_browser",
                        message: "MD-DISPLAY: display admission is already pending or at capacity"
                            .into(),
                    },
                )),
            },
        );
    }
    Ok(())
}

pub(super) async fn handle_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    tasks: &RelaySubscriptionTasks,
    subscription: DisplaySubscription,
) -> Result<(), DaemonError> {
    let DisplaySubscription {
        request_id,
        relay_id,
        display_id,
        generation,
        identity,
        public_key,
        resume,
        recover,
    } = subscription;
    use crate::runtime::command::{KernelCommand, KernelCommandSource};
    let request =
        crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::State,
        });
    let subscription_caller = match display_subscription_caller(router, identity, &public_key) {
        Ok(caller) => caller,
        Err(error) => {
            // MP-08/MP-10/MP-11: a revoked grant or failed pairing read rejects
            // this subscription only; it must not abort every relay subscriber.
            let mut error = super::request_errors::map_relay_error(&error);
            error.retryable = false;
            return send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id: request_id,
                    encrypted_response: None,
                    error: Some(error),
                },
            );
        }
    };
    let caller = KernelCommand::from_local_request_with_caller(
        request_id.clone(),
        KernelCommandSource::RelayClient,
        subscription_caller,
        None,
        None,
        &request,
    );
    let admission = match generation.parse::<u64>() {
        Ok(generation) if resume.is_none() => {
            if recover {
                router
                    .runtime_state()
                    .kernel_browser_display_resume_attach(&caller, display_id.clone(), generation)
                    .await
            } else {
                router
                    .runtime_state()
                    .kernel_browser_display_attach(&caller, display_id.clone(), generation)
                    .await
            }
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "kernel_browser",
            message: "MD-DISPLAY: fresh generation-bound subscription required".into(),
        }),
    };
    let binding = match admission {
        Ok(binding) => binding,
        Err(error) => {
            return send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id: request_id,
                    encrypted_response: None,
                    error: Some(super::request_errors::map_relay_error(&error)),
                },
            )
        }
    };
    let initial_credit = registration_credit(recover, &binding);
    let (display_id, generation) = if recover {
        match resumed_delivery(&binding, &display_id) {
            Some(delivery) => delivery,
            None => {
                // MP-11: answer the viewer and release whatever the attach just bound;
                // a dropped registration would hang the viewer on a rotated lease.
                if let Some((subscription_id, generation)) =
                    rejected_delivery_release(&binding, &display_id, &generation)
                {
                    let release = crate::local::KernelBrowserCommand::Unsubscribe {
                        subscription_id,
                        generation,
                    };
                    let _ = router
                        .runtime_state()
                        .kernel_browser_terminal_request(&caller, release)
                        .await;
                }
                let error = DaemonError::LocalTransport {
                    operation: "kernel_browser",
                    message: "MP-11: invalid resumed delivery binding".into(),
                };
                return send_outgoing_envelope(
                    outgoing_tx,
                    RelayEnvelope::DaemonResponse {
                        relay_request_id: request_id,
                        encrypted_response: None,
                        error: Some(super::request_errors::map_relay_error(&error)),
                    },
                );
            }
        }
    } else {
        (display_id, generation.parse::<u64>().unwrap_or_default())
    };
    let mut guard = tasks.lock().await;
    guard.retain(|_, task| task.display_id.is_none() || !task.handle.is_finished());
    let key = format!("display:{display_id}:{public_key}");
    if let Some(old) = guard.remove(&key) {
        old.handle.abort();
        if let Some(pump) = &old.display_pump {
            pump.stop();
        }
    }
    // MP-08/MP-10: protocol 491 push pump, dormant until the first ack (or granted at registration).
    let pump = super::display_pump::DisplayPump::new(generation, &display_id);
    guard.insert(
        key.clone(),
        RelaySubscriptionTask {
            display_id: Some(display_id.clone()),
            display_pump: Some(pump.clone()),
            relay_subscription_id: relay_id.clone(),
            client_public_key: public_key.clone(),
            handle: tokio::spawn(sleep(Duration::from_secs(60))),
        },
    );
    drop(guard);
    let request =
        crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::DisplayNext {
                subscription_id: display_id.clone(),
                generation,
                after_sequence: 0,
            },
        });
    let pump_caller = KernelCommand::from_local_request_with_caller(
        format!("{request_id}-push"),
        KernelCommandSource::RelayClient,
        caller.caller.clone(),
        None,
        None,
        &request,
    );
    // Queue the authenticated binding response before granting initial credit.
    // The relay commits its event route on that response; first frames cannot race it.
    let mut response = serde_json::json!({"ok":true,"display_attached":true});
    if recover {
        response["binding"] = binding;
    }
    let encrypted = encrypt_json_response(router, &public_key, response).await?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonResponse {
            relay_request_id: request_id,
            encrypted_response: Some(encrypted),
            error: None,
        },
    )?;
    if initial_credit {
        pump.acknowledge(generation, 0, true, None);
    }
    let delivery_key = key.clone();
    let delivery_relay = relay_id.clone();
    let delivery_viewer = public_key.clone();
    super::display_delivery::start(
        tasks,
        &delivery_key,
        &delivery_relay,
        &delivery_viewer,
        super::display_pump::run(
            super::display_pump::PumpRoute {
                router: router.clone(),
                outgoing_tx: outgoing_tx.clone(),
                tasks: tasks.clone(),
                key,
                caller: pump_caller,
                display_id,
                relay_id,
                public_key,
            },
            pump,
        ),
    )
    .await;
    Ok(())
}

pub(super) async fn browser_display_delivery_id(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
) -> Option<String> {
    let mut guard = tasks.lock().await;
    let task = guard.values_mut().find(|task| {
        task.display_id.as_deref() == Some(display_id)
            && task.client_public_key == public_key
            && !task.handle.is_finished()
    })?;
    task.handle.abort();
    task.handle = tokio::spawn(sleep(Duration::from_secs(60)));
    Some(task.relay_subscription_id.clone())
}

/// MP-08/MP-10: protocol 491 acknowledgement from the subscription's own
/// sender key; renews its delivery lease and reports the pump state.
pub(super) async fn acknowledge(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
    generation: u64,
    sequence: u64,
    lost: bool,
    held_ms: Option<u32>,
) -> Option<&'static str> {
    let pump = {
        let guard = tasks.lock().await;
        guard
            .values()
            .find(|task| {
                task.display_id.as_deref() == Some(display_id)
                    && task.client_public_key == public_key
                    && !task.handle.is_finished()
            })?
            .display_pump
            .clone()?
    };
    let status = pump.acknowledge(generation, sequence, lost, held_ms)?;
    browser_display_delivery_id(tasks, display_id, public_key).await?;
    Some(status)
}

pub(super) async fn refresh_admitted_display_poll(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
) -> bool {
    browser_display_delivery_id(tasks, display_id, public_key)
        .await
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn md_display_static_polling_renews_delivery_beyond_sixty_seconds() {
        let tasks = Arc::new(Mutex::new(std::collections::BTreeMap::from([(
            "display".into(),
            RelaySubscriptionTask {
                display_id: Some("d".into()),
                display_pump: None,
                relay_subscription_id: "relay".into(),
                client_public_key: "key".into(),
                handle: tokio::spawn(sleep(Duration::from_secs(60))),
            },
        )])));
        tokio::task::yield_now().await;
        for _ in 0..4 {
            tokio::time::advance(Duration::from_secs(20)).await;
            assert!(refresh_admitted_display_poll(&tasks, "d", "key").await);
            tokio::task::yield_now().await;
        }
        assert_eq!(
            browser_display_delivery_id(&tasks, "d", "key")
                .await
                .as_deref(),
            Some("relay")
        );
        assert!(browser_display_delivery_id(&tasks, "d", "foreign")
            .await
            .is_none());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(browser_display_delivery_id(&tasks, "d", "key")
            .await
            .is_none());
    }
}

/// MP-08/MP-10: a 535 resume, and a 540 viewer that holds frames pushed before its registration
/// response (PERF_AUDIT 4.2.3), get their first frame at registration; others on their first ACK.
fn registration_credit(recover: bool, binding: &serde_json::Value) -> bool {
    recover
        || binding
            .get("initial_push")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

// MP-08/MP-10/MP-11: metadata must retain the RPC's terminal actor.
/// MP-11: a resumed (535) delivery is a new subscription id with its generation, marked resumed.
fn resumed_delivery(binding: &serde_json::Value, previous: &str) -> Option<(String, u64)> {
    let id = binding.get("subscription_id")?.as_str()?;
    let generation = binding.get("generation")?.as_u64()?;
    (id != previous && binding.get("resumed")?.as_bool()?).then(|| (id.to_string(), generation))
}

/// The delivery a rejected resume binding must release: its own id/generation where present,
/// else the pre-attach id and the generation the attach was admitted under.
fn rejected_delivery_release(
    binding: &serde_json::Value,
    previous: &str,
    generation: &str,
) -> Option<(String, u64)> {
    let id = binding
        .get("subscription_id")
        .and_then(|v| v.as_str())
        .unwrap_or(previous);
    let generation = binding
        .get("generation")
        .and_then(|v| v.as_u64())
        .or_else(|| generation.parse().ok())?;
    Some((id.to_string(), generation))
}

fn display_subscription_caller(
    router: &CommandRouter,
    identity: Option<RelayCallerIdentity>,
    public_key: &str,
) -> Result<crate::runtime::command::KernelCaller, DaemonError> {
    use crate::runtime::command::KernelCaller;
    Ok(match router.admit_self_host_terminal(public_key)? {
        Some(grant) => KernelCaller::from_paired_terminal(grant),
        None => KernelCaller::for_relay_request(identity),
    })
}

#[cfg(test)]
mod resumed_delivery_tests {
    use super::{registration_credit, rejected_delivery_release, resumed_delivery};
    use serde_json::json;
    // PERF_AUDIT 4.2.3: the viewer that asked for it is not left waiting one round trip for its first frame.
    #[test]
    fn mp08_mp10_an_initial_push_viewer_is_credited_at_registration() {
        assert!(registration_credit(true, &json!({})));
        assert!(registration_credit(
            false,
            &json!({"attached": true, "initial_push": true})
        ));
        assert!(!registration_credit(false, &json!({"attached": true})));
        assert!(!registration_credit(
            false,
            &json!({"attached": true, "initial_push": "yes"})
        ));
    }
    // Review #1016 @52a7bb1ee P3: a rejected binding without a numeric generation (or id) is
    // still released, with the pre-attach id/generation the attach was admitted under.
    #[test]
    fn mp11_a_rejected_resumed_binding_is_always_released() {
        let release = |binding, generation| rejected_delivery_release(&binding, "s1", generation);
        assert_eq!(
            release(json!({"subscription_id": "s2", "generation": 7}), "6"),
            Some(("s2".into(), 7))
        );
        assert_eq!(
            release(json!({"subscription_id": "s2"}), "6"),
            Some(("s2".into(), 6))
        );
        assert_eq!(
            release(json!({"subscription_id": "s2", "generation": "7"}), "6"),
            Some(("s2".into(), 6))
        );
        assert_eq!(
            release(json!({"generation": 7}), "6"),
            Some(("s1".into(), 7))
        );
        assert_eq!(release(json!(null), "6"), Some(("s1".into(), 6)));
        assert_eq!(
            release(json!({"subscription_id": 3}), "x"),
            None,
            "no generation at all"
        );
    }
    #[test]
    fn mp11_resumed_delivery_requires_a_new_marked_generation_bound_id() {
        let ok = json!({"subscription_id": "s2", "generation": 7, "resumed": true});
        assert_eq!(resumed_delivery(&ok, "s1"), Some(("s2".into(), 7)));
        // Review #1016 @57b49eac1 P3: RoomDisplayBinding.resumed is #[serde(default)].
        assert_eq!(
            resumed_delivery(&json!({"subscription_id": "s2", "generation": 7}), "s1"),
            None
        );
        assert_eq!(
            resumed_delivery(
                &json!({"subscription_id": "s2", "generation": 7, "resumed": false}),
                "s1"
            ),
            None
        );
        assert_eq!(
            resumed_delivery(
                &json!({"subscription_id": "s1", "generation": 7, "resumed": true}),
                "s1"
            ),
            None
        );
        assert_eq!(
            resumed_delivery(&json!({"subscription_id": "s2", "resumed": true}), "s1"),
            None
        );
    }
}

#[cfg(test)]
mod paired_tests {
    use super::*;
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    #[tokio::test]
    async fn mp08_mp10_mp11_paired_display_metadata_keeps_rpc_actor() {
        let work = crate::test_support::TestWorktree::new("paired-display-actor");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config_path = work.path().join("config.toml");
        config.relay_url = Some("ws://127.0.0.1:1".into());
        config.relay_token = Some("test-transport".into());
        config.user_config.state.path = Some(work.path().join("state.db").display().to_string());
        let private = crate::transport::relay_crypto::generate_private_key_base64();
        let public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&private).unwrap();
        let grant = crate::config::PersistedClientPairing {
            client_id: "paired-web".into(),
            alias: None,
            terminal_type: "web".into(),
            public_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &public,
            ),
            paired_at_ms: crate::session::unix_epoch_ms(),
            revoked: false,
        };
        crate::runtime::self_host_terminal_grants::register(
            &config,
            "pairing-link",
            &grant.client_id,
            crate::session::unix_epoch_ms() + 60_000,
            "web",
            None,
        )
        .unwrap();
        crate::runtime::self_host_terminal_grants::redeem(&config, "pairing-link", grant.clone())
            .unwrap();
        let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let request =
            crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                command: crate::local::KernelBrowserCommand::State,
            });
        // This is the RPC's pre-existing paired admission contract.
        let mut rpc = KernelCaller::for_relay_request(None);
        rpc.caller_id = grant.client_id.clone();
        rpc.client_id = Some(grant.client_id.clone());
        rpc.user_id = Some(crate::session::DEFAULT_LOCAL_USER_ID.into());
        rpc.public_key_thumbprint = Some(grant.public_key_thumbprint.clone());
        rpc.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
        let command = |caller| {
            KernelCommand::from_local_request_with_caller(
                "paired",
                KernelCommandSource::RelayClient,
                caller,
                None,
                None,
                &request,
            )
        };
        let rpc_actor = router
            .runtime_state()
            .kernel_browser_terminal_context(&command(rpc))
            .unwrap();
        let display = display_subscription_caller(&router, None, &public).unwrap();
        assert_eq!(
            router
                .runtime_state()
                .kernel_browser_terminal_context(&command(display))
                .expect("MP-08 P2: paired metadata is an authenticated terminal"),
            rpc_actor
        );
        crate::runtime::self_host_terminal_grants::revoke(&config, &grant.client_id).unwrap();
        assert!(
            display_subscription_caller(&router, None, &public).is_err(),
            "MP-11: revoked keys never acquire a display caller"
        );
        // MP-08/MP-10/MP-11: revocation between outer admission and the display
        // lookup is one definitive subscription failure, never a connection error.
        let tasks = Arc::new(Mutex::new(BTreeMap::from([(
            "unrelated".into(),
            RelaySubscriptionTask {
                relay_subscription_id: "other-relay".into(),
                client_public_key: "other-key".into(),
                display_id: None,
                display_pump: None,
                handle: tokio::spawn(sleep(Duration::from_secs(60))),
            },
        )])));
        let (outgoing, mut replies, _events) = RelayOutgoingSender::channel(4);
        let router = Arc::new(router);
        let result = handle_subscribe(
            &router,
            &outgoing,
            &tasks,
            DisplaySubscription {
                request_id: "revoked-display".into(),
                relay_id: "display-relay".into(),
                display_id: "display".into(),
                generation: "1".into(),
                identity: None,
                public_key: public,
                resume: None,
                recover: false,
            },
        )
        .await;
        assert!(
            result.is_ok(),
            "MP-11: pairing error must not drop relay: {result:?}"
        );
        let Some(RelayEnvelope::DaemonResponse {
            relay_request_id,
            error: Some(error),
            ..
        }) = replies.recv().await
        else {
            panic!("display admission error response")
        };
        assert_eq!(relay_request_id, "revoked-display");
        assert!(
            !error.retryable,
            "MP-11: a rejected display subscription is definitive"
        );
        tokio::task::yield_now().await;
        {
            let guard = tasks.lock().await;
            assert_eq!(guard.len(), 1);
            assert!(!guard["unrelated"].handle.is_finished());
        }
        assert!(replies.try_recv().is_err());
        super::super::subscriptions::abort_subscription_tasks(&router, &tasks).await;
        router.runtime_state().shutdown_cleanup().await.unwrap();
    }
}
