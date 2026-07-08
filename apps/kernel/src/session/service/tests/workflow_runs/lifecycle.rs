use super::*;

#[test]
fn creates_lists_resolves_and_cancels_workflow_runs() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    assert_eq!(workflow_run.entry_node_id(), node.id());
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(
        workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Ready
    );
    assert_eq!(workflow_run.messages().len(), 1);
    assert_eq!(workflow_run.messages()[0].target_node_id(), node.id());

    let listed = service
        .list_workflow_runs(session.id(), Some(workflow.id()))
        .expect("workflow runs should list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), workflow_run.id());

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.id(), workflow_run.id());

    let cancelled = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect("workflow run should cancel");
    assert_eq!(cancelled.status(), WorkflowRunStatus::Stopped);
    assert_eq!(cancelled.active_node_run_id(), None);
    assert_eq!(
        cancelled.node_runs()[0].status(),
        WorkflowNodeRunStatus::Stopped
    );

    let error = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect_err("terminal workflow run should reject a second cancellation");
    assert!(matches!(error, DaemonError::InvalidWorkflowRunState { .. }));
}

#[test]
fn workflow_run_keeps_publication_invocation_metadata_separate_from_prompt() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "published", "agent-1");
    let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
        publication_id: "publication-1".to_string(),
        hook_id: Some("hook-1".to_string()),
        invocation_id: "req-1".to_string(),
        transport: "human_http".to_string(),
        endpoint_id: endpoint.id().to_string(),
        queue_ref: Some("default".to_string()),
        input: serde_json::json!({ "prompt": "make tea" }),
        artifacts: Vec::new(),
        mode: Some("sync".to_string()),
        caller: serde_json::json!({ "type": "anonymous" }),
    };

    let workflow_run = service
        .invoke_workflow_endpoint_with_publication_invocation(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("make tea".to_string()),
            Some(publication_invocation),
        )
        .expect("workflow run should be created");

    assert_eq!(workflow_run.invocation_prompt(), Some("make tea"));
    let metadata = workflow_run
        .publication_invocation()
        .expect("publication invocation should be stored on workflow run");
    assert_eq!(metadata.invocation_id(), "req-1");
    assert_eq!(metadata.input, serde_json::json!({ "prompt": "make tea" }));
}

#[test]
fn provider_failure_marks_workflow_and_node_failed() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();

    let failed = service
        .fail_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should fail");

    assert_eq!(failed.status(), WorkflowRunStatus::Failed);
    assert_eq!(failed.active_node_run_id(), None);
    assert_eq!(
        failed.node_runs()[0].status(),
        WorkflowNodeRunStatus::Failed
    );
}

#[test]
fn node_turn_budget_exhaustion_stops_the_whole_run() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    service
        .set_workflow_node_max_turns(session.id(), workflow.id(), node.id(), Some(1))
        .expect("node max turns should update");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist");

    let update = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            node_run.id(),
            Some(WorkflowCompletionSnapshot::new("done", None)),
            None,
        )
        .expect("node completion should succeed");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Stopped);
    assert!(update.dispatches.is_empty());
    assert!(update.workflow_run.final_output().is_none());
}
