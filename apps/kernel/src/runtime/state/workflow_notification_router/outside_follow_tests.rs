//! A grant says what it follows, a wait on nothing answers at once, and a new grant takes over an
//! earlier grant's targets in one step. Through the real router, with a grant as the caller.
use super::outside_agent_tests::dispatch;
use super::tests::router;
use super::*;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
use crate::runtime::router::CommandRouter;
use std::time::{Duration, Instant};

async fn follow(
    router: &CommandRouter,
    grant: &str,
    request: serde_json::Value,
) -> LocalDaemonResponse {
    dispatch(
        router,
        grant,
        serde_json::json!({ "SubscribeEvents": request }),
    )
    .await
    .unwrap()
}
async fn subscription(router: &CommandRouter, grant: &str) -> EventSubscription {
    match dispatch(
        router,
        grant,
        serde_json::json!({"GetEventSubscription":{}}),
    )
    .await
    .unwrap()
    {
        LocalDaemonResponse::EventSubscription { subscription } => subscription,
        other => panic!("expected a subscription, got {other:?}"),
    }
}
fn refusal(error: DaemonError) -> String {
    match error {
        DaemonError::MachineRequest { code, message } => {
            assert_eq!(code, "events_refused");
            message
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_new_grant_follows_nothing_and_a_wait_or_list_says_so_at_once() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let grant = kernel.insert_access_grant_for_test(&f.session);
    // The subscription of a grant that never subscribed is empty, not an error.
    let empty = subscription(&router, &grant).await;
    assert_eq!(empty.subscription_id, grant);
    assert!(empty.targets.is_empty() && empty.sources.is_empty() && empty.ended.is_none());
    // Access status counts what the grant follows.
    assert_eq!(kernel.outside_follow_count(&grant).unwrap(), 0);
    let started = Instant::now();
    for request in [
        serde_json::json!({"WaitEvents":{"timeout_ms":60000}}),
        serde_json::json!({"ListEvents":{}}),
    ] {
        let message = refusal(dispatch(&router, &grant, request).await.unwrap_err());
        assert!(message.contains("follows nothing"), "{message}");
        assert!(
            message.contains("events subscribe --like <grant id>"),
            "{message}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "answered at once, not after the timeout"
    );
    // A grant is admitted with an empty subscription of its own: the same answers.
    let admitted = kernel.insert_access_grant_for_test(&f.session);
    kernel
        .subscribe_outside_part(&admitted, true, vec![])
        .unwrap();
    for request in [
        serde_json::json!({"WaitEvents":{"timeout_ms":60000}}),
        serde_json::json!({"ListEvents":{}}),
    ] {
        let message = refusal(dispatch(&router, &admitted, request).await.unwrap_err());
        assert!(message.contains("follows nothing"), "{message}");
    }
    // Dropping the last target is the same wait on nothing, but what was already unread is still read.
    let target = EventTarget {
        kernel: kernel.owned.config_projection.snapshot().daemon_id,
        session_id: f.session.clone(),
        agent_id: None,
    };
    follow(&router, &grant, serde_json::json!({"targets":[target]})).await;
    kernel
        .owned
        .durable_state_store
        .outside_mutate(
            crate::durable_state::agent_lifecycle::outside::Mutation::Publish {
                occurrence: "unread-before-unfollow".into(),
                event: Box::new(OutsideEvent {
                    cursor: 0,
                    kernel: target.kernel.clone(),
                    machine: String::new(),
                    session_id: f.session.clone(),
                    agent_id: None,
                    value: OutsideEventValue::Session {
                        state: EventSessionState::AgentAdded,
                    },
                    summary: "added".into(),
                    at_ms: 1,
                    refs: Default::default(),
                }),
            },
        )
        .unwrap();
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"targets":[target]}}),
    )
    .await
    .unwrap();
    let LocalDaemonResponse::EventsListed {
        events,
        next_cursor,
        ..
    } = dispatch(&router, &grant, serde_json::json!({"ListEvents":{}}))
        .await
        .unwrap()
    else {
        panic!("expected events")
    };
    assert_eq!(events.len(), 1, "the unread record is still read");
    dispatch(
        &router,
        &grant,
        serde_json::json!({"AckEvents":{"cursor":next_cursor}}),
    )
    .await
    .unwrap();
    for request in [
        serde_json::json!({"WaitEvents":{"timeout_ms":60000}}),
        serde_json::json!({"ListEvents":{}}),
    ] {
        let message = refusal(dispatch(&router, &grant, request).await.unwrap_err());
        assert!(message.contains("follows nothing"), "{message}");
    }
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_wait_over_the_limit_is_held_no_longer_than_the_limit() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let grant = kernel.insert_access_grant_for_test(&f.session);
    let target = EventTarget {
        kernel: kernel.owned.config_projection.snapshot().daemon_id,
        session_id: f.session.clone(),
        agent_id: None,
    };
    follow(&router, &grant, serde_json::json!({"targets":[target]})).await;
    let request = WaitEventsRequest {
        since: None,
        timeout_ms: 600_000,
        limit: 10,
    };
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let LocalDaemonResponse::EventsListed { timed_out, .. } = kernel
        .outside_wait(&grant, &request, Some(&grant), None)
        .await
        .unwrap()
    else {
        panic!("expected a batch")
    };
    assert!(timed_out);
    let held = started.elapsed();
    assert!(
        held >= Duration::from_secs(540) && held < Duration::from_secs(541),
        "the kernel holds a wait at most 540 s, held {held:?}"
    );
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_new_grant_takes_over_an_earlier_grants_targets_in_one_step() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let old = kernel.insert_access_grant_for_test(&f.session);
    let new = kernel.insert_access_grant_for_test(&f.session);
    let target = EventTarget {
        kernel: kernel.owned.config_projection.snapshot().daemon_id,
        session_id: f.session.clone(),
        agent_id: None,
    };
    follow(&router, &old, serde_json::json!({"targets":[target]})).await;
    assert_eq!(kernel.outside_follow_count(&new).unwrap(), 0);

    let LocalDaemonResponse::EventSubscription { subscription } =
        follow(&router, &new, serde_json::json!({"targets":[],"like":old})).await
    else {
        panic!("expected a subscription")
    };
    assert_eq!(subscription.targets, std::slice::from_ref(&target));
    // A retry follows the same targets once.
    follow(&router, &new, serde_json::json!({"targets":[],"like":old})).await;
    assert_eq!(
        subscription_targets(&router, &new).await,
        std::slice::from_ref(&target)
    );
    assert_eq!(kernel.outside_follow_count(&new).unwrap(), 1);
    // The earlier grant is not changed by being taken over.
    assert_eq!(
        subscription_targets(&router, &old).await,
        std::slice::from_ref(&target)
    );

    // The next event of a followed agent reaches the new grant.
    kernel
        .owned
        .durable_state_store
        .outside_mutate(
            crate::durable_state::agent_lifecycle::outside::Mutation::Publish {
                occurrence: "takeover-event".into(),
                event: Box::new(OutsideEvent {
                    cursor: 0,
                    kernel: target.kernel.clone(),
                    machine: String::new(),
                    session_id: f.session.clone(),
                    agent_id: None,
                    value: OutsideEventValue::Session {
                        state: EventSessionState::AgentAdded,
                    },
                    summary: "idle".into(),
                    at_ms: 1,
                    refs: Default::default(),
                }),
            },
        )
        .unwrap();
    let LocalDaemonResponse::EventsListed { events, .. } = dispatch(
        &router,
        &new,
        serde_json::json!({"WaitEvents":{"timeout_ms":1000}}),
    )
    .await
    .unwrap() else {
        panic!("expected events")
    };
    assert_eq!(events.len(), 1);

    // The earlier grant ends: the new grant's subscription stays.
    kernel
        .revoke_kernel_access(Some("local"), Some(&old), "owner")
        .unwrap();
    assert_eq!(
        subscription_targets(&router, &new).await,
        std::slice::from_ref(&target)
    );
    // An ended earlier grant has nothing to take over; the answer says what to do instead.
    let third = kernel.insert_access_grant_for_test(&f.session);
    let message = refusal(
        dispatch(
            &router,
            &third,
            serde_json::json!({"SubscribeEvents":{"targets":[],"like":old}}),
        )
        .await
        .unwrap_err(),
    );
    assert!(
        message.contains("events subscribe --kernel K --session S"),
        "{message}"
    );
    assert!(subscription_targets(&router, &third).await.is_empty());
    drop((router, kernel));
    cleanup(f);
}

async fn subscription_targets(router: &CommandRouter, grant: &str) -> Vec<EventTarget> {
    subscription(router, grant).await.targets
}

#[tokio::test]
async fn a_takeover_names_one_other_live_grant_of_the_same_owner_and_nothing_else() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let new = kernel.insert_access_grant_for_test(&f.session);
    let empty = kernel.insert_access_grant_for_test(&f.session);
    let stranger = kernel.insert_access_grant_for_test(&f.session);
    kernel
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .get_mut(&stranger)
        .unwrap()
        .summary
        .owner_user_id = "stranger".into();
    let target = EventTarget {
        kernel: kernel.owned.config_projection.snapshot().daemon_id,
        session_id: f.session.clone(),
        agent_id: None,
    };
    // The stranger's grant follows it already (a session it does not own cannot be followed through the router).
    kernel
        .owned
        .durable_state_store
        .outside_mutate(
            crate::durable_state::agent_lifecycle::outside::Mutation::Subscribe {
                id: stranger.clone(),
                home: true,
                targets: vec![target.clone()],
                baseline: vec![],
                open_decisions: vec![],
            },
        )
        .unwrap();
    let attempt = |like: &str, extra: serde_json::Value| {
        let mut body = serde_json::json!({"targets":[],"like":like});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        dispatch(&router, &new, serde_json::json!({"SubscribeEvents":body}))
    };
    // Another owner's grant, an unknown id and this very grant follow nothing for the caller.
    assert!(
        refusal(attempt(&stranger, serde_json::json!({})).await.unwrap_err())
            .contains("no live grant of yours")
    );
    assert!(refusal(
        attempt("access-nope", serde_json::json!({}))
            .await
            .unwrap_err()
    )
    .contains("no live grant of yours"));
    assert!(
        refusal(attempt(&new, serde_json::json!({})).await.unwrap_err())
            .contains("not the one you hold")
    );
    // A live grant of mine that follows nothing has nothing to take over.
    assert!(
        refusal(attempt(&empty, serde_json::json!({})).await.unwrap_err())
            .contains("follows no targets")
    );
    // --like stands alone.
    let mixed = serde_json::json!({"targets":[target]});
    assert!(refusal(attempt(&empty, mixed).await.unwrap_err()).contains("no other target"));
    assert!(subscription_targets(&router, &new).await.is_empty());
    drop((router, kernel));
    cleanup(f);
}
