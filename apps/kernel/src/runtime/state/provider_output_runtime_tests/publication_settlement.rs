use super::*;

#[tokio::test]
async fn completed_publication_output_releases_workflow_workspace_claim() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-publication-claim",
            "worktree-publication-claim",
        ))
        .expect("session should be created");
    let provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-test",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(provider_run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("published".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be added");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("workflow node should complete the run");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
        publication_id: "publication-1".to_string(),
        hook_id: Some("hook-1".to_string()),
        invocation_id: "request-1".to_string(),
        transport: "human_http".to_string(),
        endpoint_id: endpoint.id().to_string(),
        queue_ref: Some("default".to_string()),
        input: serde_json::json!({ "prompt": "render a dashboard" }),
        artifacts: Vec::new(),
        mode: Some("sync".to_string()),
        caller: serde_json::json!({ "type": "anonymous" }),
    };
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint_with_publication_invocation(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("render a dashboard".to_string()),
            Some(publication_invocation),
        )
        .expect("published workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    let claim_id = format!(
        "workflow-node:{}:{}:{}",
        session.id(),
        workflow_run.id(),
        node_run_id,
    );
    app.acquire_workflow_node_workspace_claim(
        session.id(),
        &claim_id,
        agent.id(),
        workflow_run.id(),
        &node_run_id,
    )
    .expect("workflow workspace claim should be acquired");
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let context = crate::transport::runtime_tools::WorkflowRuntimeToolContext {
        session_id: session.id().to_string(),
        workflow_run_ref: workflow_run.id().to_string(),
        workflow_node_run_id: node_run_id.clone(),
        delivery_token: None,
        allowed_handoff_schema_refs: Vec::new(),
        workflow_run_output_schema_ref: None,
        workflow_intermediate_output_schema_ref: None,
        can_complete_workflow_run: true,
        can_emit_intermediate_workflow_run_output: true,
    };
    let (result, _) = runtime
        .owned
        .workflow_submit_output_tool_result(
            &serde_json::json!({ "workflow_output_json": "{\"status\":\"done\"}" }),
            &context,
            true,
        )
        .expect("final publication output should settle");

    assert_eq!(result.payload["valid"], true);
    assert!(
        !runtime.owned.prompt_workspace_claims.contains(&claim_id),
        "fast publication completion must release the workflow workspace claim"
    );
}
