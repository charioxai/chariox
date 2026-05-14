use super::*;

#[tokio::test]
async fn stale_workflow_revision_rejects_graph_mutation_before_state_changes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-workflow-revision",
            "worktree-workflow-revision",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let first_agent = spawn_test_agent(&mut app, &session_id, "first", "dev-stub");
    let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let create_workflow = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
        session_id: session_id.clone(),
        alias: Some("revision-flow".to_string()),
    });
    let workflow = match router
        .dispatch(
            KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
            create_workflow,
        )
        .await
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        other => panic!("unexpected workflow response: {other:?}"),
    };
    assert_eq!(workflow.revision(), 0);

    let add_first = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow.id().to_string(),
        agent_id: first_agent.id().to_string(),
        expected_workflow_revision: Some(workflow.revision()),
    });
    let workflow = match router
        .dispatch(
            KernelCommand::from_local_request("add-first", None, None, &add_first),
            add_first,
        )
        .await
        .expect("first mutation should match revision")
    {
        LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => workflow,
        other => panic!("unexpected add response: {other:?}"),
    };
    assert_eq!(workflow.revision(), 1);

    let stale_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow.id().to_string(),
        agent_id: second_agent.id().to_string(),
        expected_workflow_revision: Some(0),
    });
    let rejected = router
        .dispatch(
            KernelCommand::from_local_request("stale-add", None, None, &stale_add),
            stale_add,
        )
        .await
        .expect_err("stale revision should reject before mutation");
    assert!(matches!(
        rejected,
        DaemonError::WorkflowRevisionConflict {
            expected_revision: 0,
            current_revision: 1,
            ..
        }
    ));

    let resolve = LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow.id().to_string(),
    });
    match router
        .dispatch(
            KernelCommand::from_local_request("resolve-after-stale", None, None, &resolve),
            resolve,
        )
        .await
        .expect("workflow should resolve")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => {
            assert_eq!(workflow.revision(), 1);
            assert_eq!(workflow.nodes().len(), 1);
            assert_eq!(workflow.nodes()[0].agent_id(), first_agent.id());
        }
        other => panic!("unexpected resolve response: {other:?}"),
    }

    let fresh_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id,
        workflow_ref: workflow.id().to_string(),
        agent_id: second_agent.id().to_string(),
        expected_workflow_revision: Some(workflow.revision()),
    });
    match router
        .dispatch(
            KernelCommand::from_local_request("fresh-add", None, None, &fresh_add),
            fresh_add,
        )
        .await
        .expect("fresh revision should succeed")
    {
        LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => {
            assert_eq!(workflow.revision(), 2);
            assert_eq!(workflow.nodes().len(), 2);
        }
        other => panic!("unexpected fresh add response: {other:?}"),
    }
}
