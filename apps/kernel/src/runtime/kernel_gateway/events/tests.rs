//! MP-08 / MP-09 / MP-10 / MP-11: health transitions share the grant's inbox.
use super::*;
use std::future::Future;

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

fn run_with_gateway<F, Fut>(check: F)
where
    F: FnOnce(Arc<KernelGateway>, crate::runtime::state::KernelRuntimeState) -> Fut
        + Send
        + 'static,
    Fut: Future<Output = ()>,
{
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                        .unwrap();
                    let router =
                        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
                            Arc::new(Mutex::new(app)),
                            4,
                        );
                    check(router.kernel_gateway().clone(), router.runtime_state()).await
                });
        })
        .unwrap()
        .join()
        .unwrap();
}

fn remote_session(session_id: &str) -> EventTarget {
    EventTarget {
        kernel: "remote".into(),
        session_id: session_id.into(),
        agent_id: None,
    }
}

fn remote_error(code: &str) -> GatewayError {
    GatewayError::Remote {
        name: "remote".into(),
        code: code.into(),
        message: "target-controlled-detail".into(),
        retryable: false,
    }
}

fn follow(
    state: &crate::runtime::state::KernelRuntimeState,
    grants: &[&str],
    targets: &[&EventTarget],
) {
    for id in grants {
        state
            .outside_store()
            .outside_mutate(Mutation::Subscribe {
                id: (*id).into(),
                home: true,
                baseline: vec![],
                open_decisions: vec![],
                targets: targets.iter().map(|target| (*target).clone()).collect(),
            })
            .unwrap();
    }
}

#[test]
fn a_permanent_error_that_is_not_a_refused_target_keeps_every_followed_target() {
    run_with_gateway(|gateway, state| async move {
        let (live, other) = (remote_session("live"), remote_session("other"));
        follow(&state, &["a", "b"], &[&live, &other]);
        let result = gateway
            .install_each("remote", &[live.clone(), other.clone()], |_| async {
                Err(remote_error("relay_request_failed"))
            })
            .await;
        assert!(
            matches!(&result, Err(GatewayError::Remote { code, .. }) if code == "relay_request_failed"),
            "the storage failure stays a health failure: {result:?}"
        );
        for id in ["a", "b"] {
            let part = state.outside_store().outside_part(id).unwrap().unwrap();
            assert_eq!(
                part.targets,
                vec![live.clone(), other.clone()],
                "grant {id}"
            );
            assert!(
                state
                    .outside_store()
                    .outside_list(id, None, 100)
                    .unwrap()
                    .unwrap()
                    .events
                    .is_empty(),
                "grant {id} must not be told a live target ended"
            );
        }
    });
}

#[test]
fn a_target_the_kernel_refuses_alone_is_dropped_and_reported_ended() {
    run_with_gateway(|gateway, state| async move {
        let (live, stale) = (remote_session("live"), remote_session("stale"));
        follow(&state, &["a", "b"], &[&live, &stale]);
        let stale_id = stale.clone();
        gateway
            .install_each("remote", &[live.clone(), stale.clone()], move |batch| {
                let refused = batch.contains(&stale_id);
                async move {
                    if refused {
                        Err(remote_error("events_refused"))
                    } else {
                        Ok(())
                    }
                }
            })
            .await
            .unwrap();
        for id in ["a", "b"] {
            let part = state.outside_store().outside_part(id).unwrap().unwrap();
            assert_eq!(part.targets, vec![live.clone()], "grant {id}");
            let events = state
                .outside_store()
                .outside_list(id, None, 100)
                .unwrap()
                .unwrap()
                .events;
            assert_eq!(events.len(), 1, "grant {id}");
            assert_eq!(events[0].session_id, "stale");
            assert!(matches!(
                events[0].value,
                OutsideEventValue::Session {
                    state: EventSessionState::Ended
                }
            ));
        }
    });
}

#[test]
fn cleanup_of_targets_the_kernel_never_installed_completes() {
    run_with_gateway(|gateway, state| async move {
        let (first, second) = (remote_session("first"), remote_session("second"));
        follow(&state, &["a"], &[&first, &second]);
        // Every session ended before the retry: no remote part was ever created.
        gateway
            .install_each("remote", &[first.clone(), second.clone()], |_| async {
                Err(remote_error("events_refused"))
            })
            .await
            .unwrap();
        let origin = Origin {
            grant_id: "a".into(),
            harness: "test".into(),
        };
        let owed = || {
            state
                .outside_store()
                .outside_part("a")
                .unwrap()
                .unwrap()
                .cleanup_targets
        };
        assert_eq!(owed(), vec![first.clone(), second.clone()]);
        // The target has nothing to remove and refuses the unsubscribe for lack of a part.
        gateway
            .clean_remote_part(&origin, &owed(), false, |_| async {
                Err(remote_error("events_refused"))
            })
            .await
            .unwrap();
        assert!(owed().is_empty(), "the cleanup debt must be settled");
    });
}

#[test]
fn cleanup_stays_owed_while_the_target_cannot_be_reached() {
    run_with_gateway(|gateway, state| async move {
        let target = remote_session("room");
        follow(&state, &["a"], &[&target]);
        gateway
            .install_each("remote", std::slice::from_ref(&target), |_| async {
                Err(remote_error("events_refused"))
            })
            .await
            .unwrap();
        let origin = Origin {
            grant_id: "a".into(),
            harness: "test".into(),
        };
        let cleaned = gateway
            .clean_remote_part(&origin, std::slice::from_ref(&target), false, |_| async {
                Err(GatewayError::Offline {
                    name: "remote".into(),
                })
            })
            .await;
        assert!(matches!(cleaned, Err(GatewayError::Offline { .. })));
        let part = state.outside_store().outside_part("a").unwrap().unwrap();
        assert_eq!(part.cleanup_targets, vec![target]);
    });
}
