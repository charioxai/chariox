//! NC-09 / NC-10 / NC-11: the subscriptions table lists each subscription of the owner's agents
//! once, with what it picked and its counters; pause holds delivery back and resume lets the
//! next event through; Test is a dry run; an orphan says why, raises a flag in the person's
//! Inbox, and is removed cleanly.
use super::options::{app_event_source, review_requested, subscribe};
use super::*;
use crate::local::{
    NotificationSubscriptionChange, NotificationSubscriptionHealth, NotificationSubscriptionRow,
    NotificationTraceLine, NotificationTraceOutcome, UpdateNotificationSubscriptionRequest,
};

impl Rig {
    fn rows(&self) -> Vec<NotificationSubscriptionRow> {
        match self
            .runtime
            .list_notification_subscriptions("local")
            .unwrap()
        {
            LocalDaemonResponse::NotificationSubscriptions { subscriptions, .. } => subscriptions,
            other => panic!("expected the subscriptions, got {other:?}"),
        }
    }

    async fn change(&self, subscription_id: &str, change: NotificationSubscriptionChange) {
        self.runtime
            .update_notification_subscription(
                UpdateNotificationSubscriptionRequest {
                    subscription_id: subscription_id.into(),
                    change,
                },
                "local",
            )
            .await
            .unwrap();
    }

    fn trace(&self, subscription_id: &str) -> Vec<NotificationTraceLine> {
        match self
            .runtime
            .read_notification_trace(subscription_id, "local")
            .unwrap()
        {
            LocalDaemonResponse::NotificationTrace { lines, .. } => lines,
            other => panic!("expected the trace, got {other:?}"),
        }
    }

    /// The flag cards of the person's Inbox: subscription and health.
    fn flags(&self) -> Vec<(String, NotificationSubscriptionHealth)> {
        match crate::runtime::person_inbox::execute_person_inbox_request(&self.runtime, "local")
            .unwrap()
        {
            LocalDaemonResponse::PersonInbox { cards, .. } => cards
                .into_iter()
                .filter_map(|card| match card.body {
                    crate::local::PersonInboxCardBody::Flag(flag) => {
                        assert_eq!(card.card_id, format!("flag:{}", flag.subscription_id));
                        Some((flag.subscription_id, flag.health))
                    }
                    _ => None,
                })
                .collect(),
            other => panic!("expected the Inbox, got {other:?}"),
        }
    }

    fn test(&self, subscription_id: &str) -> String {
        match self
            .runtime
            .test_notification_subscription(subscription_id, "local")
            .unwrap()
        {
            LocalDaemonResponse::NotificationSubscriptionTested { result } => result,
            other => panic!("expected a dry run, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn the_table_lists_a_subscription_with_its_pick_and_counters_and_pause_holds_delivery() {
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
    let rows = rig.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    let id = row.subscription.subscription_id.clone();
    assert_eq!(row.source.as_ref().unwrap().source_id, source);
    assert!(row.subscriber_name.is_some());
    assert_eq!(
        row.prompt.as_deref(),
        Some("Triage what the pipeline reports")
    );
    assert_eq!(
        serde_json::to_value(&row.options).unwrap(),
        serde_json::json!([{"name": "pr", "value": 976}])
    );
    assert_eq!(
        (row.fired, row.last_fired_at_ms, row.health),
        (0, None, NotificationSubscriptionHealth::Ok)
    );

    review_requested(&rig, "pr-9760", 9760);
    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    let row = rig.rows().remove(0);
    assert_eq!(row.fired, 1);
    assert!(row.last_fired_at_ms.is_some());

    rig.change(&id, NotificationSubscriptionChange::Paused { paused: true })
        .await;
    assert_eq!(rig.rows()[0].health, NotificationSubscriptionHealth::Paused);
    review_requested(&rig, "pr-976-paused", 976);
    rig.pass().await;
    assert_eq!(
        rig.queued_prompts().len(),
        1,
        "a paused subscription wakes no one"
    );

    rig.change(
        &id,
        NotificationSubscriptionChange::Paused { paused: false },
    )
    .await;
    review_requested(&rig, "pr-976-resumed", 976);
    rig.pass().await;
    assert_eq!(
        rig.queued_prompts().len(),
        2,
        "resume delivers the next event"
    );
    let row = rig.rows().remove(0);
    assert_eq!(
        (row.fired, row.health),
        (2, NotificationSubscriptionHealth::Ok)
    );

    let trace: Vec<_> = rig
        .trace(&id)
        .into_iter()
        .map(|line| {
            (
                line.occurrence_id,
                line.outcome,
                line.detail,
                line.turn_ref.is_some(),
            )
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            (
                "pr-976-resumed".into(),
                NotificationTraceOutcome::Delivered,
                None,
                true
            ),
            (
                "pr-976-paused".into(),
                NotificationTraceOutcome::Paused,
                None,
                false
            ),
            (
                "pr-976".into(),
                NotificationTraceOutcome::Delivered,
                None,
                true
            ),
            (
                "pr-9760".into(),
                NotificationTraceOutcome::Filtered,
                Some("Not PR number 976".into()),
                false
            ),
        ]
    );
}

#[tokio::test]
async fn a_test_names_who_would_be_woken_and_sends_nothing() {
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
    assert_eq!(
        rig.test(&id),
        "Nothing has arrived from this source yet, so there is nothing to test. Nothing was sent."
    );

    review_requested(&rig, "pr-9760", 9760);
    rig.pass().await;
    assert_eq!(
        rig.test(&id),
        "\"Review requested on PR #9760\" would be filtered out: Not PR number 976. Nothing was sent."
    );

    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    let (queued, trace) = (rig.queued_prompts().len(), rig.trace(&id).len());
    let name = rig.rows()[0].subscriber_name.clone().unwrap();
    assert_eq!(
        rig.test(&id),
        format!("\"Review requested on PR #976\" would wake {name}. Nothing was sent.")
    );
    let row = rig.rows().remove(0);
    assert_eq!(
        (rig.queued_prompts().len(), rig.trace(&id).len(), row.fired),
        (queued, trace, 1),
        "a dry run queues nothing, traces nothing and moves no counter"
    );
}

#[tokio::test]
async fn an_orphan_says_its_source_is_gone_and_is_removed_with_its_trigger() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.trigger_on(&source).await;
    let id = rig.rows()[0].subscription.subscription_id.clone();
    assert!(
        rig.flags().is_empty(),
        "a healthy subscription raises no flag"
    );
    rig.set_source_enabled(&workflow, false).await;
    let row = rig.rows().remove(0);
    assert_eq!(
        (row.source.is_none(), row.health),
        (true, NotificationSubscriptionHealth::SourceGone)
    );
    assert_eq!(
        rig.flags(),
        vec![(id.clone(), NotificationSubscriptionHealth::SourceGone)]
    );
    let read = crate::runtime::person_inbox::execute_set_inbox_card_state(
        &rig.runtime,
        crate::local::SetInboxCardStateRequest {
            card_ids: vec![format!("flag:{id}")],
            change: crate::local::InboxCardChange::Read { read: true },
        },
        "local",
    )
    .unwrap_err()
    .to_string();
    assert!(
        read.contains("leaves when the subscription is fixed or removed"),
        "{read}"
    );
    let moved = rig
        .runtime
        .update_notification_subscription(
            UpdateNotificationSubscriptionRequest {
                subscription_id: id.clone(),
                change: NotificationSubscriptionChange::Subscriber {
                    agent_id: "anyone".into(),
                },
            },
            "local",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(moved.contains("The source is gone"), "{moved}");

    rig.runtime
        .remove_notification_subscription(&id, "local")
        .await
        .unwrap();
    assert!(rig.rows().is_empty(), "no stale row remains");
    assert!(
        rig.flags().is_empty(),
        "the flag leaves with the subscription"
    );
    assert!(rig.subscriptions().is_empty());
    assert!(rig
        .runtime
        .owned
        .session_snapshot(&rig.session_id)
        .unwrap()
        .agent_triggers()
        .is_empty());
    let unknown = rig
        .runtime
        .remove_notification_subscription(&id, "local")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        unknown.contains("List the subscriptions again"),
        "{unknown}"
    );
}

#[tokio::test]
async fn another_agent_of_the_room_takes_a_subscription_over_with_its_counters() {
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
    review_requested(&rig, "pr-976", 976);
    rig.pass().await;
    let before = rig.rows().remove(0);
    let reviewer = rig
        .runtime
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&rig.session_id, "dev-stub")
                .with_alias("reviewer"),
        )
        .await
        .unwrap();
    rig.change(
        &before.subscription.subscription_id,
        NotificationSubscriptionChange::Subscriber {
            agent_id: reviewer.id().into(),
        },
    )
    .await;
    let rows = rig.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let after = &rows[0];
    assert_eq!(after.subscription.agent_id.as_deref(), Some(reviewer.id()));
    assert_eq!(
        (&after.prompt, &after.options, after.fired, after.health),
        (
            &before.prompt,
            &before.options,
            1,
            NotificationSubscriptionHealth::Ok
        )
    );
    assert_eq!(rig.trace(&after.subscription.subscription_id).len(), 1);
    let triggers = rig
        .runtime
        .owned
        .session_snapshot(&rig.session_id)
        .unwrap()
        .agent_triggers()
        .iter()
        .map(|trigger| trigger.agent_id().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(triggers, vec![reviewer.id().to_owned()]);
}

/// NC-12: a delivery that comes back to the subscription that caused it is dropped, and the
/// row and the person's Inbox say the loop was stopped.
#[tokio::test]
async fn a_delivery_that_comes_back_to_its_subscriber_stops_the_loop_and_raises_a_flag() {
    let rig = rig().await;
    rig.app_notification().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let state = &rig.runtime.owned.prompt_state_owner;
    state
        .complete_active_prompt_only(&room, &rig.agent_id)
        .unwrap();
    state
        .activate_next_queued_prompt(&room, &rig.agent_id, None)
        .unwrap();
    let workflow = rig.workflow_with_exit("reply");
    let source = rig.register(&workflow);
    rig.trigger_on(&source).await;
    let row = |rig: &Rig| {
        rig.rows()
            .into_iter()
            .find(|row| {
                row.source
                    .as_ref()
                    .is_some_and(|summary| summary.source_id == source)
            })
            .unwrap()
    };
    let id = row(&rig).subscription.subscription_id;
    let invocation = rig
        .runtime
        .owned
        .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
        .unwrap();
    rig.complete_with_invocation(&workflow, "cycle", "done", invocation);
    rig.pass().await;

    let stopped = row(&rig);
    assert_eq!(stopped.health, NotificationSubscriptionHealth::LoopStopped);
    assert!(stopped.loop_stopped_at_ms.is_some());
    assert_eq!(
        rig.flags(),
        vec![(id.clone(), NotificationSubscriptionHealth::LoopStopped)]
    );
    assert_eq!(
        rig.trace(&id)[0].outcome,
        NotificationTraceOutcome::LoopStopped
    );
}
