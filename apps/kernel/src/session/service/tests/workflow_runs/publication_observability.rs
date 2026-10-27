use super::*;

#[test]
fn publication_runtime_observability_applies_trace_exposure() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("published".to_string()))
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
            Some("prompt".to_string()),
        )
        .expect("workflow run should be created");
    let run_id = workflow_run.id().to_string();
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    {
        let session_mut = service
            .store
            .get_mut(session.id())
            .expect("session should exist");
        let run_mut = session_mut
            .workflow_run_mut(&run_id)
            .expect("run should exist");
        let node_run = run_mut
            .node_run_mut(&node_run_id)
            .expect("node run should exist");
        node_run.set_summary(Some("TRACE_SUMMARY".to_string()));
        node_run.set_completion(Some(WorkflowCompletionSnapshot::new(
            "TRACE_SUMMARY",
            Some(crate::session::WorkflowOutputPayload::new(
                "TRACE_ASSISTANT",
                Vec::new(),
            )),
        )));
        node_run.add_thinking_trace("TRACE_THINKING");
        let mut envelope =
            crate::session::WorkflowTurnEnvelope::new("token-1", "prompt".to_string(), None, None);
        envelope.add_runtime_tool_call(crate::session::WorkflowRuntimeToolCallEvent::new(
            "workflow_console_write",
            "{\"text\":\"TRACE_TOOL\"}",
            Some("{\"ok\":true}".to_string()),
            true,
        ));
        node_run.set_turn_envelope(Some(envelope));
        node_run.set_status(WorkflowNodeRunStatus::Completed);
        run_mut.set_final_output(
            Some(crate::session::WorkflowOutputPayload::new(
                "TRACE_FINAL",
                Vec::new(),
            )),
            Some(true),
            None,
            Some(node_run_id.clone()),
        );
        run_mut.set_status(WorkflowRunStatus::Completed);
    }

    let hidden_publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("default".to_string()),
            Some("hidden".to_string()),
            Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
            Some("/*".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            None,
            Some("sync".to_string()),
            None,
            None,
            "local".to_string(),
        )
        .expect("publication should be created");
    let hidden_publication = service
        .mark_workflow_publication_runtime_status(
            session.id(),
            hidden_publication.id(),
            "running",
            None,
            None,
        )
        .expect("runtime status should update");
    let hidden_text =
        serde_json::to_string(&hidden_publication).expect("publication should serialize");
    assert!(hidden_text.contains("TRACE_FINAL"));
    assert!(!hidden_text.contains("TRACE_SUMMARY"));
    assert!(!hidden_text.contains("TRACE_ASSISTANT"));
    assert!(!hidden_text.contains("TRACE_THINKING"));
    assert!(!hidden_text.contains("TRACE_TOOL"));

    let exposed_publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("default".to_string()),
            Some("exposed".to_string()),
            Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
            Some("/*".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            Some(serde_json::json!({
                "nodes": {
                    node.id(): ["output_summary", "assistant_messages", "thinking", "tool_use"]
                }
            })),
            Some("sync".to_string()),
            None,
            None,
            "local".to_string(),
        )
        .expect("publication should be created");
    let exposed_publication = service
        .mark_workflow_publication_runtime_status(
            session.id(),
            exposed_publication.id(),
            "running",
            None,
            None,
        )
        .expect("runtime status should update");
    let exposed_text =
        serde_json::to_string(&exposed_publication).expect("publication should serialize");
    assert!(exposed_text.contains("TRACE_SUMMARY"));
    assert!(exposed_text.contains("TRACE_ASSISTANT"));
    assert!(exposed_text.contains("TRACE_THINKING"));
    assert!(exposed_text.contains("runtime_tool_calls"));
    assert!(exposed_text.contains("workflow_console_write"));
    assert!(!exposed_text.contains("arguments_json"));
    assert!(!exposed_text.contains("result_json"));
    assert!(!exposed_text.contains("TRACE_TOOL"));
}
