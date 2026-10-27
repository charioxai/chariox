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
