//! MP-08 / MP-09 / MP-10 / MP-11 (GE-07): an App's incoming event is a source an outside agent
//! follows without a workflow in between, on the real durable writer. Supplementary regressions,
//! not live acceptance.
use super::outside_agent_tests::{accept, append, events, follow};
use super::tests::{cleanup, Fixture};
use super::*;
use crate::durable_state::app_inbox::{AppInboxOperation, AppInboxOutcome};
use crate::local::{NotificationSourceKind, OutsideEventValue};
use chariox_app_runtime::app_inbox::{Accepted, InboxRoute};
use serde_json::json;

const INSTALLATION: &str = "installed";
const ROUTE: &str = "mail";

fn installed_route(f: &Fixture) {
    crate::durable_state::app_state::fixture_inbox_installation(&f.store, "local");
    f.store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: InboxRoute {
                route_id: ROUTE.into(),
                owner_id: "local".into(),
                installation_id: INSTALLATION.into(),
                event_name: "received".into(),
                source_event_type: "dev.chariox.dummy/dummy.test".into(),
                source_event_version: 1,
                active: true,
                source: None,
            },
            now_ms: 1,
        })
        .unwrap();
}
fn register(f: &Fixture, enabled: bool) -> WorkflowNotificationSource {
    let source = WorkflowNotificationSource {
        kind: NotificationSourceKind::AppEvent,
        source_id: "app-event-source".into(),
        owner_user_id: "local".into(),
        kernel_id: "daemon-test".into(),
        session_id: INSTALLATION.into(),
        workflow_id: ROUTE.into(),
        exit_node_id: "received".into(),
        enabled,
        available: true,
        name: "received".into(),
        exit_ordinal: 0,
        display_name: "Received".into(),
    };
    match f
        .store
        .notify(NotificationOperation::Register(SourceAdmission {
            source,
            workflow_json: String::new(),
        }))
        .unwrap()
    {
        NotificationOutcome::Source(source) => source,
        other => panic!("{other:?}"),
    }
}
fn arrive(f: &Fixture, occurrence: &str, text: &str) -> Accepted {
    match f
        .store
        .app_inbox(AppInboxOperation::Accept {
            owner: "local".into(),
            installation: INSTALLATION.into(),
            route_id: ROUTE.into(),
            occurrence_id: occurrence.into(),
            payload: json!({"text": text, "occurred_at": "2026-11-07T22:00:00Z"}),
            generation: 1,
            now_ms: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    {
        AppInboxOutcome::Accepted(accepted) => accepted,
        other => panic!("{other:?}"),
    }
}
fn delivered(f: &Fixture, sub: &WorkflowNotificationSubscription) -> Vec<(String, String)> {
    for (candidate, env) in f.candidates(false) {
        assert_eq!(candidate.subscription_id, sub.subscription_id);
        accept(f, sub, &env).unwrap();
        append(f, sub, &env);
    }
    events(f)
        .into_iter()
        .map(|event| {
            let OutsideEventValue::Notification {
                source_kind,
                output,
                occurrence_id,
                ..
            } = event.value
            else {
                panic!("expected a notification, got {event:?}");
            };
            assert_eq!(source_kind, NotificationSourceKind::AppEvent);
            (occurrence_id, output.message().to_owned())
        })
        .collect()
}

#[test]
fn a_followed_apps_event_reaches_the_outside_agent_as_the_event_text_once() {
    let f = Fixture::new();
    installed_route(&f);
    let source = register(&f, true);
    let sub = follow(&f, &source);
    assert!(matches!(
        arrive(&f, "occ-1", "Pull request #7 updated"),
        Accepted::New(_)
    ));
    assert!(matches!(
        arrive(&f, "occ-1", "Pull request #7 updated"),
        Accepted::Duplicate(_)
    ));
    assert_eq!(
        delivered(&f, &sub),
        vec![("occ-1".to_owned(), "Pull request #7 updated".to_owned())]
    );
    cleanup(f);
}

#[test]
fn an_event_nobody_follows_queues_nothing_and_a_later_follower_gets_only_later_events() {
    let f = Fixture::new();
    installed_route(&f);
    let source = register(&f, true);
    arrive(&f, "before", "nobody was listening");
    assert!(f.candidates(false).is_empty() && f.candidates(true).is_empty());
    let sub = follow(&f, &source);
    arrive(&f, "after", "now someone is");
    assert_eq!(
        delivered(&f, &sub),
        vec![("after".to_owned(), "now someone is".to_owned())]
    );
    cleanup(f);
}

#[test]
fn a_disabled_source_delivers_nothing() {
    let f = Fixture::new();
    installed_route(&f);
    let source = register(&f, true);
    let sub = follow(&f, &source);
    register(&f, false);
    arrive(&f, "occ-1", "ignored");
    assert!(delivered(&f, &sub).is_empty());
    cleanup(f);
}
