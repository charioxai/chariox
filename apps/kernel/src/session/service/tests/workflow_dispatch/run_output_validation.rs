use super::*;
use crate::session::{RuntimeSession, WorkflowOutputPayload, WorkflowRun};

fn create_schema_bound_completion_workflow(
    service: &mut SessionService,
) -> (RuntimeSession, WorkflowRun, std::path::PathBuf) {
    static SCHEMA_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    let session = service
        .create_session(CreateSessionRequest::new(
            "workspace-output",
            "worktree-output",
        ))
        .expect("session should be created");
    seed_agents(service, session.id(), &["finisher"]);
    let workflow = service
        .create_workflow(session.id(), Some("validated-output".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "finisher")
        .expect("completion node should be added");
    service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("completion capability should update");
    let schema_path = std::env::temp_dir().join(format!(
        "chariox-workflow-output-schema-{}-{}-{}.json",
        std::process::id(),
        unix_epoch_ms(),
        SCHEMA_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(
        &schema_path,
        r#"{"type":"object","required":["invocation_challenge"],"properties":{"invocation_challenge":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema should write");
    service
        .set_workflow_run_output_schema_ref(
            session.id(),
            workflow.id(),
            Some(schema_path.to_string_lossy().to_string()),
        )
        .expect("run output schema should update");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("return challenge ROTATED-CHALLENGE".to_string()),
        )
        .expect("workflow should invoke");
    (session, run, schema_path)
}

#[test]
fn missing_terminal_output_fails_without_automatic_retry() {
    let mut service = SessionService::new(&test_config());
    let (session, run, schema_path) = create_schema_bound_completion_workflow(&mut service);
    let node_run_id = run.node_runs()[0].id().to_string();

    let update = service
        .complete_workflow_node_run_after_provider_turn(
            session.id(),
            run.id(),
            &node_run_id,
            None,
            None,
        )
        .expect("missing output should become a visible failure");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Failed);
    assert!(update.workflow_run.final_output().is_none());
    assert!(update.dispatches.is_empty());
    let failure = update
        .missing_output_failure
        .as_ref()
        .expect("missing output failure should be reported");
    assert_eq!(
        failure.message,
        "provider completed workflow turn without a validated workflow output"
    );
    assert_eq!(
        update.workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Failed
    );
    std::fs::remove_file(schema_path).ok();
}

#[test]
fn invalid_terminal_output_fails_without_automatic_retry() {
    let mut service = SessionService::new(&test_config());
    let (session, run, schema_path) = create_schema_bound_completion_workflow(&mut service);
    let node_run_id = run.node_runs()[0].id().to_string();

    let update = service
        .complete_workflow_node_run(
            session.id(),
            run.id(),
            &node_run_id,
            Some(completion_with_message(r#"{"stale_challenge":"OLD"}"#)),
            None,
        )
        .expect("invalid output should become a visible failure");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Failed);
    assert!(update.workflow_run.final_output().is_none());
    assert!(update.dispatches.is_empty());
    let failure = update
        .run_output_validation_failure
        .as_ref()
        .expect("validation failure should be reported");
    assert!(!failure.message.is_empty());
    std::fs::remove_file(schema_path).ok();
}

#[test]
fn provider_completion_after_valid_tool_submission_is_idempotent() {
    let mut service = SessionService::new(&test_config());
    let (session, run, schema_path) = create_schema_bound_completion_workflow(&mut service);
    let node_run_id = run.node_runs()[0].id().to_string();
    service
        .start_workflow_node_run(session.id(), run.id(), &node_run_id)
        .expect("workflow node run should start");
    service
        .prepare_workflow_turn(
            session.id(),
            run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "review the pull request".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    service
        .submit_workflow_run_final_output(
            session.id(),
            run.id(),
            &node_run_id,
            WorkflowOutputPayload::new(
                r#"{"invocation_challenge":"ROTATED-CHALLENGE"}"#,
                Vec::new(),
            ),
            true,
            None,
        )
        .expect("valid runtime-tool output should be staged");
    let completed = service
        .complete_workflow_node_run(session.id(), run.id(), &node_run_id, None, None)
        .expect("runtime-tool output should complete the workflow");
    assert_eq!(
        completed.workflow_run.status(),
        WorkflowRunStatus::Completed
    );

    let duplicate = service
        .complete_workflow_node_run_after_provider_turn(
            session.id(),
            run.id(),
            &node_run_id,
            None,
            None,
        )
        .expect("late provider completion should be an idempotent no-op");

    assert_eq!(
        duplicate.workflow_run.status(),
        WorkflowRunStatus::Completed
    );
    assert_eq!(
        duplicate.workflow_run.completed_by_node_run_id(),
        Some(node_run_id.as_str())
    );
    assert_eq!(duplicate.workflow_run.final_output_valid(), Some(true));
    assert_eq!(duplicate.workflow_run.node_runs().len(), 1);
    assert!(duplicate.dispatches.is_empty());
    assert!(duplicate.missing_output_failure.is_none());
    assert!(duplicate.workflow_run.failure_events().is_empty());
    std::fs::remove_file(schema_path).ok();
}
