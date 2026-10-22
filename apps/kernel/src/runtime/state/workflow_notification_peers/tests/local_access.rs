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
    let mut remote = summary(&source);
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
        delivery_mode: NotificationDeliveryMode::Queue,
        session_id: f.session.clone(),
        source_id: source.into(),
        publication_ref: publication.into(),
        queue_ref: None,
        ttl_days: 7,
        events: WorkflowNotificationEvents::Both,
        filters: serde_json::Value::Null,
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
        assert_eq!(sources.len(), 2);
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
