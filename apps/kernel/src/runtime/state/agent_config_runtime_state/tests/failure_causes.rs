//! EA2-75: a failed turn says why, typed, on its outside event and in history.

use super::automatic_substitutes::{failing_turn, FailingTurn};
use crate::local::{
    EventRunState, EventTarget, LocalDaemonRequest, OutsideEventValue, SubscribeEventsRequest,
};
use crate::provider::{ProviderFailure, ProviderFailureCause};

const OVERLOADED: &str = "Provider prompt dispatch failed: Codex error [server_overloaded]: \
     Selected model is at capacity. Please try a different model.";

async fn subscribed_failing_turn() -> FailingTurn {
    let turn = failing_turn(&[]).await;
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
    turn
}

fn followed_error_failure(turn: &FailingTurn) -> Option<ProviderFailure> {
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
        .find_map(|event| match event.value {
            OutsideEventValue::Agent {
                run_state: Some(EventRunState::Error),
                failure,
                ..
            } => Some(failure),
            _ => None,
        })
        .expect("the follower receives the error run state")
}

fn settled_history_failure(turn: &FailingTurn) -> Option<ProviderFailure> {
    let settlement = turn
        .runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(&turn.session_id, &turn.agent_id, &turn.prompt_id)
        .unwrap()
        .expect("the failed turn has durable settlement history");
    settlement
        .metadata
        .get(crate::history::PROMPT_FAILURE_METADATA_KEY)
        .map(|failure| serde_json::from_value(failure.clone()).unwrap())
}

fn provider_error_outputs(turn: &FailingTurn) -> Vec<String> {
    turn.runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
        .collect()
}

#[tokio::test]
async fn a_codex_overload_reaches_the_follower_and_history_as_a_retryable_cause() {
    let turn = subscribed_failing_turn().await;

    turn.fail_run(&turn.failed_run_id, OVERLOADED).await;

    turn.assert_turn_failed();
    let expected = ProviderFailure {
        cause: ProviderFailureCause::Overloaded,
        retryable: true,
        message: "Selected model is at capacity. Please try a different model.".to_string(),
    };
    assert_eq!(followed_error_failure(&turn), Some(expected.clone()));
    assert_eq!(settled_history_failure(&turn), Some(expected));
}

#[tokio::test]
async fn a_claude_session_limit_reaches_the_follower_as_a_usage_limit_that_will_not_clear_by_retrying(
) {
    let turn = subscribed_failing_turn().await;

    turn.fail_run(
        &turn.failed_run_id,
        "Claude StopFailure [rate_limit]: You've hit your session limit · resets 8:30am",
    )
    .await;

    let failure = followed_error_failure(&turn).expect("typed cause");
    assert_eq!(failure.cause, ProviderFailureCause::UsageLimit);
    assert!(!failure.retryable);
    assert_eq!(
        failure.message,
        "You've hit your session limit · resets 8:30am"
    );
    assert_eq!(settled_history_failure(&turn), Some(failure));
}

#[tokio::test]
async fn a_declined_login_recovery_preserves_the_native_error_in_events_and_history() {
    let turn = subscribed_failing_turn().await;
    let native = ProviderFailure::classify(
        "codex",
        "Codex error [unauthorized]: unexpected status 401 Unauthorized",
    );
    turn.runtime
        .fail_owned_provider_prompt_with_termination_if_matches(
            &turn.session_id,
            &turn.failed_run_id,
            "Provider login did not complete; retry after logging in on this machine.",
            true,
            Some(&turn.prompt_id),
            None,
            Some(native.clone()),
        )
        .await
        .unwrap();
    turn.assert_turn_failed();
    assert_eq!(followed_error_failure(&turn), Some(native.clone()));
    assert_eq!(settled_history_failure(&turn), Some(native.clone()));
    assert_eq!(
        provider_error_outputs(&turn),
        vec![
            native.notice(),
            crate::agent::failed_request_notice(&crate::agent::failed_request_reason(
                "dev-stub",
                "Provider login did not complete; retry after logging in on this machine.",
            )),
        ],
        "the transcript says what the history says"
    );
}

#[tokio::test]
async fn a_long_multiline_structured_error_stays_whole_in_events_history_and_transcript() {
    let turn = subscribed_failing_turn().await;
    let native = format!(
        "First line\n{}\nlast line",
        "native diagnostic\n".repeat(60)
    );
    assert!(native.len() > 1000);

    turn.runtime
        .apply_owned_structured_output_batch(
            &turn.session_id,
            &turn.failed_run_id,
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                terminal_failure: Some(crate::provider::provider_coded_failure_text(
                    "Codex", "other", &native,
                )),
                explicit_provider_error: true,
                prompt_completed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    turn.assert_turn_failed();
    let failure = followed_error_failure(&turn).expect("typed cause");
    assert_eq!(failure.message, native);
    assert_eq!(settled_history_failure(&turn), Some(failure.clone()));
    assert!(
        provider_error_outputs(&turn).contains(&failure.notice()),
        "the transcript entry carries the whole native error"
    );
}

#[tokio::test]
async fn a_cancelled_turn_does_not_retain_a_late_failure() {
    let turn = subscribed_failing_turn().await;
    let session = turn
        .runtime
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let prompt = turn
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &turn.agent_id)
        .unwrap();
    turn.runtime
        .owned
        .completed_git_turn_snapshots
        .record_prompt_settlement_with_termination(
            &turn.session_id,
            &turn.agent_id,
            &turn.failed_run_id,
            &prompt,
            crate::session::unix_epoch_ms(),
            None,
            crate::git_observer::CompletedTurnSettlementStatus::Cancelled,
            None,
            Some(ProviderFailure::classify("codex", OVERLOADED)),
        );
    let settled = turn
        .runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(&turn.session_id, &turn.agent_id)
        .unwrap();
    assert_eq!(settled.failure, None);
}
