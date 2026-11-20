//! Every operation of the subscriptions table either applies whole or refuses before it writes,
//! for each kind of subscription: a workflow's completion, an App event, and a workflow's
//! responsible agent. A pause holds an occurrence where its prompt would be queued, and every
//! trace line moves the person's Inbox revision.
use super::super::options::{app_event_source, review_requested, subscribe};
use super::*;
use crate::local::{SetWorkflowResponsibleRequest, WorkflowResponsibleTarget};

/// Triggers of the Room: id, agent and prompt.
fn triggers(rig: &Rig) -> Vec<(String, String, String)> {
    rig.runtime
        .owned
        .session_snapshot(&rig.session_id)
        .unwrap()
        .agent_triggers()
        .iter()
        .map(|t| (t.id().into(), t.agent_id().into(), t.prompt().into()))
        .collect()
}

async fn reviewer(rig: &Rig) -> String {
    rig.runtime
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&rig.session_id, "dev-stub")
                .with_alias("reviewer"),
        )
        .await
        .unwrap()
        .id()
        .to_owned()
}

async fn move_to(rig: &Rig, id: &str, agent_id: &str) -> Result<LocalDaemonResponse, DaemonError> {
    rig.runtime
        .update_notification_subscription(
            UpdateNotificationSubscriptionRequest {
                subscription_id: id.into(),
                change: NotificationSubscriptionChange::Subscriber {
                    agent_id: agent_id.into(),
                },
            },
            "local",
        )
        .await
}

/// An accepted occurrence is queued again after the one-second rotation of a held one.
async fn after_rotation() {
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
}

#[tokio::test]
async fn a_responsibility_subscription_refuses_move_removal_and_resubscribe_before_any_write() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let (result, _) = rig
        .runtime
        .execute_workflow_set_responsible_request(
            SetWorkflowResponsibleRequest {
                session_id: rig.session_id.clone(),
                workflow_ref: workflow.clone(),
                responsible: WorkflowResponsibleTarget::Agent {
                    kernel_id: rig.runtime.owned.config_projection.snapshot().daemon_id,
                    room_id: rig.session_id.clone(),
                    agent_id: rig.agent_id.clone(),
                },
            },
            "local",
            None,
        )
        .await;
    result.unwrap();
    let rows = rig.rows();
    let row = rows
        .iter()
        .find(|row| row.responsible_for.is_some())
        .unwrap_or_else(|| panic!("a responsibility row: {rows:?}"))
        .clone();
    let id = row.subscription.subscription_id.clone();
    let other = reviewer(&rig).await;
    let (before_triggers, before_rows) = (triggers(&rig), format!("{:?}", rig.rows()));

    let moved = move_to(&rig, &id, &other).await.unwrap_err().to_string();
    assert!(moved.contains("change who is responsible"), "{moved}");
    let removed = rig
        .runtime
        .remove_notification_subscription(&id, "local")
        .await
        .unwrap_err()
        .to_string();
    assert!(removed.contains("change who is responsible"), "{removed}");
    let mut again = rig.request(&row.subscription.source_id);
    again.prompt = Some("Someone else's words".into());
    let resubscribed = rig
        .runtime
        .create_agent_trigger(again, "local")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        resubscribed.contains("change who is responsible"),
        "{resubscribed}"
    );

    assert_eq!(triggers(&rig), before_triggers, "no trigger changed");
    assert_eq!(
        format!("{:?}", rig.rows()),
        before_rows,
        "no row, setting or counter changed"
    );
}

#[tokio::test]
async fn a_move_onto_an_agent_that_already_listens_to_the_source_changes_nothing() {
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
    let other = reviewer(&rig).await;
    let mut own = rig.request(&source);
    own.agent_id = other.clone();
    own.prompt = Some("The reviewer's own words".into());
    rig.runtime
        .create_agent_trigger(own, "local")
        .await
        .unwrap();
    let rows = rig.rows();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let mine = rows
        .iter()
        .find(|row| row.subscription.agent_id.as_deref() == Some(rig.agent_id.as_str()))
        .unwrap()
        .subscription
        .subscription_id
        .clone();
    let (before_triggers, before_rows) = (triggers(&rig), format!("{:?}", rig.rows()));

    let refused = move_to(&rig, &mine, &other).await.unwrap_err().to_string();
    assert!(refused.contains("already subscribes"), "{refused}");
    assert_eq!(
        triggers(&rig),
        before_triggers,
        "the reviewer keeps its prompt"
    );
    assert_eq!(
        format!("{:?}", rig.rows()),
        before_rows,
        "the reviewer keeps its options"
    );
}

#[tokio::test]
async fn an_app_event_listener_is_a_row_that_pauses_tests_moves_and_is_removed() {
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
    // The notification_subscribe tool's App source, as an agent sends it.
    let source: AgentTriggerNotificationSource = serde_json::from_value(serde_json::json!({
        "app": {"installation_id": catalog.installation_id(), "event_name": "changed"}
    }))
    .unwrap();
    let mut request = rig.request("unused");
    request.source = AgentTriggerSourceRequest::Notification {
        source,
        options: Vec::new(),
        once: false,
    };
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = rig
        .runtime
        .create_agent_trigger(request, "local")
        .await
        .unwrap()
    else {
        panic!("App trigger")
    };
    let id = notification_subscription_id(&trigger).unwrap().to_owned();
    let row = |rig: &Rig| {
        rig.rows()
            .into_iter()
            .find(|row| row.subscription.subscription_id == id)
    };
    let listed = row(&rig).expect("the App listener is a row");
    assert_eq!(
        listed.source.as_ref().map(|source| source.kind),
        Some(crate::local::NotificationSourceKind::AppEvent)
    );
    assert_eq!(
        (listed.health, listed.prompt.as_deref()),
        (
            NotificationSubscriptionHealth::Ok,
            Some("Triage what the pipeline reports")
        )
    );
    assert_eq!(
        rig.test(&id),
        "Nothing has arrived from this source yet, so there is nothing to test. Nothing was sent."
    );

    rig.change(&id, NotificationSubscriptionChange::Paused { paused: true })
        .await;
    let queued = rig.queued_prompts().len();
    let inbox = rig.runtime.person_inbox_change_sequence();
    rig.emit_app(catalog.clone(), &trigger, "held");
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    assert_eq!(
        rig.queued_prompts().len(),
        queued,
        "a paused App listener wakes no one"
    );
    assert_eq!(rig.trace(&id)[0].outcome, NotificationTraceOutcome::Paused);
    assert!(rig.runtime.person_inbox_change_sequence() > inbox);
    assert_eq!(
        rig.test(&id),
        "\"App hint\" would wait: the subscription is paused. Nothing was sent."
    );

    rig.change(
        &id,
        NotificationSubscriptionChange::Paused { paused: false },
    )
    .await;
    after_rotation().await;
    let inbox = rig.runtime.person_inbox_change_sequence();
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    assert_eq!(
        rig.queued_prompts().len(),
        queued + 1,
        "resume delivers the held event"
    );
    assert!(rig.runtime.person_inbox_change_sequence() > inbox);
    let delivered = row(&rig).unwrap();
    assert_eq!(
        (delivered.fired, delivered.health),
        (1, NotificationSubscriptionHealth::Ok)
    );
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::Delivered
    );
    let name = delivered.subscriber_name.clone().unwrap();
    // A delivered App occurrence keeps no words, so the dry run names its event.
    assert_eq!(
        rig.test(&id),
        format!("\"changed\" would wake {name}. Nothing was sent.")
    );

    let other = reviewer(&rig).await;
    move_to(&rig, &id, &other).await.unwrap();
    assert!(row(&rig).is_none(), "the old App listener is gone");
    let moved = rig
        .rows()
        .into_iter()
        .find(|row| row.subscription.agent_id.as_deref() == Some(other.as_str()))
        .expect("the reviewer's App listener");
    let moved_id = moved.subscription.subscription_id.clone();
    assert_eq!((moved.fired, rig.trace(&moved_id).len()), (1, 1));
    assert_eq!(
        triggers(&rig)
            .into_iter()
            .map(|(_, agent, _)| agent)
            .collect::<Vec<_>>(),
        vec![other.clone()]
    );

    rig.runtime
        .remove_notification_subscription(&moved_id, "local")
        .await
        .unwrap();
    assert!(rig.rows().is_empty(), "no stale row remains");
    assert!(triggers(&rig).is_empty());
}

#[tokio::test]
async fn a_pause_after_acceptance_holds_the_occurrence_until_resume() {
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
    let id = rig.rows()[0].subscription.subscription_id.clone();
    review_requested(&rig, "pr-976", 976);
    // Accepted for the agent, not yet queued as its prompt.
    rig.runtime.owned.route_workflow_notifications();
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::Delivered
    );
    rig.change(&id, NotificationSubscriptionChange::Paused { paused: true })
        .await;

    rig.runtime.route_agent_notifications().await;
    assert!(
        rig.queued_prompts().is_empty(),
        "a pause before queueing holds the accepted occurrence"
    );
    assert_eq!(rig.trace(&id)[0].outcome, NotificationTraceOutcome::Paused);

    rig.change(
        &id,
        NotificationSubscriptionChange::Paused { paused: false },
    )
    .await;
    after_rotation().await;
    rig.runtime.route_agent_notifications().await;
    assert_eq!(
        rig.queued_prompts().len(),
        1,
        "resume queues the held occurrence"
    );
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::Delivered
    );
}

#[tokio::test]
async fn a_filtered_occurrence_moves_the_inbox_revision() {
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
    let id = rig.rows()[0].subscription.subscription_id.clone();
    let before = rig.runtime.person_inbox_change_sequence();
    review_requested(&rig, "pr-9760", 9760);
    rig.pass().await;
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::Filtered
    );
    assert!(
        rig.runtime.person_inbox_change_sequence() > before,
        "an open trace reads the filtered line without polling"
    );
}

#[tokio::test]
async fn a_delivered_occurrence_moves_the_inbox_revision_with_its_fired_count() {
    let rig = rig().await;
    let source = app_event_source(&rig);
    subscribe(&rig, &source, serde_json::json!([]), false)
        .await
        .unwrap();
    let id = rig.rows()[0].subscription.subscription_id.clone();
    let before = rig.runtime.person_inbox_change_sequence();
    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    assert_eq!(rig.rows()[0].fired, 1);
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::Delivered
    );
    assert!(
        rig.runtime.person_inbox_change_sequence() > before,
        "another client reads the new fired count without polling"
    );
}
