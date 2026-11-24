//! A prompt steered into a running turn has no turn of its own: its task settles with the turn it
//! was steered into, and its answer is that turn's reply after the steer. Left Working, the
//! supervision sweep found it without an executor and blocked it as interrupted (live: an urgent
//! message absorbed into a running task raised "Interrupted before it finished" right after that
//! task finished).
use super::child_finished::{child_finished, team_with};
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::provider::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
};
use crate::terminal::TerminalOutputKind;

#[tokio::test]
async fn a_prompt_steered_into_a_turn_finishes_with_that_turn_and_its_reply() {
    let team = team_with("steered-prompt-settles", None, false).await;
    let runtime = &team.runtime;
    let queued = crate::session::PromptQueueItem::new(
        runtime.owned.session_store.reserve_prompt_id(),
        team.child_prompt.source_attachment_id(),
        &team.child,
        "Report your status",
        crate::session::PromptStatus::Queued,
    );
    runtime
        .submit_prepared_prompt_with_queue_policy(
            crate::app::KernelPreparedPromptSubmission {
                session_id: team.room.clone(),
                prompt: queued,
                force_queue: true,
                refresh_projection: false,
            },
            true,
        )
        .await
        .unwrap();
    let session = runtime.owned.session_store.get_session(&team.room).unwrap();
    let (_, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &team.child);
    let steered = queued[0].id().to_string();
    runtime
        .owned
        .steer_queued_prompt(
            &team.room,
            &team.child,
            team.child_prompt.source_attachment_id(),
            &steered,
        )
        .unwrap();
    let steered_task = || {
        runtime
            .owned
            .durable_state_store
            .agent_turn_task(&team.room, &team.child, &steered)
            .unwrap()
            .expect("the steered prompt has a task")
    };
    assert_eq!(steered_task().state, ExecutionState::Working);

    runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks: vec![ProviderPromptChunk {
                    kind: TerminalOutputKind::ProviderOutput,
                    merge_key: None,
                    bytes: b"status: the work is done".to_vec(),
                }],
                completions: vec![ProviderAssistantCompletion {
                    message_id: "claude:steered:1".into(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    runtime
        .settle_owned_provider_prompt(&team.room, &team.child_run, false, false, false)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .durable_state_store
            .agent_task(&team.task.task_id)
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Done,
        "the turn it was steered into finished"
    );
    assert_eq!(
        steered_task().state,
        ExecutionState::Done,
        "the steered prompt finishes with that turn: {}",
        steered_task().reason
    );
    let finished = child_finished(&team);
    let excerpt = finished
        .iter()
        .find(|event| event.payload["child_task_id"] == steered_task().task_id)
        .and_then(|event| event.payload["public_answer"]["excerpt"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        excerpt.contains("status: the work is done"),
        "its answer is the reply after the steer: {excerpt:?} in {finished:?}"
    );
    runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(steered_task().state, ExecutionState::Done);
}
