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
    let key = format!("display:{display_id}:{public_key}");
    // MP-08/MP-10: protocol 491 push pump, dormant until the first ack (or granted at registration).
    // MP-08 TR-16: a viewer registering again (its lanes moved carrier) keeps the pump serving it.
    let delivery = register_delivery(
        super::display_pump::PumpRoute {
            router: router.clone(),
            outgoing_tx: outgoing_tx.clone(),
            tasks: tasks.clone(),
            key: key.clone(),
            caller: pump_caller,
            display_id,
            relay_id: relay_id.clone(),
            public_key: public_key.clone(),
        },
        generation,
        !recover,
    )
    .await;
    let pump = delivery.pump().clone();
    // Queue the authenticated binding response before granting initial credit.
    // The relay commits its event route on that response; first frames cannot race it.
    let mut response = serde_json::json!({"ok":true,"display_attached":true});
    if recover {
        response["binding"] = binding;
    }
    let sent = match encrypt_json_response(router, &public_key, response).await {
        Ok(encrypted) => send_outgoing_envelope(
            outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: request_id,
                encrypted_response: Some(encrypted),
                error: None,
            },
        ),
        Err(error) => Err(error),
    };
    if let Err(error) = sent {
        pump.stop();
        return Err(error);
    }
    pump.release();
    match delivery {
        Delivery::Moved { pump, left } => {
            if let Some(left) = left.filter(|left| !Arc::ptr_eq(&left.tasks, tasks)) {
                forget_moved_delivery(&left.tasks, &pump).await;
            }
        }
        Delivery::Fresh(pump) => {
            if initial_credit {
                pump.acknowledge(generation, 0, true, None, false);
            }
            super::display_delivery::start(
                tasks,
                &key,
                &relay_id,
                &public_key,
                super::display_pump::run(pump),
            )
            .await;
        }
    }
    Ok(())
}

/// What a display registration delivers on.
enum Delivery {
    /// The viewer's running pump, moved from the carrier it left (`left`).
    Moved {
        pump: Arc<super::display_pump::DisplayPump>,
        left: Option<Arc<super::display_pump::PumpRoute>>,
    },
    /// A new pump, which starts on an independent frame.
    Fresh(Arc<super::display_pump::DisplayPump>),
}

impl Delivery {
    fn pump(&self) -> &Arc<super::display_pump::DisplayPump> {
        match self {
            Self::Moved { pump, .. } | Self::Fresh(pump) => pump,
        }
    }
}

/// Registers the viewer display `route.key` on `route`'s carrier, its frames held until the
/// registration response is queued. With `takeover`, the same viewer key at the same generation
/// moves its running pump here; otherwise (another viewer, another generation, a 535 resume, or
/// a pump that already ended) a fresh pump replaces whatever fed that key.
async fn register_delivery(
    route: super::display_pump::PumpRoute,
    generation: u64,
    takeover: bool,
) -> Delivery {
    let route = Arc::new(route);
    let moved = takeover
        .then(|| super::display_pump::live(&route.key, generation))
        .flatten()
        .and_then(|pump| {
            let left = pump.claim(route.clone())?;
            Some(Delivery::Moved { pump, left })
        });
    let delivery = moved.unwrap_or_else(|| {
        let pump = super::display_pump::DisplayPump::new(generation, &route.display_id);
        super::display_pump::supersede(&route.key, &pump);
        pump.claim(route.clone());
        Delivery::Fresh(pump)
    });
    let mut guard = route.tasks.lock().await;
    guard.retain(|_, task| task.display_id.is_none() || !task.handle.is_finished());
    if let Some(old) = guard.remove(&route.key) {
        old.handle.abort();
        if let Some(pump) = old
            .display_pump
            .filter(|pump| !Arc::ptr_eq(pump, delivery.pump()))
        {
            pump.stop();
        }
    }
    guard.insert(
        route.key.clone(),
        RelaySubscriptionTask {
            display_id: Some(route.display_id.clone()),
            display_pump: Some(delivery.pump().clone()),
            relay_subscription_id: route.relay_id.clone(),
            client_public_key: route.public_key.clone(),
            handle: tokio::spawn(sleep(Duration::from_secs(60))),
        },
    );
    delivery
}

/// The carrier a pump moved away from no longer holds its delivery lease.
async fn forget_moved_delivery(
    tasks: &RelaySubscriptionTasks,
    pump: &Arc<super::display_pump::DisplayPump>,
) {
    tasks.lock().await.retain(|_, task| {
        let moved = task
            .display_pump
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, pump));
        if moved {
            task.handle.abort();
        }
        !moved
    });
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

/// What a `display_ack` reports.
#[derive(Clone, Copy)]
pub(super) struct AckReport {
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) lost: bool,
    pub(super) held_ms: Option<u32>,
    pub(super) paused: bool,
}

/// MP-08/MP-10: protocol 491 acknowledgement from the subscription's own
/// sender key; renews its delivery lease and reports the pump state.
pub(super) async fn acknowledge(
    tasks: &RelaySubscriptionTasks,
    display_id: &str,
    public_key: &str,
    report: AckReport,
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
    let status = pump.acknowledge(
        report.generation,
        report.sequence,
        report.lost,
        report.held_ms,
        report.paused,
    )?;
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

    #[tokio::test(start_paused = true)]
    async fn mp08_a_paused_display_keeps_its_delivery_for_minutes_and_resumes_on_the_same_pump() {
        let pump = super::super::display_pump::DisplayPump::new(7, "d");
        let tasks = Arc::new(Mutex::new(std::collections::BTreeMap::from([(
            "display".into(),
            RelaySubscriptionTask {
                display_id: Some("d".into()),
                display_pump: Some(pump.clone()),
                relay_subscription_id: "relay".into(),
                client_public_key: "key".into(),
                handle: tokio::spawn(sleep(Duration::from_secs(60))),
            },
        )])));
        tokio::task::yield_now().await;
        // Hidden for five minutes: a paused acknowledgement every 20 s is the only traffic.
        for _ in 0..15 {
            tokio::time::advance(Duration::from_secs(20)).await;
            assert_eq!(
                acknowledge(
                    &tasks,
                    "d",
                    "key",
                    AckReport {
                        generation: 7,
                        sequence: 3,
                        lost: false,
                        held_ms: None,
                        paused: true
                    }
                )
                .await,
                Some("running")
            );
            tokio::task::yield_now().await;
        }
        // Watched again: the independent-frame request reaches the same registration.
        assert_eq!(
            acknowledge(
                &tasks,
                "d",
                "key",
                AckReport {
                    generation: 7,
                    sequence: 3,
                    lost: true,
                    held_ms: None,
                    paused: false
                }
            )
            .await,
            Some("running")
        );
        // Silence alone still ends the delivery after sixty seconds.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(acknowledge(
            &tasks,
            "d",
            "key",
            AckReport {
                generation: 7,
                sequence: 4,
                lost: false,
                held_ms: None,
                paused: false
            }
        )
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
mod pump_move_tests {
    use super::super::display_pump::PumpRoute;
    use super::*;
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    use std::time::Instant;

    struct Carrier {
        tasks: RelaySubscriptionTasks,
        outgoing: RelayOutgoingSender,
        frames: Option<tokio::sync::mpsc::Receiver<RelayEnvelope>>,
        _events: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    }

    fn carrier() -> Carrier {
        let (outgoing, frames, events) = RelayOutgoingSender::channel(64);
        Carrier {
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            outgoing,
            frames: Some(frames),
            _events: events,
        }
    }

    fn test_router(name: &str) -> (Arc<CommandRouter>, crate::test_support::TestWorktree) {
        let work = crate::test_support::TestWorktree::new(name);
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config_path = work.path().join("config.toml");
        config.user_config.state.path = Some(work.path().join("state.db").display().to_string());
        let app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        (Arc::new(router), work)
    }

    fn route(
        router: &Arc<CommandRouter>,
        carrier: &Carrier,
        display: &str,
        lane: &str,
        viewer: &str,
    ) -> PumpRoute {
        let request =
            crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                command: crate::local::KernelBrowserCommand::State,
            });
        PumpRoute {
            router: router.clone(),
            outgoing_tx: carrier.outgoing.clone(),
            tasks: carrier.tasks.clone(),
            key: format!("display:{display}:{viewer}"),
            caller: KernelCommand::from_local_request_with_caller(
                "push",
                KernelCommandSource::RelayClient,
                KernelCaller::for_relay_request(None),
                None,
                None,
                &request,
            ),
            display_id: display.into(),
            relay_id: lane.into(),
            public_key: viewer.into(),
        }
    }

    fn frame() -> chariox_relay::protocol::EncryptedRelayPayload {
        chariox_relay::protocol::EncryptedRelayPayload {
            sender_public_key: "kernel".into(),
            nonce: "nonce".into(),
            ciphertext: "frame".into(),
        }
    }

    /// Every frame a carrier delivers: (sequence, lane, arrival).
    fn collect(carrier: &mut Carrier) -> tokio::task::JoinHandle<Vec<(u64, String, Instant)>> {
        let mut frames = carrier.frames.take().unwrap();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(RelayEnvelope::DaemonEvent {
                subscription_id,
                event_id,
                ..
            }) = frames.recv().await
            {
                seen.push((event_id, subscription_id, Instant::now()));
            }
            seen
        })
    }

    // MP-08 TR-16: the viewer's lanes move from the relay to a direct carrier and its display
    // registers again there (same viewer key, same generation). The pump that serves it moves
    // with it: no fresh pump, no independent frame, no frame skipped, and no pause longer than
    // one frame interval; tearing the relay carrier down afterwards leaves it running.
    #[tokio::test]
    async fn mp08_tr16_a_viewer_registered_again_on_another_carrier_keeps_its_running_pump() {
        let (router, _work) = test_router("display-pump-move");
        let (mut relay, mut direct) = (carrier(), carrier());
        let (on_relay, on_direct) = (collect(&mut relay), collect(&mut direct));
        let display = "display-tr16-move";
        let Delivery::Fresh(pump) = register_delivery(
            route(&router, &relay, display, "relay-lane", "viewer"),
            7,
            true,
        )
        .await
        else {
            panic!("the first registration starts a pump");
        };
        pump.release();
        pump.acknowledge(7, 0, false, None, false);
        let interval = Duration::from_millis(33);
        let producer = tokio::spawn({
            let pump = pump.clone();
            async move {
                let mut resets = Vec::new();
                for sequence in 1..=30 {
                    let Some(credit) = pump.push_test_frame(sequence, frame()).await else {
                        break;
                    };
                    resets.push(credit.reset);
                    pump.acknowledge(7, sequence, false, None, false);
                    sleep(interval).await;
                }
                resets
            }
        });
        sleep(interval * 10).await;
        let moved = register_delivery(
            route(&router, &direct, display, "direct-lane", "viewer"),
            7,
            true,
        )
        .await;
        let Delivery::Moved { pump: moved, left } = moved else {
            panic!("the viewer's running pump must move to the carrier it registered on");
        };
        assert!(Arc::ptr_eq(&moved, &pump));
        moved.release();
        let left = left.expect("the relay route it left");
        assert!(Arc::ptr_eq(&left.tasks, &relay.tasks));
        forget_moved_delivery(&left.tasks, &moved).await;
        assert!(relay.tasks.lock().await.is_empty());
        super::super::subscriptions::abort_subscription_tasks(&router, &relay.tasks).await;
        assert!(!pump.is_stopped(), "the carrier it left cannot stop it");
        let resets = producer.await.unwrap();
        assert_eq!(
            resets.len(),
            30,
            "the pump kept producing on the new carrier"
        );
        assert_eq!(
            resets.iter().filter(|reset| **reset).count(),
            1,
            "only the first frame is independent: {resets:?}"
        );
        drop((relay.outgoing, direct.outgoing, left));
        super::super::subscriptions::abort_subscription_tasks(&router, &direct.tasks).await;
        drop((moved, pump));
        let (relay_frames, direct_frames) = (on_relay.await.unwrap(), on_direct.await.unwrap());
        assert!(!relay_frames.is_empty() && !direct_frames.is_empty());
        assert!(relay_frames.iter().all(|(_, lane, _)| lane == "relay-lane"));
        assert!(direct_frames
            .iter()
            .all(|(_, lane, _)| lane == "direct-lane"));
        let sequences: Vec<u64> = relay_frames
            .iter()
            .chain(&direct_frames)
            .map(|(sequence, ..)| *sequence)
            .collect();
        assert_eq!(sequences, (1..=30).collect::<Vec<_>>(), "no frame skipped");
        let gap = direct_frames[0].2 - relay_frames.last().unwrap().2;
        assert!(
            gap <= interval + Duration::from_millis(25),
            "the switch paused the view for {gap:?}"
        );
        router.runtime_state().shutdown_cleanup().await.unwrap();
    }

    // Only the same viewer key at the same generation takes a pump over: another viewer, a new
    // generation and a 535 resume (a rotated delivery) each get a fresh pump.
    #[tokio::test]
    async fn mp08_tr16_another_viewer_or_generation_gets_a_fresh_pump() {
        let (router, _work) = test_router("display-pump-fresh");
        let (relay, direct) = (carrier(), carrier());
        let display = "display-tr16-fresh";
        let register = |carrier: &Carrier, viewer: &str, generation, takeover| {
            register_delivery(
                route(&router, carrier, display, "lane", viewer),
                generation,
                takeover,
            )
        };
        let Delivery::Fresh(first) = register(&relay, "viewer", 7, true).await else {
            panic!("first registration");
        };
        first.release();
        let Delivery::Fresh(other) = register(&direct, "other-viewer", 7, true).await else {
            panic!("another viewer key gets its own pump");
        };
        assert!(!Arc::ptr_eq(&other, &first) && !first.is_stopped());
        let Delivery::Fresh(next) = register(&direct, "viewer", 8, true).await else {
            panic!("a new generation gets a fresh pump");
        };
        assert!(!Arc::ptr_eq(&next, &first) && first.is_stopped());
        let Delivery::Fresh(resumed) = register(&relay, "other-viewer", 7, false).await else {
            panic!("a 535 resume never takes a pump over");
        };
        assert!(!Arc::ptr_eq(&resumed, &other) && other.is_stopped());
        for pump in [&next, &resumed] {
            pump.stop();
        }
        router.runtime_state().shutdown_cleanup().await.unwrap();
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
