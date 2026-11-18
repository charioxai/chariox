//! NC-06: an agent subscribes to an App event with values for the options the event declares.
//! An occurrence whose value differs wakes no one, a value the source cannot carry is refused
//! when the subscription is made, and `once` ends the subscription after its first delivery.
use super::*;
use crate::durable_state::app_inbox::AppInboxOperation;
use crate::local::{NotificationOptionValue, NotificationSourceKind};
use chariox_app_runtime::app_inbox::InboxRoute;

/// The dummy App's "received" event routed for the owner; it declares `pr` (a number) and
/// `repo` (text). Returns its source id.
pub(super) fn app_event_source(rig: &Rig) -> String {
    let store = &rig.runtime.owned.durable_state_store;
    crate::durable_state::app_state::fixture_inbox_installation(store, "local");
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
    let (stored, _, _) = store.notification_inventory("local").unwrap();
    rig.runtime
        .owned
        .sync_app_event_notification_sources("local", &stored)
        .unwrap();
    store
        .notification_inventory("local")
        .unwrap()
        .0
        .into_iter()
        .find(|source| source.kind == NotificationSourceKind::AppEvent)
        .unwrap()
        .source_id
}

pub(super) fn review_requested(rig: &Rig, occurrence: &str, pr: u64) {
    rig.runtime
        .owned
        .durable_state_store
        .app_inbox(AppInboxOperation::Accept {
            owner: "local".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
            occurrence_id: occurrence.into(),
            payload: serde_json::json!({"text": format!("Review requested on PR #{pr}"), "pull_request": pr}),
            generation: 1,
            now_ms: crate::session::unix_epoch_ms(),
        })
        .unwrap();
}

pub(super) async fn subscribe(
    rig: &Rig,
    source: &str,
    options: serde_json::Value,
    once: bool,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut request = rig.request(source);
    request.source = AgentTriggerSourceRequest::Notification {
        source: AgentTriggerNotificationSource::Workflow {
            source_id: source.into(),
        },
        options: serde_json::from_value::<Vec<NotificationOptionValue>>(options).unwrap(),
        once,
    };
    rig.runtime.create_agent_trigger(request, "local").await
}

fn triggers(rig: &Rig) -> usize {
    rig.runtime
        .owned
        .session_snapshot(&rig.session_id)
        .unwrap()
        .agent_triggers()
        .len()
}

#[tokio::test]
async fn an_agent_waiting_for_pr_976_is_woken_by_976_and_not_by_9760() {
    let rig = rig().await;
    let source = app_event_source(&rig);
    subscribe(
        &rig,
        &source,
        serde_json::json!([{"name": "pr", "value": 976}]),
        false,
    )
    .await
    .unwrap();
    review_requested(&rig, "pr-9760", 9760);
    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    let queued = rig.queued_prompts();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(
        queued[0].contains("Review requested on PR #976\""),
        "{}",
        queued[0]
    );
    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    assert_eq!(session.agent_triggers()[0].fired(), 1);
    // The skipped occurrence is settled, so it never wakes the agent later.
    rig.pass().await;
    assert_eq!(rig.queued_prompts().len(), 1);
}

#[tokio::test]
async fn options_the_source_cannot_carry_are_refused_and_nothing_is_subscribed() {
    let rig = rig().await;
    let source = app_event_source(&rig);
    for (options, refusal) in [
        (
            serde_json::json!([{"name": "pr", "value": "976"}]),
            "option pr takes a number value, not \"976\"",
        ),
        (
            serde_json::json!([{"name": "branch", "value": "main"}]),
            "option branch is not declared",
        ),
        (
            serde_json::json!([{"name": "pr", "value": 976}, {"name": "pr", "value": 977}]),
            "option pr is picked twice",
        ),
    ] {
        let error = subscribe(&rig, &source, options, false)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(refusal), "{error}");
        assert!(
            error.contains("this source declares pr (PR number, number), repo (Repository, text)"),
            "{error}"
        );
    }
    let mut app = rig.request(&source);
    app.source = AgentTriggerSourceRequest::Notification {
        source: AgentTriggerNotificationSource::App {
            installation_id: "installed".into(),
            event_name: "received".into(),
        },
        options: Vec::new(),
        once: true,
    };
    assert!(rig
        .runtime
        .create_agent_trigger(app, "local")
        .await
        .is_err());
    assert_eq!(triggers(&rig), 0);
    assert!(rig.subscriptions().is_empty());
}

#[tokio::test]
async fn a_once_subscription_ends_after_its_first_delivery() {
    let rig = rig().await;
    let source = app_event_source(&rig);
    subscribe(
        &rig,
        &source,
        serde_json::json!([{"name": "pr", "value": 976}]),
        true,
    )
    .await
    .unwrap();
    // An occurrence the options skip does not use up the one delivery.
    review_requested(&rig, "pr-977", 977);
    rig.pass().await;
    assert!(rig.queued_prompts().is_empty());
    assert_eq!(triggers(&rig), 1);

    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    assert_eq!(rig.queued_prompts().len(), 1);
    assert_eq!(triggers(&rig), 0);
    assert!(rig.subscriptions().is_empty());

    review_requested(&rig, "pr-976-again", 976);
    rig.pass().await;
    assert_eq!(rig.queued_prompts().len(), 1);
}
