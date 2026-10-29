//! MP-08 / MP-09 / MP-10 / MP-11: durable outside delivery, not provider acceptance.
use super::*;
use crate::local::*;
use outside::Mutation;
use tests::Fixture;

fn target(session: &str, agent: Option<&str>) -> EventTarget {
    EventTarget {
        kernel: "kernel-a".into(),
        session_id: session.into(),
        agent_id: agent.map(Into::into),
    }
}
fn subscribe(f: &Fixture, id: &str, home: bool, targets: Vec<EventTarget>) {
    f.store
        .outside_mutate(Mutation::Subscribe {
            open_decisions: vec![],
            baseline: vec![],
            id: id.into(),
            home,
            targets,
        })
        .unwrap();
}
fn event(session: &str, agent: &str) -> OutsideEvent {
    OutsideEvent {
        cursor: 0,
        kernel: "kernel-a".into(),
        machine: "machine-a".into(),
        session_id: session.into(),
        agent_id: Some(agent.into()),
        value: OutsideEventValue::Agent {
            run_state: Some(EventRunState::Idle),
            task_state: None,
            reason: None,
        },
        summary: "idle".into(),
        at_ms: 1,
        refs: Default::default(),
    }
}
fn publish(f: &Fixture, occurrence: &str, event: OutsideEvent) {
    f.store
        .outside_mutate(Mutation::Publish {
            occurrence: occurrence.into(),
            event: Box::new(event),
        })
        .unwrap();
}
fn list(f: &Fixture, id: &str) -> EventsBatch {
    f.store.outside_list(id, None, 1000).unwrap().unwrap()
}

#[test]
fn outside_parts_filter_at_the_producer_and_do_not_enter_session_delivery() {
    let f = Fixture::new();
    subscribe(
        &f,
        "grant",
        true,
        vec![target("x", None), target("y", Some("y2"))],
    );
    publish(&f, "1", event("x", "x1"));
    publish(&f, "2", event("x", "new-agent"));
    publish(&f, "3", event("y", "y1"));
    publish(&f, "4", event("y", "y2"));
    publish(&f, "4", event("y", "y2"));
    publish(&f, "5", event("unfollowed", "x1"));
    let batch = list(&f, "grant");
    assert_eq!(
        batch
            .events
            .iter()
            .map(|e| e.agent_id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["x1", "new-agent", "y2"]
    );
    assert!(batch.events.windows(2).all(|p| p[0].cursor < p[1].cursor));
    assert!(f.store.agent_pending_inbox_recipients().unwrap().is_empty());
    assert!(f.store.agent_inbox("x", "x1", 0).unwrap().is_empty());
}

#[test]
fn outside_ack_and_merged_high_water_are_durable() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    let mut first = event("x", "x1");
    first.cursor = 10;
    let mut second = event("x", "x1");
    second.cursor = 20;
    let import = Mutation::Import {
        id: "grant".into(),
        kernel: "kernel-a".into(),
        events: vec![first, second],
    };
    f.store.outside_mutate(import.clone()).unwrap();
    let batch = list(&f, "grant");
    assert_eq!(batch.events.len(), 2);
    f.store
        .outside_mutate(Mutation::Ack {
            id: "grant".into(),
            cursor: batch.next_cursor,
        })
        .unwrap();
    f.store.outside_mutate(import.clone()).unwrap();
    assert!(list(&f, "grant").events.is_empty());
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    assert_eq!(
        reopened
            .outside_part("grant")
            .unwrap()
            .unwrap()
            .remote_cursors["kernel-a"],
        20
    );
    reopened.outside_mutate(import).unwrap();
    assert!(reopened
        .outside_list("grant", None, 1000)
        .unwrap()
        .unwrap()
        .events
        .is_empty());
    assert!(reopened
        .outside_mutate(Mutation::Ack {
            id: "grant".into(),
            cursor: batch.next_cursor + 1
        })
        .is_err());
}

#[test]
fn outside_restart_ends_home_only_and_session_end_removes_only_its_targets() {
    let f = Fixture::new();
    subscribe(
        &f,
        "home",
        true,
        vec![target("x", None), target("y", Some("y2"))],
    );
    subscribe(&f, "part", false, vec![target("x", None)]);
    let mut ended = event("x", "x1");
    ended.agent_id = None;
    ended.value = OutsideEventValue::Session {
        state: EventSessionState::Ended,
    };
    publish(&f, "session-end", ended);
    assert_eq!(
        f.store.outside_part("home").unwrap().unwrap().targets,
        vec![target("y", Some("y2"))]
    );
    assert!(f
        .store
        .outside_part("part")
        .unwrap()
        .unwrap()
        .ended
        .is_none());
    f.store
        .outside_mutate(Mutation::Restart { keep: vec![] })
        .unwrap();
    publish(&f, "after", event("y", "y2"));
    let home = list(&f, "home");
    assert_eq!(home.events.len(), 2);
    assert!(
        matches!(&home.events.last().unwrap().value,OutsideEventValue::SubscriptionEnded { reason } if reason=="kernel_restarted")
    );
    assert!(f
        .store
        .outside_part("part")
        .unwrap()
        .unwrap()
        .ended
        .is_none());
    assert!(f
        .store
        .outside_mutate(Mutation::Subscribe {
            open_decisions: vec![],
            baseline: vec![],
            id: "home".into(),
            home: true,
            targets: vec![target("y", None)]
        })
        .is_err());
}

#[test]
fn outside_retention_returns_an_expired_cursor_and_an_ordered_gap() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    for n in 0..8 {
        publish(&f, &n.to_string(), event("x", "x1"));
    }
    f.store
        .outside_mutate(Mutation::Bound {
            id: "grant".into(),
            limit: 4,
        })
        .unwrap();
    let oldest = f
        .store
        .outside_list("grant", Some(0), 1000)
        .unwrap()
        .unwrap_err();
    let batch = f
        .store
        .outside_list("grant", Some(oldest - 1), 1000)
        .unwrap()
        .unwrap();
    assert_eq!(batch.events.len(), 4);
    assert!(matches!(
        batch.events[0].value,
        OutsideEventValue::Gap {
            dropped: 5,
            from: 1,
            to: 5,
            ..
        }
    ));
    assert!(batch.events.windows(2).all(|p| p[0].cursor < p[1].cursor));
}

#[test]
fn outside_retention_excludes_health_from_remote_gap_ranges() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    for (n, online) in [(1000, true), (1001, false), (1002, true), (1003, false)] {
        f.store
            .outside_mutate(Mutation::KernelHealth {
                id: "grant".into(),
                kernel: "kernel-a".into(),
                online,
                reason: "test".into(),
            })
            .unwrap();
        let mut e = event("x", "x1");
        e.cursor = n;
        f.store
            .outside_mutate(Mutation::Import {
                id: "grant".into(),
                kernel: "kernel-a".into(),
                events: vec![e],
            })
            .unwrap();
    }
    for (limit, expected) in [(4, (1000, 1001, 2)), (2, (1000, 1002, 3))] {
        f.store
            .outside_mutate(Mutation::Bound {
                id: "grant".into(),
                limit,
            })
            .unwrap();
        let oldest = f
            .store
            .outside_list("grant", Some(0), 1000)
            .unwrap()
            .unwrap_err();
        let batch = f
            .store
            .outside_list("grant", Some(oldest - 1), 1000)
            .unwrap()
            .unwrap();
        assert!(batch.events.len() <= limit);
        let gap = batch.events.iter().find_map(|e| match e.value {
            OutsideEventValue::Gap {
                from, to, dropped, ..
            } => Some((from, to, dropped)),
            _ => None,
        });
        assert_eq!(
            gap,
            Some(expected),
            "remote gap metadata must only count remote source events"
        );
        assert!(batch.events.windows(2).all(|p| p[0].cursor < p[1].cursor));
    }
}

#[test]
fn outside_retention_can_drop_only_health_without_a_source_gap() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    for online in [false, true, false, true, false, true] {
        f.store
            .outside_mutate(Mutation::KernelHealth {
                id: "grant".into(),
                kernel: "kernel-a".into(),
                online,
                reason: "test".into(),
            })
            .unwrap();
    }
    f.store
        .outside_mutate(Mutation::Bound {
            id: "grant".into(),
            limit: 2,
        })
        .unwrap();
    let oldest = f
        .store
        .outside_list("grant", Some(0), 1000)
        .unwrap()
        .unwrap_err();
    let batch = f
        .store
        .outside_list("grant", Some(oldest - 1), 1000)
        .unwrap()
        .unwrap();
    assert!(batch.events.len() <= 2);
    assert!(batch
        .events
        .iter()
        .all(|e| matches!(e.value, OutsideEventValue::KernelHealth { .. })));
    let part = f.store.outside_part("grant").unwrap().unwrap();
    assert!(part.kernel_health["kernel-a"]);
}

#[test]
fn outside_retention_identifies_each_kernel_and_preserves_the_cap() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    let mut events = Vec::new();
    for n in 1..=4 {
        let mut e = event("x", "x1");
        e.cursor = n;
        events.push(e);
    }
    f.store
        .outside_mutate(Mutation::Import {
            id: "grant".into(),
            kernel: "kernel-a".into(),
            events,
        })
        .unwrap();
    let events_b = (1..=4)
        .map(|n| {
            let mut event = event("x", "x1");
            event.kernel = "kernel-b".into();
            event.cursor = n;
            event
        })
        .collect();
    f.store
        .outside_mutate(Mutation::Import {
            id: "grant".into(),
            kernel: "kernel-b".into(),
            events: events_b,
        })
        .unwrap();
    f.store
        .outside_mutate(Mutation::Bound {
            id: "grant".into(),
            limit: 4,
        })
        .unwrap();
    let oldest = f
        .store
        .outside_list("grant", Some(0), 100)
        .unwrap()
        .unwrap_err();
    let batch = f
        .store
        .outside_list("grant", Some(oldest - 1), 100)
        .unwrap()
        .unwrap();
    assert_eq!(batch.events.len(), 4);
    assert_eq!(
        batch
            .events
            .iter()
            .filter(|event| matches!(event.value, OutsideEventValue::Gap { .. }))
            .count(),
        2
    );
    assert!(batch
        .events
        .iter()
        .filter(|e| matches!(e.value, OutsideEventValue::Gap { .. }))
        .all(|e| matches!(&e.value,OutsideEventValue::Gap { kernel,.. } if kernel==&e.kernel)));
}
#[test]
fn outside_import_rejects_unordered_or_misattributed_batches_atomically() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    let mut one = event("x", "x1");
    one.cursor = 1;
    let mut two = one.clone();
    two.cursor = 2;
    assert!(f
        .store
        .outside_mutate(Mutation::Import {
            id: "grant".into(),
            kernel: "kernel-a".into(),
            events: vec![two, one.clone()]
        })
        .is_err());
    assert!(f
        .store
        .outside_mutate(Mutation::Import {
            id: "grant".into(),
            kernel: "wrong-kernel".into(),
            events: vec![one]
        })
        .is_err());
    assert!(list(&f, "grant").events.is_empty());
}

#[test]
fn outside_ack_expires_cursors_whose_payload_was_released() {
    let f = Fixture::new();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    publish(&f, "first", event("x", "x1"));
    publish(&f, "second", event("x", "x1"));
    let batch = list(&f, "grant");
    let first = batch.events[0].cursor;
    let last = batch.next_cursor;
    f.store
        .outside_mutate(Mutation::Ack {
            id: "grant".into(),
            cursor: first,
        })
        .unwrap();
    assert_eq!(
        f.store
            .outside_list("grant", Some(0), 100)
            .unwrap()
            .unwrap_err(),
        first + 1
    );
    let remaining = f
        .store
        .outside_list("grant", Some(first), 100)
        .unwrap()
        .unwrap();
    assert_eq!(remaining.events.len(), 1);
    assert_eq!(remaining.events[0].cursor, last);
    f.store
        .outside_mutate(Mutation::Ack {
            id: "grant".into(),
            cursor: last,
        })
        .unwrap();
    assert_eq!(
        f.store
            .outside_list("grant", Some(0), 100)
            .unwrap()
            .unwrap_err(),
        last + 1
    );
    assert!(list(&f, "grant").events.is_empty());
}

// MP-08 / MP-09 / MP-10 / MP-11: local targets have no remote part to remove.
#[test]
fn outside_cancel_and_target_removal_queue_only_remote_cleanup() {
    let f = Fixture::new();
    f.store
        .outside_mutate(Mutation::Identity {
            kernel: "kernel-a".into(),
            machine: "machine-a".into(),
        })
        .unwrap();
    let local = target("x", None);
    let mut remote = target("y", None);
    remote.kernel = "kernel-b".into();
    for all in [false, true] {
        let id = if all { "cancel" } else { "remove" };
        subscribe(&f, id, true, vec![local.clone(), remote.clone()]);
        f.store
            .outside_mutate(Mutation::Unsubscribe {
                id: id.into(),
                targets: vec![local.clone(), remote.clone()],
                all,
                reason: "cancelled".into(),
            })
            .unwrap();
        assert_eq!(
            f.store.outside_part(id).unwrap().unwrap().cleanup_targets,
            vec![remote.clone()],
            "only a remote kernel needs cleanup"
        );
    }
    subscribe(&f, "ended-session", true, vec![local]);
    let mut ended = event("x", "x1");
    ended.agent_id = None;
    ended.value = OutsideEventValue::Session {
        state: EventSessionState::Ended,
    };
    publish(&f, "end-local-session", ended);
    assert!(f
        .store
        .outside_part("ended-session")
        .unwrap()
        .unwrap()
        .cleanup_targets
        .is_empty());
}

#[test]
fn outside_restart_repairs_old_local_cleanup_without_losing_remote_work() {
    let f = Fixture::new();
    f.store
        .outside_mutate(Mutation::Identity {
            kernel: "kernel-a".into(),
            machine: "machine-a".into(),
        })
        .unwrap();
    let local = target("x", None);
    let mut remote = target("y", None);
    remote.kernel = "kernel-b".into();
    subscribe(&f, "cancel", true, vec![local.clone(), remote.clone()]);
    f.store
        .outside_mutate(Mutation::Unsubscribe {
            id: "cancel".into(),
            targets: vec![],
            all: true,
            reason: "cancelled".into(),
        })
        .unwrap();
    // Simulate the durable payload written by the previous version, including
    // an already-cancelled home so recovery cannot rely on ending it again.
    let mut part = f.store.outside_part("cancel").unwrap().unwrap();
    part.cleanup_targets = vec![local, remote.clone()];
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE agent_outside_subscriptions SET payload=?1 WHERE id='cancel'",
        [serde_json::to_string(&part).unwrap()],
    )
    .unwrap();
    drop(db);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    reopened
        .outside_mutate(Mutation::Restart { keep: vec![] })
        .unwrap();
    let part = reopened.outside_part("cancel").unwrap().unwrap();
    assert_eq!(part.ended.as_deref(), Some("cancelled"));
    assert_eq!(part.cleanup_targets, vec![remote.clone()]);
    reopened
        .outside_mutate(Mutation::CleanupAck {
            id: "cancel".into(),
            targets: vec![remote],
        })
        .unwrap();
    reopened
        .outside_mutate(Mutation::Restart { keep: vec![] })
        .unwrap();
    assert!(reopened
        .outside_part("cancel")
        .unwrap()
        .unwrap()
        .cleanup_targets
        .is_empty());
}

#[test]
fn a_producer_without_a_subscription_does_not_wait_for_the_durable_writer() {
    let f = Fixture::new();
    f.store
        .outside_mutate(Mutation::Identity {
            kernel: "kernel-a".into(),
            machine: "machine-a".into(),
        })
        .unwrap();
    let publish_session_event = || {
        f.store.outside_publish(
            "x",
            Some("x1"),
            OutsideEventValue::Session {
                state: EventSessionState::AgentAdded,
            },
            "added",
            Default::default(),
        )
    };
    let mut blocker = rusqlite::Connection::open(f.store.path()).unwrap();
    let held = blocker
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    publish_session_event().expect("nobody follows this kernel, so nothing is written");
    held.rollback().unwrap();
    subscribe(&f, "grant", true, vec![target("x", None)]);
    publish_session_event().unwrap();
    assert_eq!(list(&f, "grant").events.len(), 1);
}

#[test]
fn outside_health_is_durable_ordered_and_scoped_to_followed_home_parts() {
    let f = Fixture::new();
    subscribe(&f, "home", true, vec![target("room", None)]);
    subscribe(&f, "other", true, vec![target("room", None)]);
    subscribe(&f, "source", false, vec![target("room", None)]);
    let health = |id: &str, online| Mutation::KernelHealth {
        id: id.into(),
        kernel: "kernel-a".into(),
        online,
        reason: if online {
            "connected"
        } else {
            "kernel_offline"
        }
        .into(),
    };
    f.store.outside_mutate(health("home", true)).unwrap();
    assert!(list(&f, "home").events.is_empty());
    publish(&f, "before-health", event("room", "agent"));
    f.store.outside_mutate(health("home", false)).unwrap();
    f.store.outside_mutate(health("source", false)).unwrap();
    let first = list(&f, "home");
    assert_eq!(first.events.len(), 2);
    assert!(matches!(
        first.events[0].value,
        OutsideEventValue::Agent { .. }
    ));
    assert!(matches!(
        first.events[1].value,
        OutsideEventValue::KernelHealth { online: false, .. }
    ));
    assert!(first.events[1].cursor > first.events[0].cursor);
    assert_eq!(first.next_cursor, first.events[1].cursor);
    assert_eq!(list(&f, "other").events.len(), 1);
    assert_eq!(list(&f, "source").events.len(), 1);

    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    reopened.outside_mutate(health("home", false)).unwrap();
    assert_eq!(list(&f, "home"), first);
    reopened.outside_mutate(health("home", true)).unwrap();
    let recovered = list(&f, "home");
    assert_eq!(recovered.events.len(), 3);
    assert!(
        matches!(&recovered.events[2].value, OutsideEventValue::KernelHealth { online: true, reason } if reason == "connected")
    );
    assert!(recovered.next_cursor > first.next_cursor);
    reopened
        .outside_mutate(Mutation::Ack {
            id: "home".into(),
            cursor: first.next_cursor,
        })
        .unwrap();
    assert_eq!(list(&f, "home").events, recovered.events[2..]);
}

#[test]
fn outside_health_ignores_removed_targets_and_ended_subscriptions() {
    let f = Fixture::new();
    subscribe(
        &f,
        "home",
        true,
        vec![target("one", None), target("two", None)],
    );
    let health = |kernel: &str, online| Mutation::KernelHealth {
        id: "home".into(),
        kernel: kernel.into(),
        online,
        reason: if online {
            "connected"
        } else {
            "kernel_offline"
        }
        .into(),
    };
    f.store.outside_mutate(health("unknown", false)).unwrap();
    assert!(list(&f, "home").events.is_empty());
    // An initial failure is observable even if this part never connected.
    f.store.outside_mutate(health("kernel-a", false)).unwrap();
    assert_eq!(list(&f, "home").events.len(), 1);
    f.store
        .outside_mutate(Mutation::Unsubscribe {
            id: "home".into(),
            targets: vec![target("one", None)],
            all: false,
            reason: "cancelled".into(),
        })
        .unwrap();
    f.store.outside_mutate(health("kernel-a", true)).unwrap();
    assert_eq!(list(&f, "home").events.len(), 2);
    f.store
        .outside_mutate(Mutation::Unsubscribe {
            id: "home".into(),
            targets: vec![target("two", None)],
            all: false,
            reason: "cancelled".into(),
        })
        .unwrap();
    f.store.outside_mutate(health("kernel-a", false)).unwrap();
    assert_eq!(list(&f, "home").events.len(), 2);
    assert!(f
        .store
        .outside_part("home")
        .unwrap()
        .unwrap()
        .kernel_health
        .is_empty());
    subscribe(&f, "home", true, vec![target("three", None)]);
    f.store.outside_mutate(health("kernel-a", true)).unwrap();
    assert_eq!(list(&f, "home").events.len(), 2);
    f.store
        .outside_mutate(Mutation::Unsubscribe {
            id: "home".into(),
            targets: vec![],
            all: true,
            reason: "cancelled".into(),
        })
        .unwrap();
    f.store.outside_mutate(health("kernel-a", false)).unwrap();
    let events = list(&f, "home").events;
    assert_eq!(events.len(), 3);
    assert!(matches!(
        events[2].value,
        OutsideEventValue::SubscriptionEnded { .. }
    ));
}

// A client may list a source event without acknowledging it before retention.
fn listed_terminal_at_ten(f: &Fixture, kernels: &[&str]) -> u64 {
    let targets = kernels
        .iter()
        .map(|kernel| EventTarget {
            kernel: (*kernel).into(),
            ..target("x", None)
        })
        .collect();
    subscribe(f, "grant", true, targets);
    for n in 1..=9 {
        publish(f, &n.to_string(), event("x", "x1"));
    }
    f.store
        .outside_mutate(Mutation::Ack {
            id: "grant".into(),
            cursor: 9,
        })
        .unwrap();
    import_terminal(f, "kernel-a", 1000);
    let listed = list(f, "grant");
    assert_eq!(listed.next_cursor, 10);
    assert_eq!(listed.events.len(), 1);
    10
}
fn import_terminal(f: &Fixture, kernel: &str, cursor: u64) {
    let mut e = event("x", "x1");
    e.kernel = kernel.into();
    e.cursor = cursor;
    e.value = OutsideEventValue::Terminal {
        exit_code: Some(23),
    };
    f.store
        .outside_mutate(Mutation::Import {
            id: "grant".into(),
            kernel: kernel.into(),
            events: vec![e],
        })
        .unwrap();
}
fn offline(f: &Fixture, kernel: &str) {
    f.store
        .outside_mutate(Mutation::KernelHealth {
            id: "grant".into(),
            kernel: kernel.into(),
            online: false,
            reason: "test".into(),
        })
        .unwrap();
}

#[test]
fn outside_retention_keeps_health_after_the_listed_source_cursor_covered() {
    let f = Fixture::new();
    let listed = listed_terminal_at_ten(&f, &["kernel-a"]);
    offline(&f, "kernel-a"); // health@11 between terminal@10 and terminal@12
    import_terminal(&f, "kernel-a", 1001);
    import_terminal(&f, "kernel-a", 1002);
    f.store
        .outside_mutate(Mutation::Bound {
            id: "grant".into(),
            limit: 3,
        })
        .unwrap();
    let resumed = f
        .store
        .outside_list("grant", Some(listed), 1000)
        .unwrap()
        .unwrap();
    assert_eq!(
        resumed.events[0].cursor, 11,
        "the gap must cover the dropped health row after cursor 10"
    );
    assert!(matches!(
        resumed.events[0].value,
        OutsideEventValue::Gap {
            dropped: 1,
            from: 1000,
            to: 1000,
            ..
        }
    ));
}

#[test]
fn outside_retention_covers_later_health_for_each_kernel() {
    check_other_kernel_health(true);
}

#[test]
fn outside_retention_health_only_kernel_expires_past_other_gaps() {
    check_other_kernel_health(false);
}

fn check_other_kernel_health(source_before_health: bool) {
    let f = Fixture::new();
    let mut listed = listed_terminal_at_ten(&f, &["kernel-a", "kernel-b"]);
    if source_before_health {
        import_terminal(&f, "kernel-b", 2000);
        listed = list(&f, "grant").next_cursor;
    }
    offline(&f, "kernel-b");
    for cursor in 1001..=1003 {
        import_terminal(&f, "kernel-a", cursor);
    }
    f.store
        .outside_mutate(Mutation::Bound {
            id: "grant".into(),
            limit: 4,
        })
        .unwrap();
    let resumed = f.store.outside_list("grant", Some(listed), 1000).unwrap();
    if source_before_health {
        let resumed = resumed.unwrap();
        assert!(
            resumed.events.iter().any(|e| e.kernel == "kernel-b"
                && matches!(
                    e.value,
                    OutsideEventValue::Gap {
                        dropped: 1,
                        from: 2000,
                        to: 2000,
                        ..
                    }
                )),
            "kernel B's health after the listed cursor must be covered by its gap"
        );
    } else {
        assert!(
            resumed.is_err(),
            "health-only kernel B must expire the cursor past its lost transition"
        );
    }
}

fn identify(store: &DurableKernelStateStore) {
    store
        .outside_mutate(Mutation::Identity {
            kernel: "kernel-a".into(),
            machine: "machine-a".into(),
        })
        .unwrap();
}
fn working(session: &str, agent: &str) -> outside::RunState {
    outside::RunState {
        session_id: session.into(),
        agent_id: agent.into(),
        state: EventRunState::Working,
    }
}
fn run_states(f: &Fixture, id: &str) -> Vec<(String, EventRunState)> {
    list(f, id)
        .events
        .into_iter()
        .filter_map(|event| match event.value {
            OutsideEventValue::Agent { run_state, .. } => Some((event.agent_id?, run_state?)),
            _ => None,
        })
        .collect()
}

// MP-08 / MP-09 / MP-10 / MP-11: the writer cost of a publication must not multiply
// the subscription payloads by the number of published states.
#[test]
fn outside_run_states_write_each_followed_part_once() {
    let f = Fixture::new();
    identify(&f.store);
    for id in ["a", "b", "c"] {
        subscribe(&f, id, true, vec![target("s", None)]);
    }
    let states = (0..50)
        .map(|i| working("s", &format!("agent-{i}")))
        .collect();
    let mut db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    let before = tx.total_changes();
    outside::apply(&tx, Mutation::RunStates { states }).unwrap();
    assert_eq!(
        tx.total_changes() - before,
        3 * 50 + 3,
        "one inbox row per part and state, one part write per part"
    );
    tx.commit().unwrap();
    assert_eq!(run_states(&f, "b").len(), 50);
}

// MP-08 / MP-09 / MP-10 / MP-11: unchanged run states never reach the writer again,
// and a new follower is offered every state.
#[test]
fn outside_run_states_queue_only_changes_until_the_targets_change() {
    let f = Fixture::new();
    identify(&f.store);
    subscribe(&f, "a", true, vec![target("s", None)]);
    let queue = |states: Vec<outside::RunState>| {
        f.store
            .enqueue_outside_run_states(Some("s"), |_| Some(states))
            .unwrap()
    };
    assert!(queue(vec![working("s", "x")]));
    assert!(
        !queue(vec![working("s", "x")]),
        "unchanged state queued again"
    );
    assert!(!f
        .store
        .enqueue_outside_run_states(Some("unfollowed"), |_| panic!(
            "captured an unfollowed session"
        ))
        .unwrap());
    subscribe(&f, "b", true, vec![target("s", None)]);
    assert!(
        queue(vec![working("s", "x")]),
        "a subscribe must offer every state again"
    );
    f.store.outside_settled().unwrap();
    assert_eq!(run_states(&f, "a"), [("x".into(), EventRunState::Working)]);
    assert_eq!(run_states(&f, "b"), [("x".into(), EventRunState::Working)]);
}

// MP-08 / MP-09 / MP-10 / MP-11: nobody waits for a publication, so a state the writer
// could not commit is offered again by the next one instead of being lost.
#[test]
fn outside_run_states_offer_again_after_a_failed_commit() {
    let root = std::env::temp_dir().join(format!(
        "chariox-outside-reoffer-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(root.join("faults")).unwrap();
    let store = {
        let _faults = faults::TestFaultDir::set(&root.join("faults"));
        DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap()
    };
    let f = Fixture { root, store };
    identify(&f.store);
    subscribe(&f, "a", true, vec![target("s", None)]);
    let queue = || {
        f.store
            .enqueue_outside_run_states(None, |_| Some(vec![working("s", "x")]))
            .unwrap()
    };
    std::fs::write(f.root.join("faults/writer-fail"), "").unwrap();
    assert!(queue());
    assert!(f.store.outside_settled().is_err());
    std::fs::remove_file(f.root.join("faults/writer-fail")).unwrap();
    assert!(queue(), "the failed state must be offered again");
    f.store.outside_settled().unwrap();
    assert_eq!(run_states(&f, "a"), [("x".into(), EventRunState::Working)]);
}
