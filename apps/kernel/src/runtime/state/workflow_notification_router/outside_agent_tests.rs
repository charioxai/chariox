//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17, ON-18): a grant follows the owner's notification
//! sources through the real router and receives their occurrences as events. Supplementary
//! regressions, not live acceptance.
use super::tests::router;
use super::*;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
use crate::runtime::command::KernelCommand;
use crate::runtime::router::CommandRouter;

async fn dispatch(
    router: &CommandRouter,
    grant: &str,
    request: serde_json::Value,
) -> Result<LocalDaemonResponse, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(request).unwrap();
    let mut command = KernelCommand::from_local_request(
        format!("outside-source-{:016x}", rand::random::<u64>()),
        None,
        None,
        &request,
    );
    command.caller.caller_id = grant.into();
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.user_id = Some(crate::session::DEFAULT_LOCAL_USER_ID.into());
    router.dispatch(command, request).await
}
fn register(
    kernel: &KernelRuntimeState,
    f: &Fixture,
    workflow: &str,
) -> WorkflowNotificationSource {
    let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = kernel
        .execute_workflow_notification_request(
            LocalDaemonRequest::RegisterWorkflowNotificationSource(
                RegisterWorkflowNotificationSourceRequest {
                    session_id: f.session.clone(),
                    workflow_ref: workflow.into(),
                    enabled: true,
                    exit_node_id: "exit-1".into(),
                },
            ),
            "local",
        )
        .unwrap()
    else {
        panic!("expected a registered source")
    };
    source
}
fn restore(kernel: &KernelRuntimeState, f: &Fixture) {
    kernel
        .owned
        .session_store
        .write()
        .restore_session(f.sessions.get_session(&f.session).unwrap());
}
async fn wait(router: &CommandRouter, grant: &str, since: u64) -> Vec<OutsideEvent> {
    match dispatch(
        router,
        grant,
        serde_json::json!({"WaitEvents":{"since":since,"timeout_ms":5000}}),
    )
    .await
    .unwrap()
    {
        LocalDaemonResponse::EventsListed { events, .. } => events,
        other => panic!("expected events, got {other:?}"),
    }
}
fn refusal(result: Result<LocalDaemonResponse, DaemonError>) -> String {
    match result {
        Err(DaemonError::MachineRequest { code, message }) => {
            assert_eq!(code, "events_refused");
            message
        }
        other => panic!("expected an events refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_grant_waits_for_a_workflow_exit_and_reads_each_occurrence_until_acknowledged() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);

    let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}}),
    )
    .await
    .unwrap() else {
        panic!("expected a subscription")
    };
    assert_eq!(subscription.sources.len(), 1);
    assert_eq!(subscription.sources[0].source_id, source.source_id);
    assert_eq!(subscription.sources[0].display_name, "reviewer");
    assert!(subscription.sources[0].available);
    assert!(subscription.targets.is_empty());

    for run in ["run-1", "run-2"] {
        f.complete(
            &a,
            run,
            None,
            crate::session::WorkflowRunStatus::Completed,
            run,
        );
    }
    restore(&kernel, &f);
    kernel.owned.route_workflow_notifications();
    let events = wait(&router, &grant, 0).await;
    let delivered: Vec<_> = events
        .iter()
        .map(|event| match &event.value {
            OutsideEventValue::Notification {
                occurrence_id,
                output,
                ..
            } => (occurrence_id.clone(), output.message().to_owned()),
            other => panic!("expected notifications, got {other:?}"),
        })
        .collect();
    assert_eq!(
        delivered,
        vec![
            ("run-1".into(), "run-1".into()),
            ("run-2".into(), "run-2".into())
        ]
    );
    // Listing again before the acknowledgement returns them again; the acknowledgement ends it.
    let LocalDaemonResponse::EventsListed { events: again, .. } = dispatch(
        &router,
        &grant,
        serde_json::json!({"ListEvents":{"since":0}}),
    )
    .await
    .unwrap() else {
        panic!("expected events")
    };
    assert_eq!(again.len(), 2);
    let last = events.last().unwrap().cursor;
    dispatch(
        &router,
        &grant,
        serde_json::json!({"AckEvents":{"cursor":last}}),
    )
    .await
    .unwrap();
    let LocalDaemonResponse::EventsListed { events: rest, .. } =
        dispatch(&router, &grant, serde_json::json!({"ListEvents":{}}))
            .await
            .unwrap()
    else {
        panic!("expected events")
    };
    assert!(rest.is_empty());
    // Delivery is once per occurrence: another pass appends nothing.
    kernel.owned.route_workflow_notifications();
    assert!(matches!(
        dispatch(&router, &grant, serde_json::json!({"ListEvents":{}})).await.unwrap(),
        LocalDaemonResponse::EventsListed { events, .. } if events.is_empty()
    ));
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_grant_follows_the_owner_action_source_and_names_the_decision() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let session = kernel.owned.session_store.get_session(&f.session).unwrap();
    let source = kernel
        .owned
        .ensure_owner_action_notification_source(&session)
        .unwrap();
    let grant = kernel.insert_access_grant_for_test(&f.session);
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}}),
    )
    .await
    .unwrap();
    kernel
        .owned
        .durable_state_store
        .notify(NotificationOperation::EmitOwnerAction {
            source_id: source.source_id.clone(),
            occurrence_id: "question-1".into(),
            output: crate::session::WorkflowOutputPayload::new(
                serde_json::json!({"title":"Agent needs your action","room_id":f.session,"agent_id":"agent","task_id":"task-1","interaction_id":"question-1"}).to_string(),
                Vec::new(),
            ),
            at: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    kernel.owned.route_workflow_notifications();
    let events = wait(&router, &grant, 0).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].refs.interaction_id.as_deref(), Some("question-1"));
    assert_eq!(events[0].refs.task_id.as_deref(), Some("task-1"));
    assert_eq!(events[0].agent_id.as_deref(), Some("agent"));
    assert_eq!(events[0].session_id, f.session);
    assert!(matches!(
        &events[0].value,
        OutsideEventValue::Notification {
            source_kind: NotificationSourceKind::OwnerAction,
            ..
        }
    ));
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_grant_follows_only_the_owners_sources_of_this_kernel() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let own = register(&kernel, &f, &a);
    // A source of another owner, and one the kernel never heard of, are not available, and
    // the refusal does not tell them apart.
    let mut stranger = own.clone();
    stranger.source_id = "wf_source_stranger".into();
    stranger.owner_user_id = "stranger".into();
    kernel
        .owned
        .durable_state_store
        .notify(NotificationOperation::Register(SourceAdmission {
            source: stranger,
            workflow_json: serde_json::to_string(
                &f.sessions.resolve_workflow_ref(&f.session, &a).unwrap(),
            )
            .unwrap(),
        }))
        .unwrap();
    let grant = kernel.insert_access_grant_for_test(&f.session);
    let unknown = refusal(
        dispatch(
            &router,
            &grant,
            serde_json::json!({"SubscribeEvents":{"targets":[],"sources":["wf_source_unknown"]}}),
        )
        .await,
    );
    assert_eq!(unknown, "source not available");
    assert_eq!(
        refusal(
            dispatch(
                &router,
                &grant,
                serde_json::json!({"SubscribeEvents":{"targets":[],"sources":["wf_source_stranger"]}}),
            )
            .await
        ),
        unknown
    );
    // One bad source follows nothing: the good one named with it is not attached.
    assert_eq!(
        refusal(
            dispatch(
                &router,
                &grant,
                serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[own.source_id,"wf_source_unknown"]}}),
            )
            .await
        ),
        unknown
    );
    assert!(kernel
        .followed_notification_sources(&grant)
        .unwrap()
        .is_empty());
    // A reach caller (a Room agent's run) never follows sources through this request.
    let reach = crate::runtime::kernel_gateway::Origin {
        grant_id: "reach:run-1".into(),
        harness: "test".into(),
    };
    let request = LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
        targets: Vec::new(),
        sources: vec![own.source_id.clone()],
    });
    assert!(router
        .kernel_gateway()
        .event_request(&reach, &request, None)
        .await
        .is_err());
    assert!(kernel
        .owned
        .durable_state_store
        .notification_inventory("local")
        .unwrap()
        .1
        .is_empty());
    // More sources than a grant may follow are refused before any is looked up.
    let many: Vec<_> = (0..65).map(|n| format!("wf_source_{n}")).collect();
    assert_eq!(
        refusal(
            dispatch(
                &router,
                &grant,
                serde_json::json!({"SubscribeEvents":{"targets":[],"sources":many}}),
            )
            .await
        ),
        "event source limit"
    );
    drop((router, kernel));
    cleanup(f);
}

fn notifications(kernel: &KernelRuntimeState, grant: &str) -> Vec<String> {
    kernel
        .owned
        .durable_state_store
        .outside_list(grant, Some(0), 100)
        .unwrap()
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.value {
            OutsideEventValue::Notification { occurrence_id, .. } => Some(occurrence_id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_revoked_grant_receives_nothing_more_even_for_an_accepted_occurrence() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}}),
    )
    .await
    .unwrap();
    f.complete(
        &a,
        "before",
        None,
        crate::session::WorkflowRunStatus::Completed,
        "before",
    );
    restore(&kernel, &f);
    kernel.owned.route_workflow_notifications();
    assert_eq!(notifications(&kernel, &grant), ["before"]);
    // This one is accepted by the router but not yet appended when the grant ends.
    f.complete(
        &a,
        "accepted",
        None,
        crate::session::WorkflowRunStatus::Completed,
        "accepted",
    );
    restore(&kernel, &f);
    let (sub, env) = kernel
        .owned
        .durable_state_store
        .notification_candidates(false, crate::session::unix_epoch_ms(), 8)
        .unwrap()
        .remove(0);
    kernel
        .owned
        .durable_state_store
        .notify(NotificationOperation::Accept {
            subscription: sub,
            envelope: env,
        })
        .unwrap();
    f.complete(
        &a,
        "after",
        None,
        crate::session::WorkflowRunStatus::Completed,
        "after",
    );
    restore(&kernel, &f);
    kernel
        .revoke_kernel_access(Some("local"), Some(&grant), "owner")
        .unwrap();
    kernel.owned.route_workflow_notifications();
    kernel.owned.route_workflow_notifications();
    assert_eq!(notifications(&kernel, &grant), ["before"]);
    let part = kernel
        .owned
        .durable_state_store
        .outside_part(&grant)
        .unwrap()
        .unwrap();
    assert!(part.ended.is_some(), "the grant's stream ended with it");
    let (_, subscriptions, _) = kernel
        .owned
        .durable_state_store
        .notification_inventory("local")
        .unwrap();
    assert!(subscriptions.is_empty(), "its follows ended with it");
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn unsubscribing_a_source_stops_its_delivery_and_following_again_resumes_it() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    let follow = serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}});
    dispatch(&router, &grant, follow.clone()).await.unwrap();
    let complete = |f: &mut Fixture, run: &str| {
        f.complete(
            &a,
            run,
            None,
            crate::session::WorkflowRunStatus::Completed,
            run,
        );
        restore(&kernel, f);
        kernel.owned.route_workflow_notifications();
    };
    complete(&mut f, "one");
    let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"sources":[source.source_id]}}),
    )
    .await
    .unwrap() else {
        panic!("expected a subscription")
    };
    assert!(subscription.sources.is_empty());
    complete(&mut f, "two");
    // What arrived stays readable; what came after the unsubscribe does not arrive.
    assert_eq!(notifications(&kernel, &grant), ["one"]);
    // Naming a source that is not followed changes nothing; asking for nothing is refused.
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"sources":["wf_source_unknown"]}}),
    )
    .await
    .unwrap();
    assert!(
        dispatch(&router, &grant, serde_json::json!({"UnsubscribeEvents":{}}))
            .await
            .is_err()
    );
    let LocalDaemonResponse::EventSubscription { subscription } =
        dispatch(&router, &grant, follow).await.unwrap()
    else {
        panic!("expected a subscription")
    };
    assert_eq!(subscription.sources.len(), 1);
    complete(&mut f, "three");
    assert_eq!(notifications(&kernel, &grant), ["one", "three"]);
    // Ending the whole subscription ends every follow with it.
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"all":true}}),
    )
    .await
    .unwrap();
    complete(&mut f, "four");
    assert_eq!(notifications(&kernel, &grant), ["one", "three"]);
    drop((router, kernel));
    cleanup(f);
}
