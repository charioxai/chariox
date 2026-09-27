use super::*;

#[tokio::test]
async fn archived_validated_publication_settlement_releases_claim_and_retries_successor() {
    let worktree = crate::test_support::TestWorktree::new("workflow-claim-release-archived");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let holder = spawn_shared_worktree_agent(&mut app, session.id(), "holder", worktree.path());
    let successor =
        spawn_shared_worktree_agent(&mut app, session.id(), "successor", worktree.path());
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-test",
    )
    .with_agent_id(&holder);
    let mut provider_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-workflow-claim-release",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-workflow-claim-release".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-workflow-claim-release".to_string()),
        },
    );
    provider_run.mark_running();
    spawn_inert_pty_for_run(&mut app, provider_run.id());
    app.providers_mut().insert_run_for_test(provider_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(provider_run.clone());
    let successor_provider = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "test-model",
            )
            .with_agent_id(&successor),
        )
        .expect("successor provider should launch");
    app.update_provider_run_projection(successor_provider);

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("published-holder".to_string()))
        .expect("publication workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), &holder)
        .expect("publication node should be added");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("publication node should complete the run");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("publication endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint_with_publication_invocation(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("complete the published workflow".to_string()),
            Some(crate::session::WorkflowPublicationInvocationEnvelope {
                publication_id: "publication-claim-release".to_string(),
                hook_id: Some("hook-claim-release".to_string()),
                invocation_id: "request-claim-release".to_string(),
                transport: "human_http".to_string(),
                endpoint_id: endpoint.id().to_string(),
                queue_ref: Some("default".to_string()),
                input: serde_json::json!({
                    "prompt": "complete the published workflow"
                }),
                artifacts: Vec::new(),
                mode: Some("sync".to_string()),
                caller: serde_json::json!({"type":"anonymous"}),
            }),
        )
        .expect("publication workflow should be invoked");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "publication workflow prompt".to_string(),
            None,
            None,
        )
        .expect("publication workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("publication node should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        holder.clone(),
        "publication workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("publication prompt should start")
    else {
        panic!("publication prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        &holder,
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(provider_run.id().to_string()),
        provider_run.provider_session_id().map(str::to_string),
    )
    .expect("provider delivery should be acknowledged");
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());
    let claim_id = format!(
        "workflow-node:{}:{}:{}",
        session.id(),
        workflow_run.id(),
        node_run_id
    );
    app.acquire_workflow_node_workspace_claim(
        session.id(),
        &claim_id,
        &holder,
        workflow_run.id(),
        &node_run_id,
    )
    .expect("publication workflow should own the shared worktree");

    let (successor_run, successor_node) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-successor", &successor);
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should exist")
            .workflow_run(successor_run.id())
            .expect("successor workflow should exist")
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == successor_node)
            .map(|node_run| node_run.status()),
        Some(crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim),
        "the successor should wait on the publication workflow's write claim"
    );

    let app = Arc::new(Mutex::new(app));
    let _pty_cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: provider_run.id().to_string(),
    };
    let runtime = owned_runtime_state(&app).await;
    let running_workflow = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist")
        .workflow_run(workflow_run.id())
        .expect("publication workflow should still be active")
        .clone();
    assert_eq!(
        runtime
            .owned
            .release_completed_workflow_write_claims(session.id(), &running_workflow),
        0,
        "a noncompleted workflow must retain its live write claim"
    );
    assert!(runtime.owned.prompt_workspace_claims.contains(&claim_id));
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
        .dispatch_workflow_runtime_tool_call(
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
                .to_string(),
            serde_json::json!({
                "workflow_output_json": "{\"status\":\"done\"}"
            }),
            context,
        )
        .expect("validated final output should submit");
    assert_eq!(result.payload["valid"], true);
    assert!(
        runtime.owned.prompt_workspace_claims.contains(&claim_id),
        "the claim must remain while the provider is still finishing"
    );

    // Recreate the archive race: the prompt owner still holds the active prompt, but
    // the runtime-session mirror has lost it when a terminal snapshot is archived.
    runtime
        .owned
        .session_store
        .write()
        .mirror_agent_prompt_state(
            session.id(),
            &holder,
            None,
            std::collections::VecDeque::new(),
        )
        .expect("runtime prompt mirror should update");
    let archived = runtime
        .owned
        .session_store
        .write()
        .archive_terminal_workflow_runs(session.id())
        .expect("terminal workflow should archive");
    assert!(archived.iter().any(|run| run.id() == workflow_run.id()));
    assert_eq!(
        runtime.owned.release_archived_completed_workflow_claims(
            session.id(),
            workflow_run.id(),
            "unmatched-node-run",
        ),
        None,
        "archived completion evidence for another node must not release a live claim"
    );
    assert!(runtime.owned.prompt_workspace_claims.contains(&claim_id));

    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), provider_run.id(), true, false, false)
        .await
        .expect("provider completion should settle the archived validated workflow");
    assert!(settlement.had_active_prompt);
    assert!(
        !runtime.owned.prompt_workspace_claims.contains(&claim_id),
        "archived completion should release the write claim"
    );
    let durable_run = runtime
        .owned
        .durable_state_store
        .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run.id())
        .expect("durable workflow should resolve")
        .expect("validated workflow should remain durable");
    assert_eq!(
        runtime
            .owned
            .release_completed_workflow_write_claims(session.id(), &durable_run),
        0,
        "releasing a completed run twice must be a no-op"
    );

    let settled_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should remain available");
    let successor_status = settled_session
        .workflow_run(successor_run.id())
        .expect("successor run should remain active")
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == successor_node)
        .map(|node_run| node_run.status());
    assert!(
        matches!(
            successor_status,
            Some(
                crate::session::WorkflowNodeRunStatus::Ready
                    | crate::session::WorkflowNodeRunStatus::Running
            )
        ),
        "released claim should make the queued successor runnable, got {successor_status:?}"
    );
}
