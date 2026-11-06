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
