use super::*;
use crate::session::{RuntimeSession, WorkflowRun};

fn create_schema_bound_completion_workflow(
    service: &mut SessionService,
) -> (RuntimeSession, WorkflowRun, String, std::path::PathBuf) {
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
        "arroba-workflow-output-schema-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
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
    (session, run, node.id().to_string(), schema_path)
}

#[test]
fn invalid_terminal_output_schedules_a_correction_turn() {
    let mut service = SessionService::new(&test_config());
    let (session, run, node_id, schema_path) =
        create_schema_bound_completion_workflow(&mut service);
    let first_node_run_id = run.node_runs()[0].id().to_string();

    let update = service
        .complete_workflow_node_run(
            session.id(),
            run.id(),
            &first_node_run_id,
            Some(completion_with_message(r#"{"stale_challenge":"OLD"}"#)),
            None,
        )
        .expect("invalid output should be handled as a correction turn");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Waiting);
    assert!(update.workflow_run.final_output().is_none());
    assert_eq!(update.dispatches.len(), 1);
    assert_eq!(update.dispatches[0].node_run.node_id(), node_id);
    assert!(update.dispatches[0]
        .endpoint_prompt
        .as_deref()
        .is_some_and(|prompt| {
            prompt.contains("return challenge ROTATED-CHALLENGE")
                && prompt.contains("failed schema validation")
                && prompt.contains("validate_and_submit_workflow_run_output")
        }));
    let failure = update
        .run_output_validation_failure
        .as_ref()
        .expect("validation failure should be reported");
    assert_eq!(failure.attempt, 1);
    assert_eq!(failure.max_attempts, 3);
    assert!(failure.retry_scheduled);
    assert_eq!(
        update.workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Failed
    );
    assert_eq!(
        update.dispatches[0].node_run.status(),
        WorkflowNodeRunStatus::Ready
    );
    std::fs::remove_file(schema_path).ok();
}

#[test]
fn valid_correction_completes_without_exposing_the_invalid_candidate() {
    let mut service = SessionService::new(&test_config());
    let (session, run, _node_id, schema_path) =
        create_schema_bound_completion_workflow(&mut service);
    let first_node_run_id = run.node_runs()[0].id().to_string();
    let invalid = service
        .complete_workflow_node_run(
            session.id(),
            run.id(),
            &first_node_run_id,
            Some(completion_with_message(r#"{"stale_challenge":"OLD"}"#)),
            None,
        )
        .expect("invalid output should schedule correction");
    let validation_failure = invalid
        .run_output_validation_failure
        .as_ref()
        .expect("validation failure should be reported");
    service
        .record_workflow_failure_event(
            session.id(),
            run.id(),
            crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                first_node_run_id,
                Vec::new(),
                validation_failure.message.clone(),
            ),
        )
        .expect("validation failure should be recorded");
    let retry_node_run_id = invalid.dispatches[0].node_run.id().to_string();
    service
        .start_workflow_node_run(session.id(), run.id(), &retry_node_run_id)
        .expect("correction turn should start");

    let corrected = service
        .complete_workflow_node_run(
            session.id(),
            run.id(),
            &retry_node_run_id,
            Some(completion_with_message(
                r#"{"invocation_challenge":"ROTATED-CHALLENGE"}"#,
            )),
            None,
        )
        .expect("valid correction should complete");

    assert_eq!(
        corrected.workflow_run.status(),
        WorkflowRunStatus::Completed
    );
    assert_eq!(corrected.workflow_run.final_output_valid(), Some(true));
    assert_eq!(
        corrected
            .workflow_run
            .final_output()
            .map(|output| output.message()),
        Some(r#"{"invocation_challenge":"ROTATED-CHALLENGE"}"#)
    );
    assert!(corrected.dispatches.is_empty());
    assert!(corrected
        .workflow_run
        .failure_events()
        .iter()
        .all(|event| event.kind()
            != crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed));
    std::fs::remove_file(schema_path).ok();
}

#[test]
fn repeated_invalid_terminal_output_fails_after_the_correction_budget() {
    let mut service = SessionService::new(&test_config());
    let (session, run, _node_id, schema_path) =
        create_schema_bound_completion_workflow(&mut service);
    let mut node_run_id = run.node_runs()[0].id().to_string();

    for expected_attempt in 1..=3 {
        let update = service
            .complete_workflow_node_run(
                session.id(),
                run.id(),
                &node_run_id,
                Some(completion_with_message(r#"{"stale_challenge":"OLD"}"#)),
                None,
            )
            .expect("invalid output should produce a bounded failure update");
        let failure = update
            .run_output_validation_failure
            .as_ref()
            .expect("validation failure should be reported");
        assert_eq!(failure.attempt, expected_attempt);
        assert_eq!(failure.max_attempts, 3);
        service
            .record_workflow_failure_event(
                session.id(),
                run.id(),
                crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    node_run_id.clone(),
                    Vec::new(),
                    failure.message.clone(),
                ),
            )
            .expect("validation failure should be recorded");

        if expected_attempt < 3 {
            assert!(failure.retry_scheduled);
            assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Waiting);
            assert_eq!(update.dispatches.len(), 1);
            node_run_id = update.dispatches[0].node_run.id().to_string();
            service
                .start_workflow_node_run(session.id(), run.id(), &node_run_id)
                .expect("correction turn should start");
        } else {
            assert!(!failure.retry_scheduled);
            assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Failed);
            assert!(update.dispatches.is_empty());
            assert!(update.workflow_run.final_output().is_none());
        }
    }

    std::fs::remove_file(schema_path).ok();
}
