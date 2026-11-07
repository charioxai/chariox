//! MP-08 / MP-10 / MP-11: shared access cannot borrow owner cross-kernel routing.
use super::*;

fn setup() -> (
    Fixture,
    KernelRuntimeState,
    WorkflowNotificationSource,
    String,
) {
    let mut f = Fixture::for_kernel("notification-home");
    let (workflow, _, _) = f.workflow("source");
    let (_, _, publication) = f.workflow("target");
    let (_, runtime, _) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
    let source = f.source(&workflow);
    let mut remote = summaries(
        &runtime.owned.session_store.read(),
        std::slice::from_ref(&source),
    )
    .remove(0);
    remote.source_id = "remote-source".into();
    remote.kernel_id = "other-home".into();
    runtime
        .owned
        .durable_state_store
        .notify(NotificationOperation::Cache {
            owner: "local".into(),
            kernel: remote.kernel_id.clone(),
            sources: vec![remote],
        })
        .unwrap();
    (f, runtime, source, publication)
}

fn attach(f: &Fixture, source: &str, publication: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
        session_id: f.session.clone(),
        source_id: source.into(),
        publication_ref: publication.into(),
    })
}

#[tokio::test]
async fn shared_access_notification_list_excludes_remote_cache_without_refresh() {
    let (f, runtime, source, _) = setup();
    let request = LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
        session_id: f.session.clone(),
    });
    let grant = runtime.insert_access_grant_for_test(&f.session);
    let shared = runtime.with_external_command_authority(Some((&grant, &request)));
    shared.authorize_current_external_command().unwrap();
    let before = f.store.notification_cached_sources("local").unwrap();
    let LocalDaemonResponse::WorkflowNotifications { sources, .. } = shared
        .execute_workflow_notification_command(request.clone(), "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        sources
            .iter()
            .filter(|s| s.kind == crate::local::NotificationSourceKind::WorkflowExit)
            .map(|s| s.source_id.as_str())
            .collect::<Vec<_>>(),
        vec![source.source_id.as_str()],
        "MP-11: shared list must expose local sources only"
    );
    assert_eq!(
        f.store.notification_cached_sources("local").unwrap(),
        before,
        "MP-11: shared list must not refresh owner peer inventory"
    );
    // Ordinary owner and sudo metadata retain the full cached inventory.
    for owner in [
        runtime.clone(),
        runtime.with_external_command_authority(Some(("sudo:test", &request))),
    ] {
        let LocalDaemonResponse::WorkflowNotifications { sources, .. } = owner
            .execute_workflow_notification_request(request.clone(), "local")
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            sources
                .iter()
                .filter(|s| s.kind == crate::local::NotificationSourceKind::WorkflowExit)
                .count(),
            2
        );
    }
    drop(shared);
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn shared_access_notification_remote_attach_rejects_before_persistence() {
    let (f, runtime, source, publication) = setup();
    let request = attach(&f, "remote-source", &publication);
    let grant = runtime.insert_access_grant_for_test(&f.session);
    let shared = runtime.with_external_command_authority(Some((&grant, &request)));
    shared.authorize_current_external_command().unwrap();
    assert!(shared
        .execute_workflow_notification_command(request, "local")
        .await
        .is_err());
    assert!(
        f.store
            .notification_inventory("local")
            .unwrap()
            .1
            .is_empty(),
        "MP-11: forbidden remote attach must not persist a subscription"
    );
    let local = attach(&f, &source.source_id, &publication);
    let shared = runtime.with_external_command_authority(Some((&grant, &local)));
    let LocalDaemonResponse::WorkflowNotificationAttached { subscription } = shared
        .execute_workflow_notification_command(local, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(subscription.source_kernel_id, "notification-home");
    drop(shared);
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn shared_access_notification_remote_detach_rejects_before_mutation() {
    let (f, runtime, source, publication) = setup();
    let local = attach(&f, &source.source_id, &publication);
    let LocalDaemonResponse::WorkflowNotificationAttached { mut subscription } = runtime
        .execute_workflow_notification_command(local, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    subscription.subscription_id = "owner-remote-subscription".into();
    subscription.source_id = "remote-source".into();
    subscription.source_kernel_id = "other-home".into();
    f.store
        .notify(NotificationOperation::RemoteAttach {
            subscription: subscription.clone(),
        })
        .unwrap();
    let request =
        LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
            session_id: f.session.clone(),
            subscription_id: subscription.subscription_id.clone(),
        });
    let grant = runtime.insert_access_grant_for_test(&f.session);
    let shared = runtime.with_external_command_authority(Some((&grant, &request)));
    shared.authorize_current_external_command().unwrap();
    assert!(
        shared
            .execute_workflow_notification_command(request.clone(), "local")
            .await
            .is_err(),
        "MP-11: shared caller cannot detach owner's remote subscription"
    );
    assert!(
        f.store
            .notification_inventory("local")
            .unwrap()
            .1
            .contains(&subscription),
        "MP-11: denied detach must not change durable state"
    );
    let list = LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
        session_id: f.session.clone(),
    });
    let LocalDaemonResponse::WorkflowNotifications { subscriptions, .. } = shared
        .execute_workflow_notification_request(list, "local")
        .unwrap()
    else {
        panic!()
    };
    assert!(subscriptions
        .iter()
        .all(|s| s.source_kernel_id == "notification-home"));
    // Owner detach remains available, including best-effort peer unsubscribe.
    assert!(runtime
        .execute_workflow_notification_command(request, "local")
        .await
        .is_ok());
    assert!(!f
        .store
        .notification_inventory("local")
        .unwrap()
        .1
        .contains(&subscription));
    drop(shared);
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn a_remote_attach_the_source_kernel_does_not_answer_leaves_no_subscription() {
    let (f, runtime, _source, publication) = setup();
    let request = attach(&f, "remote-source", &publication);
    assert!(runtime
        .execute_workflow_notification_command(request, "local")
        .await
        .is_err());
    assert!(
        f.store
            .notification_inventory("local")
            .unwrap()
            .1
            .is_empty(),
        "the refused attach left an active subscription the source kernel never installed"
    );
    drop(runtime);
    cleanup(f);
}

/// The subscription an attach to the remote source builds, installed as already working.
async fn working_remote_follow(
    f: &Fixture,
    runtime: &KernelRuntimeState,
    source: &WorkflowNotificationSource,
    publication: &str,
    change: impl FnOnce(&mut WorkflowNotificationSubscription),
) -> WorkflowNotificationSubscription {
    let local = attach(f, &source.source_id, publication);
    let LocalDaemonResponse::WorkflowNotificationAttached { mut subscription } = runtime
        .execute_workflow_notification_command(local, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    subscription.subscription_id = "working-remote-follow".into();
    subscription.source_id = "remote-source".into();
    subscription.source_kernel_id = "other-home".into();
    change(&mut subscription);
    f.store
        .notify(NotificationOperation::RemoteAttach {
            subscription: subscription.clone(),
        })
        .unwrap();
    subscription
}

fn due_reconciles(
    f: &Fixture,
    now: u64,
) -> Vec<crate::durable_state::workflow_notifications::remote_install::RemoteReconcile> {
    match f
        .store
        .notify(NotificationOperation::ClaimRemoteReconciles { now, limit: 8 })
        .unwrap()
    {
        NotificationOutcome::Reconciles(due) => due,
        _ => panic!("a claim answers with the due reconciliations"),
    }
}

#[tokio::test]
async fn a_repeated_remote_attach_the_peer_does_not_answer_keeps_the_working_follow() {
    let (f, runtime, source, publication) = setup();
    let working = working_remote_follow(&f, &runtime, &source, &publication, |_| {}).await;
    assert!(runtime
        .execute_workflow_notification_command(attach(&f, "remote-source", &publication), "local")
        .await
        .is_err());
    let (_, active, _) = f.store.notification_inventory("local").unwrap();
    assert!(
        active.contains(&working),
        "the unanswered repeat disabled the follow that was already working"
    );
    assert!(
        due_reconciles(&f, u64::MAX / 2).is_empty(),
        "the source holds the same subscription either way: nothing to reconcile"
    );
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn a_refused_repeat_attach_restores_the_prior_follow_and_asks_the_source_to_match_it() {
    let (f, runtime, source, publication) = setup();
    let working = working_remote_follow(&f, &runtime, &source, &publication, |s| {
        s.workflow_id = "workflow-the-first-attach-named".into();
    })
    .await;
    assert!(runtime
        .execute_workflow_notification_command(attach(&f, "remote-source", &publication), "local")
        .await
        .is_err());
    let (_, active, _) = f.store.notification_inventory("local").unwrap();
    assert_eq!(
        active
            .iter()
            .filter(|s| s.subscription_id == working.subscription_id)
            .collect::<Vec<_>>(),
        vec![&working],
        "the refused repeat must leave the prior fields, not the attempted ones"
    );
    let due = due_reconciles(&f, 1_000);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].subscription_id, working.subscription_id);
    assert_eq!(due[0].source_kernel_id, "other-home");
    assert!(
        due[0].desired.as_ref() == Some(&working),
        "the source may have installed the attempt: it is asked to hold the prior one"
    );
    assert!(
        due_reconciles(&f, 2_000).is_empty(),
        "a claimed reconciliation waits for its retry window"
    );
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn a_first_remote_attach_the_peer_does_not_answer_is_reconciled_as_absent() {
    let (f, runtime, _source, publication) = setup();
    assert!(runtime
        .execute_workflow_notification_command(attach(&f, "remote-source", &publication), "local")
        .await
        .is_err());
    assert!(f
        .store
        .notification_inventory("local")
        .unwrap()
        .1
        .is_empty());
    let due = due_reconciles(&f, u64::MAX / 2);
    assert_eq!(
        due.len(),
        1,
        "the source may hold a row nothing here knows of"
    );
    assert!(due[0].desired.is_none());
    assert_eq!(due[0].source_kernel_id, "other-home");
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn a_reconciliation_the_source_never_matches_is_parked_after_its_attempts() {
    let (f, runtime, _source, publication) = setup();
    assert!(runtime
        .execute_workflow_notification_command(attach(&f, "remote-source", &publication), "local")
        .await
        .is_err());
    let mut asked = 0;
    for round in 1..=40u64 {
        asked += due_reconciles(&f, round * 3_600_000).len();
    }
    assert_eq!(
        asked, 20,
        "an unmatched reconciliation is asked a bounded number of times"
    );
    assert!(due_reconciles(&f, u64::MAX / 2).is_empty());
    assert_eq!(
        f.store.unconfirmed_installs(),
        1,
        "the entry was forgotten while its source kernel was offline"
    );
    f.store
        .notify(NotificationOperation::RemoteKernelsConnected {
            kernels: vec!["other-home".into()],
        })
        .unwrap();
    assert_eq!(
        due_reconciles(&f, 1).len(),
        1,
        "the source kernel connected again and is asked at once"
    );
    drop(runtime);
    cleanup(f);
}

#[test]
fn a_kernel_that_appears_in_the_inventory_is_reported_once() {
    let projection = crate::runtime::projection::RemoteRelayInventoryProjectionStore::default();
    let presence = |id: &str| chariox_relay::protocol::RelayKernelPresence {
        kernel_id: id.into(),
        machine_id: "machine".into(),
        machine_alias: None,
        relay_alias: None,
        kernel_alias: None,
        available_providers: vec![],
        provider_accounts: vec![],
        capabilities: vec![],
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
        public_key: String::new(),
    };
    projection.update(vec![], vec![presence("a")]);
    assert_eq!(projection.take_appeared_kernels(), vec!["a".to_string()]);
    projection.update(vec![], vec![presence("a")]);
    assert!(projection.take_appeared_kernels().is_empty());
    projection.update(vec![], vec![]);
    projection.update(vec![], vec![presence("a"), presence("b")]);
    assert_eq!(
        projection.take_appeared_kernels(),
        vec!["a".to_string(), "b".to_string()]
    );
}

#[tokio::test]
async fn an_install_left_unsettled_by_a_stopped_kernel_restores_the_prior_follow() {
    let (f, runtime, source, publication) = setup();
    let working = working_remote_follow(&f, &runtime, &source, &publication, |s| {
        s.workflow_id = "workflow-the-first-attach-named".into();
    })
    .await;
    // The kernel stopped after the attach committed and before the source answered.
    let mut attempt = working.clone();
    attempt.workflow_id = publication.clone();
    f.store
        .notify(NotificationOperation::AttachToRemoteSource {
            subscription: attempt.clone(),
        })
        .unwrap();
    f.store
        .notify(NotificationOperation::SettleInterruptedRemoteInstalls { now: 0 })
        .unwrap();
    assert!(
        due_reconciles(&f, u64::MAX / 2).is_empty(),
        "an install that has just begun is still in flight"
    );
    f.store
        .notify(NotificationOperation::SettleInterruptedRemoteInstalls { now: u64::MAX / 2 })
        .unwrap();
    let (_, active, _) = f.store.notification_inventory("local").unwrap();
    assert!(active.contains(&working), "the prior follow is back");
    assert!(!active.contains(&attempt));
    let due = due_reconciles(&f, u64::MAX / 2);
    assert_eq!(due.len(), 1);
    assert!(due[0].desired.as_ref() == Some(&working));
    drop(runtime);
    cleanup(f);
}

/// A repeat attach is cut off after its local commit, the kernel restarts inside the grace
/// period and the user detaches: the later settlement must not bring the follow back.
async fn detach_after_an_interrupted_repeat_attach(attempt_differs: bool) {
    let (f, runtime, source, publication) = setup();
    let working = working_remote_follow(&f, &runtime, &source, &publication, |s| {
        s.workflow_id = "workflow-the-first-attach-named".into();
    })
    .await;
    let mut attempt = working.clone();
    if attempt_differs {
        attempt.workflow_id = publication.clone();
    }
    f.store
        .notify(NotificationOperation::AttachToRemoteSource {
            subscription: attempt.clone(),
        })
        .unwrap();
    f.store
        .notify(NotificationOperation::SettleInterruptedRemoteInstalls { now: 0 })
        .unwrap();
    f.store
        .notify(NotificationOperation::Detach {
            subscription_id: working.subscription_id.clone(),
            owner: "local".into(),
            kernel: working.target_kernel_id.clone(),
        })
        .unwrap();
    f.store
        .notify(NotificationOperation::SettleInterruptedRemoteInstalls { now: u64::MAX / 2 })
        .unwrap();
    let (_, active, _) = f.store.notification_inventory("local").unwrap();
    assert!(
        !active.contains(&working) && !active.contains(&attempt),
        "the settlement of the interrupted install restored a follow the user detached"
    );
    assert!(
        due_reconciles(&f, u64::MAX / 2)
            .iter()
            .all(|owed| owed.desired.is_none()),
        "the detach left install work that would subscribe the source again"
    );
    drop(runtime);
    cleanup(f);
}

#[tokio::test]
async fn a_detach_supersedes_an_interrupted_repeat_attach_that_changed_the_follow() {
    detach_after_an_interrupted_repeat_attach(true).await;
}

#[tokio::test]
async fn a_detach_supersedes_an_interrupted_repeat_attach_that_matched_the_follow() {
    detach_after_an_interrupted_repeat_attach(false).await;
}
