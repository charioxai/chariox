use super::*;

#[test]
fn caller_operation_collision_does_not_replay_a_private_workflow_entry() {
    let owner = PromptStateOwner::default();
    let session = RuntimeSession::new(
        "session",
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    let operation = format!(
        "{}{}",
        crate::durable_state::workflow_dispatch_intents::OPERATION_PREFIX,
        "01".repeat(32)
    );
    let caller = PromptQueueItem::new(
        "caller",
        "attachment",
        "agent",
        "unrelated caller",
        PromptStatus::Queued,
    )
    .with_durable_operation(&operation, "caller-fingerprint");
    owner
        .submit_prepared_prompt(&session, caller.clone(), false)
        .unwrap();
    let entry = PromptQueueItem::new(
        "entry",
        "workflow:run",
        "agent",
        "workflow entry",
        PromptStatus::Queued,
    )
    .with_workflow_context("run", "node")
    .with_durable_operation(&operation, "entry-fingerprint");
    assert!(owner
        .replay_durable_submission(&session, &entry)
        .unwrap()
        .is_none());
    let first = owner
        .submit_prepared_prompt(&session, entry.clone(), false)
        .unwrap();
    let replay = owner
        .replay_durable_submission(&session, &entry)
        .unwrap()
        .unwrap();
    let id = |outcome: PromptSubmissionOutcome| match outcome {
        PromptSubmissionOutcome::Started { prompt }
        | PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
    };
    assert_eq!(id(first), id(replay));
    assert_eq!(owner.queued_prompt_count_for_agent(&session, "agent"), 1);
    assert_eq!(
        id(owner
            .replay_durable_submission(&session, &caller)
            .unwrap()
            .unwrap()),
        "caller"
    );
    let other_node = entry.clone().with_workflow_context("run", "other-node");
    assert!(owner
        .replay_durable_submission(&session, &other_node)
        .unwrap()
        .is_none());
    let wrong_fingerprint = entry.with_durable_operation(&operation, "changed");
    assert!(owner
        .replay_durable_submission(&session, &wrong_fingerprint)
        .is_err());
}
