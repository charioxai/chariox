//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17): an outside agent's grant as a subscriber, on the
//! real durable writer. Supplementary regressions, not live acceptance.
use super::tests::{cleanup, Fixture};
use super::*;
use crate::durable_state::agent_lifecycle::outside::Mutation;
use crate::local::{
    NotificationSourceKind, OutsideEvent, OutsideEventValue, WorkflowNotificationAck,
    WorkflowNotificationTargetKind,
};
use crate::session::WorkflowRunStatus;

const GRANT: &str = "grant-a";

fn follow(f: &Fixture, source: &WorkflowNotificationSource) -> WorkflowNotificationSubscription {
    f.store
        .outside_mutate(Mutation::Identity {
            kernel: "daemon-test".into(),
            machine: "machine-test".into(),
        })
        .unwrap();
    f.store
        .outside_mutate(Mutation::Subscribe {
            id: GRANT.into(),
            home: true,
            targets: vec![],
            baseline: vec![],
            open_decisions: vec![],
        })
        .unwrap();
    let subscription = WorkflowNotificationSubscription {
        subscription_id: format!("sub-grant-{}", source.source_id),
        source_id: source.source_id.clone(),
        owner_user_id: "local".into(),
        created_by_user_id: "local".into(),
        target_kernel_id: source.kernel_id.clone(),
        target_kind: WorkflowNotificationTargetKind::OutsideAgent,
        session_id: source.session_id.clone(),
        workflow_id: String::new(),
        publication_id: GRANT.into(),
        endpoint_id: GRANT.into(),
        queue_id: "grant".into(),
        source_available: true,
        source_kernel_id: source.kernel_id.clone(),
        agent_id: None,
        trigger_id: None,
    };
    match f
        .store
        .notify(NotificationOperation::Granted {
            grant: GRANT.into(),
            attach: Box::new(NotificationOperation::RemoteAttach { subscription }),
        })
        .unwrap()
    {
        NotificationOutcome::Subscription(subscription) => subscription,
        other => panic!("{other:?}"),
    }
}
fn events(f: &Fixture) -> Vec<OutsideEvent> {
    f.store
        .outside_list(GRANT, Some(0), 100)
        .unwrap()
        .unwrap()
        .events
}
fn accept(
    f: &Fixture,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<NotificationOutcome, DaemonError> {
    f.store.notify(NotificationOperation::Accept {
        subscription: sub.clone(),
        envelope: env.clone(),
    })
}
fn append(f: &Fixture, sub: &WorkflowNotificationSubscription, env: &WorkflowNotificationEnvelope) {
    f.store
        .notify(NotificationOperation::AppendToGrantInbox {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
}

#[test]
fn an_outside_agent_receives_each_occurrence_once_and_settles_in_the_same_commit() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = f.source(&a);
    let sub = follow(&f, &source);
    f.complete(&a, "run-1", None, WorkflowRunStatus::Completed, "approved");
    let (_, env) = f.candidates(false).remove(0);
    assert!(events(&f).is_empty());
    assert!(matches!(
        accept(&f, &sub, &env).unwrap(),
        NotificationOutcome::Ack(WorkflowNotificationAck::Accepted)
    ));
    assert!(events(&f).is_empty(), "accepting appends nothing yet");
    append(&f, &sub, &env);
    let delivered = events(&f);
    assert_eq!(delivered.len(), 1);
    let OutsideEventValue::Notification {
        source_id,
        source_kind,
        occurrence_id,
        output,
        deadline_ms,
        ..
    } = &delivered[0].value
    else {
        panic!("expected a notification event, got {:?}", delivered[0]);
    };
    assert_eq!(source_id, &source.source_id);
    assert_eq!(*source_kind, NotificationSourceKind::WorkflowExit);
    assert_eq!(occurrence_id, "run-1");
    assert_eq!(output.message(), "approved");
    assert_eq!(*deadline_ms, env.deadline_ms);
    assert_eq!(delivered[0].session_id, f.session);
    assert!(
        f.candidates(true).is_empty() && f.candidates(false).is_empty(),
        "the receipt settled with the append"
    );
    // A replay is a duplicate and appends nothing; another body is refused.
    assert!(matches!(
        accept(&f, &sub, &env).unwrap(),
        NotificationOutcome::Ack(WorkflowNotificationAck::Duplicate)
    ));
    append(&f, &sub, &env);
    assert_eq!(events(&f).len(), 1);
    let mut other = env.clone();
    other.output = crate::session::WorkflowOutputPayload::new("rejected", vec![]);
    assert!(accept(&f, &sub, &other).is_err());
    assert_eq!(events(&f).len(), 1);
    cleanup(f);
}

#[test]
fn a_repeated_append_after_a_lost_response_adds_no_second_event() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = f.source(&a);
    let sub = follow(&f, &source);
    f.complete(&a, "run-1", None, WorkflowRunStatus::Completed, "approved");
    let (_, env) = f.candidates(false).remove(0);
    // An occurrence that was never accepted is not delivered.
    append(&f, &sub, &env);
    assert!(events(&f).is_empty());
    accept(&f, &sub, &env).unwrap();
    for _ in 0..3 {
        append(&f, &sub, &env);
    }
    assert_eq!(events(&f).len(), 1);
    cleanup(f);
}

#[test]
fn an_occurrence_whose_ancestry_holds_the_grant_is_loop_dropped() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = f.source(&a);
    let sub = follow(&f, &source);
    f.complete(&a, "run-1", None, WorkflowRunStatus::Completed, "approved");
    let (_, mut env) = f.candidates(false).remove(0);
    env.ancestry.push(format!("grant:{GRANT}"));
    assert!(matches!(
        accept(&f, &sub, &env).unwrap(),
        NotificationOutcome::Ack(WorkflowNotificationAck::LoopDropped)
    ));
    append(&f, &sub, &env);
    assert!(events(&f).is_empty());
    cleanup(f);
}

#[test]
fn unread_notifications_expire_after_seven_days_with_one_gap() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = f.source(&a);
    let sub = follow(&f, &source);
    f.complete(&a, "run-1", None, WorkflowRunStatus::Completed, "first");
    let (_, first) = f.candidates(false).remove(0);
    accept(&f, &sub, &first).unwrap();
    append(&f, &sub, &first);
    std::thread::sleep(std::time::Duration::from_millis(30));
    f.complete(&a, "run-2", None, WorkflowRunStatus::Completed, "second");
    let (_, second) = f.candidates(false).remove(0);
    accept(&f, &sub, &second).unwrap();
    append(&f, &sub, &second);
    assert_eq!(events(&f).len(), 2);
    // The deadline of the first has passed; the second still has time.
    f.store
        .notify(NotificationOperation::Sweep {
            now: first.deadline_ms,
        })
        .unwrap();
    let left = events(&f);
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(matches!(
        &left[0].value,
        OutsideEventValue::Notification { occurrence_id, .. } if occurrence_id == "run-2"
    ));
    assert!(matches!(
        &left[1].value,
        OutsideEventValue::Gap { dropped: 1, .. }
    ));
    // The gap is told once, and the second expires in its turn.
    f.store
        .notify(NotificationOperation::Sweep {
            now: first.deadline_ms,
        })
        .unwrap();
    assert_eq!(events(&f).len(), 2);
    f.store
        .notify(NotificationOperation::Sweep {
            now: second.deadline_ms,
        })
        .unwrap();
    let left = events(&f);
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left
        .iter()
        .all(|event| matches!(event.value, OutsideEventValue::Gap { dropped: 1, .. })));
    // The sweep finds them by deadline without reading the inbox.
    let plan: String = f
        .store
        .lock_connection("plan")
        .unwrap()
        .prepare("EXPLAIN QUERY PLAN SELECT agent_id,sequence FROM agent_inbox INDEXED BY agent_inbox_notification_deadline WHERE room_id='' AND json_extract(payload,'$.type')='notification' AND json_extract(payload,'$.value.deadline_ms')<=1 ORDER BY agent_id,sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(";");
    assert!(plan.contains("agent_inbox_notification_deadline"), "{plan}");
    cleanup(f);
}

#[test]
fn a_restart_retires_every_follow_but_those_of_a_surviving_grant() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = f.source(&a);
    let kept = follow(&f, &source);
    let mut other = kept.clone();
    other.subscription_id = "sub-grant-b".into();
    other.publication_id = "grant-b".into();
    other.endpoint_id = "grant-b".into();
    f.store
        .notify(NotificationOperation::Granted {
            grant: "grant-b".into(),
            attach: Box::new(NotificationOperation::RemoteAttach {
                subscription: other,
            }),
        })
        .unwrap();
    let NotificationOutcome::Retired(retired) = f
        .store
        .notify(NotificationOperation::RetireGrants {
            grant: None,
            keep_outside_agent_of: vec![GRANT.into()],
        })
        .unwrap()
    else {
        panic!("expected retired subscriptions")
    };
    assert_eq!(
        retired
            .iter()
            .map(|s| s.endpoint_id.as_str())
            .collect::<Vec<_>>(),
        ["grant-b"]
    );
    let (_, active, _) = f.store.notification_inventory("local").unwrap();
    assert_eq!(
        active
            .iter()
            .map(|s| s.subscription_id.as_str())
            .collect::<Vec<_>>(),
        [kept.subscription_id.as_str()]
    );
    cleanup(f);
}
