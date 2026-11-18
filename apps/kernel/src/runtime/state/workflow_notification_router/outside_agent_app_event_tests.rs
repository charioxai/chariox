//! MP-08 / MP-09 / MP-10 / MP-11 (GE-07): a route of an installed App is a source a grant lists,
//! follows and receives the event text from, through the real router. Supplementary regressions,
//! not live acceptance.
use super::outside_agent_tests::{dispatch, wait};
use super::tests::router;
use super::*;
use crate::durable_state::app_inbox::AppInboxOperation;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
use chariox_app_runtime::app_inbox::InboxRoute;

async fn listed(
    router: &crate::runtime::router::CommandRouter,
    grant: &str,
) -> Vec<WorkflowNotificationSourceSummary> {
    match dispatch(
        router,
        grant,
        serde_json::json!({"ListNotificationSources":{}}),
    )
    .await
    .unwrap()
    {
        LocalDaemonResponse::NotificationSources { sources } => sources,
        other => panic!("expected notification sources, got {other:?}"),
    }
}

fn arrive(kernel: &KernelRuntimeState, occurrence: &str, text: &str) {
    arrive_with(kernel, occurrence, serde_json::json!({"text": text}));
}

fn arrive_with(kernel: &KernelRuntimeState, occurrence: &str, payload: serde_json::Value) {
    kernel
        .owned
        .durable_state_store
        .app_inbox(AppInboxOperation::Accept {
            owner: "local".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
            occurrence_id: occurrence.into(),
            payload,
            generation: 1,
            now_ms: crate::session::unix_epoch_ms(),
        })
        .unwrap();
}

fn create_route(store: &crate::durable_state::DurableKernelStateStore) {
    store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: InboxRoute {
                route_id: "mail".into(),
                owner_id: "local".into(),
                installation_id: "installed".into(),
                event_name: "received".into(),
                source_event_type: "dev.chariox.dummy/dummy.test".into(),
                source_event_version: 1,
                active: true,
                paused: false,
                app_managed: false,
                source: None,
            },
            now_ms: 1,
        })
        .unwrap();
}

#[tokio::test]
async fn an_installed_apps_route_is_listed_followed_and_delivers_the_event_text() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let store = kernel.owned.durable_state_store.clone();
    crate::durable_state::app_state::fixture_inbox_installation(&store, "local");
    let grant = kernel.insert_access_grant_for_test(&f.session);
    // An installed App with no route has no event to follow.
    assert!(listed(&router, &grant)
        .await
        .iter()
        .all(|s| s.kind != NotificationSourceKind::AppEvent));
    create_route(&store);
    let sources: Vec<_> = listed(&router, &grant)
        .await
        .into_iter()
        .filter(|s| s.kind == NotificationSourceKind::AppEvent)
        .collect();
    assert_eq!(sources.len(), 1, "{sources:?}");
    assert_eq!(
        (sources[0].name.as_str(), sources[0].display_name.as_str()),
        ("received", "Received")
    );
    assert!(sources[0].available);
    // Listing again names the same source.
    assert_eq!(
        listed(&router, &grant)
            .await
            .iter()
            .filter(|s| s.source_id == sources[0].source_id)
            .count(),
        1
    );

    // An event nobody follows is queued for no one.
    arrive(&kernel, "before", "nobody was listening");
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[sources[0].source_id]}}),
    )
    .await
    .unwrap();
    arrive(&kernel, "after", "Pull request #7 updated");
    kernel.owned.route_workflow_notifications();
    let delivered: Vec<_> = wait(&router, &grant, 0)
        .await
        .into_iter()
        .map(|event| match event.value {
            OutsideEventValue::Notification {
                occurrence_id,
                output,
                source_kind,
                ..
            } => (source_kind, occurrence_id, output.message().to_owned()),
            other => panic!("expected notifications, got {other:?}"),
        })
        .collect();
    assert_eq!(
        delivered,
        vec![(
            NotificationSourceKind::AppEvent,
            "after".into(),
            "Pull request #7 updated".into()
        )]
    );

    // A removed route is no longer offered.
    store
        .app_inbox(AppInboxOperation::RemoveRoute {
            owner: "local".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
        })
        .unwrap();
    assert!(listed(&router, &grant)
        .await
        .iter()
        .all(|s| s.kind != NotificationSourceKind::AppEvent));
    drop((router, kernel));
    cleanup(f);
}

// NC-06: the options an App's incoming event declares are on its source, and each occurrence
// carries its values for them, keyed by option name.
#[tokio::test]
async fn an_app_events_declared_options_are_listed_and_each_occurrence_carries_their_values() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let store = kernel.owned.durable_state_store.clone();
    crate::durable_state::app_state::fixture_inbox_installation(&store, "local");
    let grant = kernel.insert_access_grant_for_test(&f.session);
    create_route(&store);
    let source = listed(&router, &grant)
        .await
        .into_iter()
        .find(|s| s.kind == NotificationSourceKind::AppEvent)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&source).unwrap()["options"],
        serde_json::json!([
            {"name":"pr","label":"PR number","type":"number","field":"pull_request"},
            {"name":"repo","label":"Repository","type":"text","field":"repository"}
        ])
    );
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}}),
    )
    .await
    .unwrap();
    arrive_with(
        &kernel,
        "pr-976",
        serde_json::json!({"text": "PR #976 merged", "pull_request": 976}),
    );
    kernel.owned.route_workflow_notifications();
    let outputs: Vec<_> = wait(&router, &grant, 0)
        .await
        .into_iter()
        .map(|event| match event.value {
            OutsideEventValue::Notification { output, .. } => serde_json::to_value(output).unwrap(),
            other => panic!("expected a notification, got {other:?}"),
        })
        .collect();
    assert_eq!(
        outputs,
        vec![serde_json::json!({"message": "PR #976 merged", "data": {"pr": 976}})]
    );
    drop((router, kernel));
    cleanup(f);
}
