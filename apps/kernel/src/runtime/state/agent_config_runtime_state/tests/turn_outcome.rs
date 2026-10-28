//! EA2-80: a turn that ends carries one typed outcome in its event, history and state.

use super::automatic_substitutes::{failing_turn, FailingTurn};
use crate::local::{EventTarget, LocalDaemonRequest, SubscribeEventsRequest};
use serde_json::{json, Value};

const OVERLOADED: &str = "Provider prompt dispatch failed: Codex error [server_overloaded]: \
     Selected model is at capacity. Please try a different model.";

fn follow(turn: &FailingTurn) {
    let target = EventTarget {
        kernel: turn.runtime.owned.config_projection.snapshot().daemon_id,
        session_id: turn.session_id.clone(),
        agent_id: Some(turn.agent_id.clone()),
    };
    turn.runtime
        .outside_part_request(
            "follower",
            true,
            Some(crate::session::DEFAULT_LOCAL_USER_ID),
            &LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                targets: vec![target],
            }),
        )
        .unwrap();
}

/// The agent events the follower holds, as they go over the wire.
fn followed_agent_events(turn: &FailingTurn) -> Vec<Value> {
    turn.runtime
        .owned
        .publish_outside_runtime_activity()
        .unwrap();
    turn.runtime
        .outside_store()
        .outside_list("follower", None, 100)
        .unwrap()
        .unwrap()
        .events
        .into_iter()
        .map(|event| serde_json::to_value(event.value).unwrap())
        .filter(|value| value["type"] == "agent" && value["value"]["run_state"].is_string())
        .map(|value| value["value"].clone())
        .collect()
}

fn history_outcome(turn: &FailingTurn) -> Value {
    turn.runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(&turn.session_id, &turn.agent_id, &turn.prompt_id)
        .unwrap()
        .expect("the ended turn has durable settlement history")
        .metadata
        .get("turn_outcome")
        .cloned()
        .unwrap_or(Value::Null)
}

fn state_outcome(turn: &FailingTurn) -> Value {
    let turn = turn
        .runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(&turn.session_id, &turn.agent_id)
        .expect("the ended turn is in the state");
    serde_json::to_value(turn).unwrap()["outcome"].clone()
}

#[tokio::test]
async fn a_completed_turn_says_completed_with_its_ids_in_event_history_and_state() {
    let turn = failing_turn(&[]).await;
    follow(&turn);

    turn.runtime
        .owned
        .complete_local_prompt_without_advance(
            &turn.session_id,
            &turn.agent_id,
            Some(&turn.failed_run_id),
        )
        .unwrap()
        .expect("the active turn completes");

    let expected = json!({ "kind": "completed", "retryable": false, "reason": "" });
    let events = followed_agent_events(&turn);
    let last = events.last().expect("the follower hears the turn end");
    assert_eq!(last["run_state"], "idle", "{last}");
    assert_eq!(last["outcome"], expected, "{last}");
    assert_eq!(last["prompt_id"], turn.prompt_id.as_str());
    assert_eq!(last["turn_id"], turn.prompt_id.as_str());
    assert_eq!(history_outcome(&turn), expected);
    assert_eq!(state_outcome(&turn), expected);
}

#[tokio::test]
async fn a_cancelled_turn_says_cancelled_and_is_not_an_error() {
    let turn = failing_turn(&[]).await;
    follow(&turn);

    turn.runtime
        .owned
        .cancel_active_prompt_only(&turn.session_id, &turn.agent_id)
        .unwrap();

    let expected = json!({ "kind": "cancelled", "retryable": false, "reason": "" });
    let events = followed_agent_events(&turn);
    let last = events.last().expect("the follower hears the turn end");
    assert_eq!(last["run_state"], "idle", "{last}");
    assert_eq!(last["outcome"], expected, "{last}");
    assert_eq!(last["turn_id"], turn.prompt_id.as_str());
    assert!(
        events.iter().all(|event| event["run_state"] != "error"),
        "{events:?}"
    );
    assert_eq!(history_outcome(&turn), expected);
    assert_eq!(state_outcome(&turn), expected);
}

#[tokio::test]
async fn a_failed_turn_carries_the_same_outcome_in_event_history_and_state() {
    let turn = failing_turn(&[]).await;
    follow(&turn);

    turn.fail_run(&turn.failed_run_id, OVERLOADED).await;

    let expected = json!({
        "kind": "overloaded",
        "retryable": true,
        "reason": "Selected model is at capacity. Please try a different model."
    });
    let events = followed_agent_events(&turn);
    let last = events.last().expect("the follower hears the turn end");
    assert_eq!(last["run_state"], "error", "{last}");
    assert_eq!(last["outcome"], expected, "{last}");
    assert_eq!(last["prompt_id"], turn.prompt_id.as_str());
    assert_eq!(history_outcome(&turn), expected);
    assert_eq!(state_outcome(&turn), expected);
}

#[tokio::test]
async fn a_credential_in_the_provider_error_never_reaches_the_outcome_reason() {
    let turn = failing_turn(&[]).await;
    follow(&turn);

    turn.fail_run(
        &turn.failed_run_id,
        "Provider prompt dispatch failed: Claude StopFailure [server_error]: API Error: 500 \
         for Authorization: Bearer sk-ant-abcdefghijklmnop0123456789",
    )
    .await;

    let events = followed_agent_events(&turn);
    let outcome = &events.last().expect("the turn ends")["outcome"];
    let reason = outcome["reason"].as_str().expect("a reason for people");
    assert!(reason.starts_with("API Error: 500"), "{reason}");
    assert!(!reason.contains("abcdefghijklmnop"), "{reason}");
    for stored in [history_outcome(&turn), state_outcome(&turn)] {
        assert!(!stored.to_string().contains("abcdefghijklmnop"), "{stored}");
    }
}

#[test]
fn a_kind_from_a_newer_kernel_reads_as_unknown_and_keeps_its_reason() {
    let value: crate::local::OutsideEventValue = serde_json::from_value(json!({
        "type": "agent",
        "value": {
            "run_state": "error",
            "prompt_id": "prompt-1",
            "turn_id": "prompt-1",
            "outcome": { "kind": "quota_wizard", "retryable": false, "reason": "A new cause." }
        }
    }))
    .unwrap();

    let wire = serde_json::to_value(value).unwrap();
    assert_eq!(
        wire["value"]["outcome"],
        json!({ "kind": "unknown", "retryable": false, "reason": "A new cause." })
    );
}
