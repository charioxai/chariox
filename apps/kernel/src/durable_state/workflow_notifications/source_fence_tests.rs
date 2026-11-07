//! MP-08 / MP-09 / MP-10 / MP-11: the source kernel decides a peer's attach or detach by the
//! generation it carries, never by arrival order, on the real durable writer and across a
//! restart. Supplementary regressions, not live acceptance.
use super::tests::{cleanup, Fixture};
use super::*;
use crate::session::WorkflowRunStatus;

/// A follow of the fixture's `source` workflow by another kernel, as this source stores it.
fn remote_follow(f: &mut Fixture) -> (String, WorkflowNotificationSubscription) {
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("follower");
    let source = f.source(&a);
    let local = f.attach(&source, &b, &bp);
    f.store
        .notify(NotificationOperation::Detach {
            subscription_id: local.subscription_id.clone(),
            owner: local.owner_user_id.clone(),
            kernel: local.target_kernel_id.clone(),
        })
        .unwrap();
    let mut sub = local;
    sub.subscription_id = "remote-follow".into();
    sub.target_kernel_id = "destination".into();
    (a, sub)
}

/// `None` when the source applied the attach, else the newest generation it has seen.
fn attach(f: &Fixture, sub: &WorkflowNotificationSubscription, generation: u64) -> Option<u64> {
    match f
        .store
        .notify(NotificationOperation::SourceAttach {
            subscription: sub.clone(),
            generation,
        })
        .unwrap()
    {
        NotificationOutcome::Subscription(_) => None,
        NotificationOutcome::Superseded { generation } => Some(generation),
        other => panic!("{other:?}"),
    }
}

fn detach(f: &Fixture, sub: &WorkflowNotificationSubscription, generation: u64) -> Option<u64> {
    match f
        .store
        .notify(NotificationOperation::SourceDetach {
            subscription_id: sub.subscription_id.clone(),
            owner: sub.owner_user_id.clone(),
            kernel: sub.target_kernel_id.clone(),
            generation,
        })
        .unwrap()
    {
        NotificationOutcome::Swept => None,
        NotificationOutcome::Superseded { generation } => Some(generation),
        other => panic!("{other:?}"),
    }
}

fn restart(f: &mut Fixture) {
    let root = f.root.clone();
    drop(std::mem::replace(
        &mut f.store,
        DurableKernelStateStore::open_owned(root.join("restart-placeholder.sqlite")).unwrap(),
    ));
    f.store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
}

fn followed_queue(f: &Fixture) -> Option<String> {
    f.store
        .notification_inventory("local")
        .unwrap()
        .1
        .into_iter()
        .find(|s| s.subscription_id == "remote-follow")
        .map(|s| s.queue_id)
}

fn run(f: &mut Fixture, workflow: &str, id: &str) {
    f.complete(workflow, id, None, WorkflowRunStatus::Completed, "out");
}

#[test]
fn an_older_unsubscribe_is_ignored_after_a_newer_subscribe_also_across_a_restart() {
    let mut f = Fixture::new();
    let (a, sub) = remote_follow(&mut f);
    assert_eq!(attach(&f, &sub, 2), None);
    restart(&mut f);
    assert_eq!(detach(&f, &sub, 1), Some(2), "the older cleanup was applied");
    assert!(followed_queue(&f).is_some());
    // A run ends in the interval the stale cleanup would have left the follow disabled.
    run(&mut f, &a, "inside-the-window");
    assert_eq!(f.store.receipts(), 1, "the occurrence has no receipt");
    cleanup(f);
}

#[test]
fn an_older_subscribe_is_ignored_after_a_newer_unsubscribe_also_across_a_restart() {
    let mut f = Fixture::new();
    let (a, sub) = remote_follow(&mut f);
    assert_eq!(attach(&f, &sub, 1), None);
    assert_eq!(detach(&f, &sub, 3), None);
    restart(&mut f);
    assert_eq!(attach(&f, &sub, 2), Some(3), "the older install was applied");
    assert!(followed_queue(&f).is_none());
    run(&mut f, &a, "after-the-detach");
    assert_eq!(f.store.receipts(), 0, "a detached follow was notified");
    cleanup(f);
}

#[test]
fn two_attaches_arriving_out_of_order_leave_the_newer_one() {
    let mut f = Fixture::new();
    let (_, mut sub) = remote_follow(&mut f);
    sub.queue_id = "newer-queue".into();
    assert_eq!(attach(&f, &sub, 4), None);
    sub.queue_id = "older-queue".into();
    assert_eq!(attach(&f, &sub, 3), Some(4));
    assert_eq!(followed_queue(&f).as_deref(), Some("newer-queue"));
    cleanup(f);
}

#[test]
fn an_unsubscribe_for_a_follow_the_source_never_held_still_fences_an_older_subscribe() {
    let mut f = Fixture::new();
    let (_, sub) = remote_follow(&mut f);
    assert_eq!(detach(&f, &sub, 5), None);
    assert_eq!(attach(&f, &sub, 4), Some(5));
    assert!(followed_queue(&f).is_none());
    cleanup(f);
}

#[test]
fn the_same_generation_asked_again_is_the_same_state_and_applied_again() {
    let mut f = Fixture::new();
    let (_, sub) = remote_follow(&mut f);
    assert_eq!(attach(&f, &sub, 2), None);
    assert_eq!(attach(&f, &sub, 2), None);
    assert_eq!(detach(&f, &sub, 3), None);
    assert_eq!(detach(&f, &sub, 3), None);
    assert!(followed_queue(&f).is_none());
    cleanup(f);
}
