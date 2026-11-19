//! Settings changed while a turn runs: the provider's process takes them at the next finished
//! step and the same turn goes on in a new one.

use super::automatic_substitutes::{failing_task_turn_on, failing_turn_on, FailingTurn};
use crate::provider::ProviderRunState;

const NEXT_MODEL: &str = "next-model";

async fn running_turn() -> FailingTurn {
    running(failing_turn_on("dev-stub").await).await
}

async fn running(turn: FailingTurn) -> FailingTurn {
    turn.run_in_worktree().await;
    // The provider has the request and is working on it.
    turn.runtime
        .owned
        .mark_active_prompt_delivery(
            &turn.session_id,
            &turn.agent_id,
            &turn.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(turn.failed_run_id.clone()),
            None,
        )
        .expect("delivery is marked");
    turn
}

/// What the provider's own output starts tracking for a running turn.
fn track_activity(turn: &FailingTurn) {
    turn.runtime
        .owned
        .prompt_activity
        .write()
        .entry(turn.failed_run_id.clone())
        .or_insert(crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
            active_tools: Default::default(),
        });
}

async fn change_model(turn: &FailingTurn, model: &str) -> crate::agent::AgentInstance {
    turn.runtime
        .update_agent_profile(
            &turn.session_id,
            &turn.agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some(model.to_string()),
            None,
        )
        .await
        .expect("a running agent accepts a profile change")
}

fn settings_events(turn: &FailingTurn) -> Vec<serde_json::Value> {
    turn.runtime
        .owned
        .durable_state_store
        .load_events_after(0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "agent.settings_changed")
        .map(|event| event.payload)
        .collect()
}

fn settings_notices(turn: &FailingTurn) -> Vec<String> {
    turn.notices()
        .into_iter()
        .filter(|notice| notice.contains(" changed to ") || notice.contains(" changes to "))
        .collect()
}

#[tokio::test]
async fn the_turn_goes_on_in_a_new_process_when_the_model_changes() {
    let turn = running_turn().await;

    let agent = change_model(&turn, NEXT_MODEL).await;

    assert_eq!(agent.model(), Some(NEXT_MODEL));
    let old_run = turn.live_run().expect("the process keeps running");
    assert_eq!(old_run.id(), turn.failed_run_id);
    assert!(old_run.settings_stale());
    assert!(
        settings_notices(&turn).is_empty(),
        "nothing shows before the swap"
    );

    let swapped = turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .expect("the swap succeeds");

    assert!(swapped);
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&turn.failed_run_id)
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    let new_run = turn.live_run().expect("the turn goes on in a new process");
    assert_ne!(new_run.id(), turn.failed_run_id);
    assert_eq!(new_run.model(), NEXT_MODEL);
    let prompt = turn.active_prompt().expect("the same turn is still active");
    assert_eq!(prompt.id(), turn.prompt_id);
    assert_eq!(
        prompt.durable_delivery_provider_run_id(),
        Some(new_run.id())
    );
    assert_eq!(
        settings_notices(&turn),
        vec![format!("Model changed to {NEXT_MODEL} by you, continuing.")]
    );
    let events = settings_events(&turn);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["changes"][0]["field"], "model");
    assert_eq!(events[0]["changes"][0]["new"], NEXT_MODEL);
    assert_eq!(events[0]["by"]["kind"], "person");
    assert_eq!(events[0]["applies"], "at_step_boundary");
    // The continuation is one durable recovery operation of the same turn, so a restart that
    // interrupts the swap finds it.
    assert!(prompt.durable_recovery_operation_id().is_some());
}

#[tokio::test]
async fn a_tool_call_that_is_still_running_is_never_ended() {
    let turn = running_turn().await;
    change_model(&turn, NEXT_MODEL).await;
    turn.runtime
        .owned
        .prompt_activity
        .write()
        .entry(turn.failed_run_id.clone())
        .or_insert(crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
            active_tools: Default::default(),
        })
        .active_tools
        .insert(
            "tool-1".into(),
            crate::provider::ProviderPromptChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderTool,
                merge_key: Some("tool-1".into()),
                bytes: b"{}".to_vec(),
            },
        );

    let swapped = turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .expect("waiting is not an error");

    assert!(!swapped);
    assert_eq!(turn.live_run().unwrap().id(), turn.failed_run_id);
    assert!(settings_notices(&turn).is_empty());

    turn.runtime
        .owned
        .prompt_activity
        .write()
        .get_mut(&turn.failed_run_id)
        .unwrap()
        .active_tools
        .clear();
    assert!(turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .unwrap());
    assert_eq!(turn.live_run().unwrap().model(), NEXT_MODEL);
}

#[tokio::test]
async fn several_changes_of_one_step_restart_the_process_once() {
    let turn = running_turn().await;
    change_model(&turn, "interim-model").await;
    change_model(&turn, NEXT_MODEL).await;
    turn.runtime
        .update_agent_profile(
            &turn.session_id,
            &turn.agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            None,
            Some(Some("high".to_string())),
        )
        .await
        .expect("effort changes too");

    assert!(turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .unwrap());

    let run = turn.live_run().unwrap();
    assert_eq!((run.model(), run.variant()), (NEXT_MODEL, Some("high")));
    assert_eq!(
        settings_notices(&turn),
        vec![format!(
            "Model changed to {NEXT_MODEL}, effort changed to high by you, continuing."
        )]
    );
    assert_eq!(
        settings_events(&turn).len(),
        3,
        "history keeps every request"
    );
    // The new process is current: another step boundary changes nothing.
    assert!(!turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, run.id())
        .await
        .unwrap());
}

#[tokio::test]
async fn a_change_taken_back_before_the_step_ends_shows_no_line() {
    let turn = running_turn().await;
    change_model(&turn, NEXT_MODEL).await;
    change_model(&turn, "primary-model").await;

    turn.runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .unwrap();

    assert!(settings_notices(&turn)
        .iter()
        .all(|notice| !notice.contains(NEXT_MODEL)));
}

#[tokio::test]
async fn a_stale_process_is_replaced_when_the_next_turn_starts() {
    let turn = running_turn().await;
    change_model(&turn, NEXT_MODEL).await;
    // The turn ends before any step boundary reached the swap.
    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &turn.failed_run_id, true, false, true)
        .await
        .expect("the turn completes");
    assert!(turn.active_prompt().is_none());

    let next = turn
        .runtime
        .with_app_side_effect(|app| {
            app.ensure_prompt_provider_run_for_agent(&turn.session_id, &turn.agent_id)
        })
        .await
        .expect("the next turn launches");

    assert_ne!(next, turn.failed_run_id);
    let run = turn.runtime.owned.provider_store.get_run(&next).unwrap();
    assert_eq!(run.model(), NEXT_MODEL);
}

#[tokio::test]
async fn a_turn_that_is_ending_is_not_swapped() {
    let turn = running_turn().await;
    change_model(&turn, NEXT_MODEL).await;
    turn.runtime
        .owned
        .note_prompt_settlement_requested(&turn.failed_run_id);
    turn.runtime
        .owned
        .mark_prompt_completion_recorded(&turn.failed_run_id);

    let swapped = turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .unwrap();

    assert!(!swapped);
}

fn tool_batch(status: &str) -> crate::provider::ProviderPromptSignalBatch {
    crate::provider::ProviderPromptSignalBatch {
        chunks: vec![crate::provider::ProviderPromptChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderTool,
            merge_key: Some("tool-1".into()),
            bytes: serde_json::to_vec(&serde_json::json!({
                "id": "tool-1", "tool": "Bash", "status": status
            }))
            .unwrap(),
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn the_output_of_a_finished_tool_call_is_the_step_boundary() {
    let turn = running_turn().await;
    track_activity(&turn);
    change_model(&turn, NEXT_MODEL).await;
    let apply = |status: &'static str| {
        turn.runtime.apply_owned_structured_output_batch(
            &turn.session_id,
            &turn.failed_run_id,
            Vec::new(),
            tool_batch(status),
        )
    };

    apply("running").await.expect("the tool call starts");
    assert_eq!(
        turn.live_run().unwrap().id(),
        turn.failed_run_id,
        "a tool call that has not finished keeps its process"
    );

    apply("completed").await.expect("the tool call ends");
    let run = turn.live_run().expect("the turn goes on");
    assert_ne!(run.id(), turn.failed_run_id);
    assert_eq!(run.model(), NEXT_MODEL);
    assert_eq!(turn.active_prompt().unwrap().id(), turn.prompt_id);
}

#[tokio::test]
async fn a_continuation_the_new_process_refuses_fails_the_turn_on_that_process() {
    let turn = running(failing_task_turn_on("dev-stub").await).await;
    let owned = &turn.runtime.owned;
    owned
        .bind_agent_task_run(
            &turn.session_id,
            &turn.agent_id,
            &turn.prompt_id,
            &turn.failed_run_id,
        )
        .expect("the turn is an admitted agent task");
    let task = owned
        .durable_state_store
        .agent_turn_or_continuation_task(&turn.session_id, &turn.agent_id, &turn.prompt_id)
        .unwrap()
        .expect("the turn has its task");
    // The owner blocks the task while the step ends: the new process cannot dispatch it.
    owned
        .durable_state_store
        .agent_lifecycle(crate::durable_state::agent_lifecycle::Operation::Block {
            task: task.task_id,
            prompt: turn.prompt_id.clone(),
            reason: "owner blocked it".into(),
        })
        .unwrap();
    change_model(&turn, NEXT_MODEL).await;

    turn.runtime
        .swap_turn_process_at_step_boundary(&turn.session_id, &turn.failed_run_id)
        .await
        .expect_err("the continuation is refused");

    turn.assert_turn_failed();
}
