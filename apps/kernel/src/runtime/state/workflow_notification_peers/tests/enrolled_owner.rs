//! MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner registers and attaches the workflow
//! notifications of its room made before enrollment under `local`. The room keeps its `local`
//! identity: what the owner registers is stored under it and lists back in the same room.
use super::*;

const ENROLLED_OWNER: &str = "kernel-owner";

/// A `local` room on a kernel then enrolled for `ENROLLED_OWNER`, with a `guest` member.
fn enrolled_owner_room() -> (Fixture, KernelRuntimeState, String, String) {
    let mut f = Fixture::for_kernel("notification-home");
    let (source, _, _) = f.workflow("source");
    let (_, _, publication) = f.workflow("target");
    let (_, runtime, _) = peer(&mut f, "ws://127.0.0.1:1", "test-only");
    let mut room = runtime.owned.session_store.get_session(&f.session).unwrap();
    assert_eq!(room.owner_user_id(), "local");
    room.add_member(
        "guest",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    runtime.owned.session_store.write().restore_session(room);
    (f, runtime, source, publication)
}

fn enroll(runtime: &KernelRuntimeState) {
    let mut config = runtime.owned.config_projection.snapshot();
    config.cloud_relay.as_mut().unwrap().user_id = ENROLLED_OWNER.into();
    runtime.owned.config_projection.update(config);
}

fn register(f: &Fixture, workflow: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::RegisterWorkflowNotificationSource(
        RegisterWorkflowNotificationSourceRequest {
            session_id: f.session.clone(),
            workflow_ref: workflow.into(),
            exit_node_id: "exit-1".into(),
            enabled: true,
        },
    )
}

fn attach(f: &Fixture, source: &str, publication: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
        session_id: f.session.clone(),
        source_id: source.into(),
        publication_ref: publication.into(),
    })
}

fn list(
    runtime: &KernelRuntimeState,
    f: &Fixture,
    caller: &str,
) -> (
    Vec<WorkflowNotificationSourceSummary>,
    Vec<WorkflowNotificationSubscription>,
) {
    let LocalDaemonResponse::WorkflowNotifications {
        sources,
        subscriptions,
        ..
    } = runtime
        .execute_workflow_notification_request(
            LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                session_id: f.session.clone(),
            }),
            caller,
        )
        .unwrap()
    else {
        panic!()
    };
    (sources, subscriptions)
}

#[test]
fn the_enrolled_owner_registers_a_source_of_a_room_made_before_enrollment() {
    let (f, runtime, workflow, _) = enrolled_owner_room();
    enroll(&runtime);
    let registered =
        runtime.execute_workflow_notification_request(register(&f, &workflow), ENROLLED_OWNER);
    let Ok(LocalDaemonResponse::WorkflowNotificationSourceRegistered { source }) = registered
    else {
        panic!("{registered:?}")
    };
    assert_eq!(source.owner_user_id, "local", "the room keeps its identity");
    let (sources, _) = list(&runtime, &f, ENROLLED_OWNER);
    assert!(
        sources
            .iter()
            .any(|listed| listed.source_id == source.source_id && listed.available),
        "{sources:?}"
    );
    assert!(
        runtime
            .execute_workflow_notification_request(register(&f, &workflow), "guest")
            .is_err(),
        "a member does not register the owner's sources"
    );
    drop(runtime);
    cleanup(f);
}

#[test]
fn the_enrolled_owner_attaches_a_notification_in_a_room_made_before_enrollment() {
    let (f, runtime, workflow, publication) = enrolled_owner_room();
    let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = runtime
        .execute_workflow_notification_request(register(&f, &workflow), "local")
        .unwrap()
    else {
        panic!()
    };
    enroll(&runtime);
    assert!(
        runtime
            .execute_workflow_notification_request(
                attach(&f, &source.source_id, &publication),
                "guest"
            )
            .is_err(),
        "a member does not attach the owner's notifications"
    );
    let attached = runtime.execute_workflow_notification_request(
        attach(&f, &source.source_id, &publication),
        ENROLLED_OWNER,
    );
    let Ok(LocalDaemonResponse::WorkflowNotificationAttached { subscription }) = attached else {
        panic!("{attached:?}")
    };
    assert_eq!(
        subscription.owner_user_id, "local",
        "the room keeps its identity"
    );
    let (_, subscriptions) = list(&runtime, &f, ENROLLED_OWNER);
    assert!(
        subscriptions
            .iter()
            .any(|listed| listed.subscription_id == subscription.subscription_id),
        "{subscriptions:?}"
    );
    drop(runtime);
    cleanup(f);
}
