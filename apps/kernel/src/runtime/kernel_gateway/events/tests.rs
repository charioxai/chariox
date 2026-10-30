//! MP-08 / MP-09 / MP-10 / MP-11: health transitions share the grant's inbox.
use super::*;

#[test]
fn outside_health_changes_wake_waits_and_stay_in_their_grant() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(check_health_changes());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn check_health_changes() {
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        4,
    );
    let gateway = router.kernel_gateway();
    let state = router.runtime_state();
    let store = state.outside_store();
    for id in ["first", "second"] {
        store
            .outside_mutate(Mutation::Subscribe {
                id: id.into(),
                home: true,
                baseline: vec![],
                open_decisions: vec![],
                targets: vec![EventTarget {
                    kernel: "remote".into(),
                    session_id: "room".into(),
                    agent_id: None,
                }],
            })
            .unwrap();
    }
    store
        .outside_mutate(Mutation::Subscribe {
            id: "refused".into(),
            home: true,
            baseline: vec![],
            open_decisions: vec![],
            targets: vec![EventTarget {
                kernel: "remote".into(),
                session_id: "room".into(),
                agent_id: None,
            }],
        })
        .unwrap();
    gateway
        .event_health(
            &Origin {
                grant_id: "refused".into(),
                harness: "test".into(),
            },
            "remote",
            Err(GatewayError::Remote {
                name: "remote".into(),
                code: "target-controlled-code".repeat(1000),
                message: "target-controlled-detail".into(),
                retryable: true,
            }),
        )
        .unwrap();
    let refused = store.outside_list("refused", None, 1000).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&refused.events[0]).unwrap()["value"],
        serde_json::json!({"online":false,"reason":"events_part_failed"})
    );
    let origin = Origin {
        grant_id: "first".into(),
        harness: "test".into(),
    };
    gateway.event_health(&origin, "remote", Ok(())).unwrap();
    assert!(store
        .outside_list("first", None, 1000)
        .unwrap()
        .unwrap()
        .events
        .is_empty());

    let wait_request = WaitEventsRequest {
        since: Some(0),
        timeout_ms: 1000,
        limit: 1000,
    };
    let waiting = state.outside_wait("first", &wait_request, None, None);
    let transition = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        gateway
            .event_health(
                &origin,
                "remote",
                Err(GatewayError::Offline {
                    name: "remote".into(),
                }),
            )
            .unwrap();
    };
    let (response, ()) = tokio::join!(waiting, transition);
    let LocalDaemonResponse::EventsListed {
        events,
        next_cursor,
        timed_out,
        ..
    } = response.unwrap()
    else {
        panic!("expected batch")
    };
    assert!(
        !timed_out,
        "remote disconnect must wake an outstanding wait"
    );
    assert_eq!(events.len(), 1);
    let value = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(value["type"], "kernel_health");
    assert_eq!(
        value["value"],
        serde_json::json!({"online":false,"reason":"kernel_offline"})
    );
    assert_eq!(events[0].kernel, "remote");
    assert!(events[0].session_id.is_empty());
    assert_eq!(events[0].cursor, next_cursor);
    assert!(next_cursor > 0);
    assert!(store
        .outside_list("second", None, 1000)
        .unwrap()
        .unwrap()
        .events
        .is_empty());

    gateway
        .event_health(
            &origin,
            "remote",
            Err(GatewayError::RequestTimeout {
                name: "remote".into(),
            }),
        )
        .unwrap();
    assert_eq!(
        store
            .outside_list("first", None, 1000)
            .unwrap()
            .unwrap()
            .events
            .len(),
        1
    );
    let wait_request = WaitEventsRequest {
        since: Some(next_cursor),
        ..wait_request
    };
    let waiting = state.outside_wait("first", &wait_request, None, None);
    let transition = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        gateway.event_health(&origin, "remote", Ok(())).unwrap();
    };
    let (response, ()) = tokio::join!(waiting, transition);
    let LocalDaemonResponse::EventsListed {
        events,
        next_cursor: recovered_cursor,
        timed_out,
        ..
    } = response.unwrap()
    else {
        panic!("expected recovery")
    };
    assert!(!timed_out);
    assert_eq!(events.len(), 1);
    assert_eq!(
        serde_json::to_value(&events[0]).unwrap()["value"],
        serde_json::json!({"online":true,"reason":"connected"})
    );
    assert!(recovered_cursor > next_cursor);
    let target = EventTarget {
        kernel: "remote".into(),
        session_id: "room".into(),
        agent_id: None,
    };
    store
        .outside_mutate(Mutation::Unsubscribe {
            id: "first".into(),
            targets: vec![target.clone()],
            all: false,
            reason: "cancelled".into(),
        })
        .unwrap();
    gateway
        .event_health(
            &origin,
            "remote",
            Err(GatewayError::Offline {
                name: "remote".into(),
            }),
        )
        .unwrap();
    store
        .outside_mutate(Mutation::Subscribe {
            id: "first".into(),
            home: true,
            targets: vec![target],
            baseline: vec![],
            open_decisions: vec![],
        })
        .unwrap();
    gateway
        .event_health(
            &origin,
            "remote",
            Err(GatewayError::Offline {
                name: "remote".into(),
            }),
        )
        .unwrap();
    let added = store
        .outside_list("first", Some(recovered_cursor), 1000)
        .unwrap()
        .unwrap();
    assert_eq!(
        added.events.len(),
        1,
        "following again must not reuse the old health cache"
    );
    assert_eq!(
        serde_json::to_value(&added.events[0]).unwrap()["value"]["online"],
        false
    );
    let recovered_cursor = added.next_cursor;
    store
        .outside_mutate(Mutation::Unsubscribe {
            id: "first".into(),
            targets: vec![],
            all: true,
            reason: "cancelled".into(),
        })
        .unwrap();
    gateway
        .event_health(
            &origin,
            "remote",
            Err(GatewayError::Offline {
                name: "remote".into(),
            }),
        )
        .unwrap();
    let events = store
        .outside_list("first", Some(recovered_cursor), 1000)
        .unwrap()
        .unwrap()
        .events;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].value,
        OutsideEventValue::SubscriptionEnded { .. }
    ));
}

#[test]
fn outside_pump_refusal_updates_health_and_wakes_once() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(check_pump_refusal());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn check_pump_refusal() {
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        4,
    );
    let gateway = router.kernel_gateway();
    let state = router.runtime_state();
    let store = state.outside_store();
    let origin = Origin {
        grant_id: "pump-refused".into(),
        harness: "test".into(),
    };
    store
        .outside_mutate(Mutation::Subscribe {
            id: origin.grant_id.clone(),
            home: true,
            baseline: vec![],
            open_decisions: vec![],
            targets: vec![EventTarget {
                kernel: "remote".into(),
                session_id: "room".into(),
                agent_id: None,
            }],
        })
        .unwrap();
    gateway.record_event_health(&origin, "remote", Ok(()));
    let request = WaitEventsRequest {
        since: Some(0),
        timeout_ms: 1000,
        limit: 1000,
    };
    let waiting = state.outside_wait(&origin.grant_id, &request, None, None);
    let refusal = || {
        Err(GatewayError::Remote {
            name: "remote".into(),
            code: "events_refused".into(),
            message: "target-controlled-detail".into(),
            retryable: false,
        })
    };
    let failing_pump = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        gateway.record_event_health(&origin, "remote", refusal());
    };
    let (response, ()) = tokio::join!(waiting, failing_pump);
    let LocalDaemonResponse::EventsListed {
        events, timed_out, ..
    } = response.unwrap()
    else {
        panic!("expected health batch")
    };
    assert!(!timed_out, "a persistent pump refusal must wake the waiter");
    assert_eq!(events.len(), 1);
    assert_eq!(
        serde_json::to_value(&events[0]).unwrap()["value"],
        serde_json::json!({"online":false,"reason":"events_part_failed"})
    );
    for _ in 0..3 {
        gateway.record_event_health(&origin, "remote", refusal());
    }
    assert_eq!(
        store
            .outside_list(&origin.grant_id, None, 1000)
            .unwrap()
            .unwrap()
            .events
            .len(),
        1,
        "repeated pump refusals must not emit duplicate transitions"
    );
    let response = gateway
        .event_request(
            &origin,
            &LocalDaemonRequest::GetEventSubscription(GetEventSubscriptionRequest {}),
            None,
        )
        .await
        .unwrap();
    let LocalDaemonResponse::EventSubscription { subscription } = response else {
        panic!("expected subscription")
    };
    assert!(!subscription.kernels[0].online);
    assert_eq!(
        subscription.kernels[0].error.as_ref().unwrap().code,
        "events_refused"
    );
    gateway.record_event_health(&origin, "remote", Ok(()));
    assert_eq!(
        store
            .outside_list(&origin.grant_id, None, 1000)
            .unwrap()
            .unwrap()
            .events
            .len(),
        2
    );
}

#[test]
fn a_busy_kernel_is_not_reported_offline_by_slow_requests() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(check_busy_kernel());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn check_busy_kernel() {
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        4,
    );
    let gateway = router.kernel_gateway();
    let state = router.runtime_state();
    let store = state.outside_store();
    let origin = Origin {
        grant_id: "busy".into(),
        harness: "test".into(),
    };
    store
        .outside_mutate(Mutation::Subscribe {
            id: origin.grant_id.clone(),
            home: true,
            baseline: vec![],
            open_decisions: vec![],
            targets: vec![EventTarget {
                kernel: "remote".into(),
                session_id: "room".into(),
                agent_id: None,
            }],
        })
        .unwrap();
    let slow = || {
        Err(GatewayError::RequestTimeout {
            name: "remote".into(),
        })
    };
    let health_events = || {
        store
            .outside_list(&origin.grant_id, None, 1000)
            .unwrap()
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.value, OutsideEventValue::KernelHealth { .. }))
            .count()
    };
    gateway.event_health(&origin, "remote", Ok(())).unwrap();
    // Load makes some pulls slow and the next ones fast: nothing to tell the follower.
    for _ in 0..5 {
        gateway.event_health(&origin, "remote", slow()).unwrap();
        gateway.event_health(&origin, "remote", Ok(())).unwrap();
    }
    assert_eq!(
        health_events(),
        0,
        "slow pulls between answers are not an outage"
    );
    // Timeouts that never let an answer through for the whole grace are an outage.
    let start = Instant::now();
    gateway
        .event_health_at(&origin, "remote", slow(), start)
        .unwrap();
    gateway
        .event_health_at(&origin, "remote", slow(), start + BUSY_GRACE / 2)
        .unwrap();
    assert_eq!(health_events(), 0);
    gateway
        .event_health_at(&origin, "remote", slow(), start + BUSY_GRACE)
        .unwrap();
    assert_eq!(health_events(), 1);
    gateway.event_health(&origin, "remote", Ok(())).unwrap();
    assert_eq!(health_events(), 2, "recovery is reported once");
    // Any other failure is reported at once, with no grace.
    gateway
        .event_health(
            &origin,
            "remote",
            Err(GatewayError::Offline {
                name: "remote".into(),
            }),
        )
        .unwrap();
    assert_eq!(health_events(), 3);
}

#[test]
fn a_pump_does_not_leave_while_a_target_was_added_to_its_kernel() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(check_pump_exit());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn check_pump_exit() {
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        4,
    );
    let gateway = router.kernel_gateway();
    let state = router.runtime_state();
    let store = state.outside_store();
    let origin = Origin {
        grant_id: "leaving".into(),
        harness: "test".into(),
    };
    let key = (origin.grant_id.clone(), "remote".to_string());
    let target = EventTarget {
        kernel: "remote".into(),
        session_id: "room".into(),
        agent_id: None,
    };
    let subscribe = |targets| Mutation::Subscribe {
        id: origin.grant_id.clone(),
        home: true,
        baseline: vec![],
        open_decisions: vec![],
        targets,
    };
    let running = || gateway.event_parts.running.lock().unwrap().contains(&key);
    gateway
        .event_parts
        .running
        .lock()
        .unwrap()
        .insert(key.clone());
    // The pump decided to leave from a look taken before this target was followed.
    store
        .outside_mutate(subscribe(vec![target.clone()]))
        .unwrap();
    assert!(
        !gateway.finish_event_part(&origin, "remote", &key),
        "a followed target keeps the pump"
    );
    assert!(running());
    store
        .outside_mutate(Mutation::Unsubscribe {
            id: origin.grant_id.clone(),
            targets: vec![target],
            all: false,
            reason: "cancelled".into(),
        })
        .unwrap();
    assert!(gateway.finish_event_part(&origin, "remote", &key));
    assert!(!running(), "with nothing followed the pump is gone");
    // The next target starts a pump again.
    assert!(gateway
        .event_parts
        .running
        .lock()
        .unwrap()
        .insert(key.clone()));
}
