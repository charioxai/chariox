use super::*;

#[test]
fn local_request_api_queues_workflow_code_run_behind_active_meta_task() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code local API test because node is not available");
        return;
    };
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-meta-queue", "worktree-meta-queue"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.agents_mut()
            .activate_agent_meta_mode(agent.id(), None)
            .expect("agent should enter meta mode");
        app.sessions_mut()
            .start_or_update_metaagent_task(session.id(), agent.id(), "finish the active task")
            .expect("meta task should start");
    });
    let source = format!(
        r#"
workflow.define({{ alias: "queued_behind_meta" }})
const worker = workflow.node({{
  handle: "worker",
  agent: workflow.existingAgent("{}"),
  instructions: "Complete the queued task.",
  canCompleteWorkflowRun: true
}})
workflow.endpoint(worker, {{ handle: "entry", alias: "entry" }})
"#,
        agent.id()
    );

    let response = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCode(
            crate::local::RunWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source,
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: None,
                prompt: "run after Meta".to_string(),
            },
        ))
        .expect("workflow-code run should enqueue behind active Meta");

    let LocalDaemonResponse::WorkflowCodeRun {
        result,
        session: queued_session,
    } = response
    else {
        panic!("unexpected local response");
    };
    let crate::workflow_code::WorkflowCodeRunInvocation::Enqueued { queued_prompt, .. } =
        result.invocation
    else {
        panic!("workflow-code run should be enqueued");
    };
    assert_eq!(queued_prompt.prompt(), Some("run after Meta"));
    assert_eq!(queued_session.workflow_queued_prompts().len(), 1);
    assert!(queued_session.has_active_metaagent_task());
    assert!(queued_session.workflow_runs().is_empty());
}

#[test]
fn local_request_api_validates_and_applies_workflow_code() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code local API test because node is not available");
        return;
    };
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-workflow-code", "worktree-workflow-code"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
workflow.define({
  alias: "scripted_flow",
  flushAgentContextBeforeRun: true,
  maxConcurrent: 8
})
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan the task and produce a concise answer.",
  canCompleteWorkflowRun: true,
  canvas: { x: 20, y: 40 }
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

    let validated = harness
        .dispatch(LocalDaemonRequest::ValidateWorkflowCode(
            crate::local::ValidateWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("workflow-code should validate");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
            assert_eq!(
                result.definition.workflow.alias.as_deref(),
                Some("scripted_flow")
            );
        }
        _ => panic!("unexpected local response"),
    }
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => assert!(workflows.is_empty()),
        _ => panic!("unexpected local response"),
    }

    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
            crate::local::ApplyWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("workflow-code should apply");
    match applied {
        LocalDaemonResponse::WorkflowCodeApplied { result, session } => {
            assert!(result.compile.validation.ok);
            assert_eq!(result.apply.node_ids.len(), 1);
            assert_eq!(result.apply.agent_ids.len(), 1);
            assert!(session
                .workflows()
                .iter()
                .any(|workflow| workflow.id() == result.apply.workflow_id));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_runs_workflow_code_with_generated_agent() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code run local API test because node is not available");
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-run-{}",
        crate::session::unix_epoch_ms()
    ));
    let worktree_root = workspace_root.join("worktree");
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace_root.display().to_string(),
                worktree_root.display().to_string(),
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
const finalOutput = workflow.schema({
  handle: "final",
  alias: "Final",
  schema: {
    type: "object",
    required: ["value"],
    properties: {
      value: { type: "number" }
    },
    additionalProperties: false
  }
})
const progressOutput = workflow.schema({
  handle: "progress",
  alias: "Progress",
  schema: {
    type: "object",
    required: ["event", "status"],
    properties: {
      event: { type: "string" },
      status: { type: "string" }
    },
    additionalProperties: false
  }
})
workflow.define({
  alias: "scripted_run_flow",
  prompt: "Use the workflow-code default invocation prompt.",
  maxConcurrent: 4,
  runOutputSchema: finalOutput
})
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "run-planner", provider: "dev-stub", model: "default" }),
  publicLabel: "Planner",
  instructions: "Submit schema-valid intermediate progress and complete the workflow run with final JSON.",
  canCompleteWorkflowRun: true,
  canEmitIntermediateRunOutput: true,
  intermediateOutputSchema: progressOutput,
  maxTurns: 2,
  canvas: { x: 24, y: 48 }
})
workflow.queue({
  handle: "fast_lane",
  alias: "urgent",
  priority: 9,
  enabled: true
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

    let response = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCode(
            crate::local::RunWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: Some("fast_lane".to_string()),
                prompt: String::new(),
            },
        ))
        .expect("workflow-code should apply and run");

    let LocalDaemonResponse::WorkflowCodeRun {
        result,
        session: run_session,
    } = response
    else {
        panic!("unexpected local response");
    };
    assert!(
        result.apply.compile.validation.ok,
        "{:?}",
        result.apply.compile.validation.diagnostics
    );
    assert_eq!(
        result.apply.compile.definition.workflow.alias.as_deref(),
        Some("scripted_run_flow")
    );
    assert_eq!(result.apply.apply.schema_refs.len(), 2);
    assert_eq!(result.apply.apply.node_ids.len(), 1);
    assert_eq!(result.apply.apply.agent_ids.len(), 1);
    assert_eq!(result.apply.apply.endpoint_ids.len(), 1);
    assert!(result.apply.apply.canvas_layout_applied);
    assert!(run_session
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("run-planner")
            && agent.provider() == "dev-stub"
            && agent.model() == Some("default")
            && Some(agent.id())
                == result
                    .apply
                    .apply
                    .agent_ids
                    .get("planner")
                    .map(String::as_str)));
    let workflow = run_session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == result.apply.apply.workflow_id)
        .expect("generated workflow should be in returned session");
    assert_eq!(workflow.alias(), Some("scripted_run_flow"));
    assert_eq!(workflow.schemas().len(), 2);
    assert_eq!(
        workflow.run_output_schema_ref(),
        result
            .apply
            .apply
            .schema_refs
            .get("final")
            .map(String::as_str)
    );
    let planner_node_id = result
        .apply
        .apply
        .node_ids
        .get("planner")
        .expect("planner node id should be reported");
    let planner_node = workflow
        .nodes()
        .iter()
        .find(|node| node.id() == planner_node_id)
        .expect("planner node should be materialized");
    assert!(planner_node.can_emit_intermediate_run_output());
    assert_eq!(
        planner_node.intermediate_output_schema_ref(),
        result
            .apply
            .apply
            .schema_refs
            .get("progress")
            .map(String::as_str)
    );
    assert!(workflow
        .endpoints()
        .iter()
        .any(|endpoint| endpoint.alias() == Some("entry")
            && Some(endpoint.id())
                == result
                    .apply
                    .apply
                    .endpoint_ids
                    .get("entry")
                    .map(String::as_str)));
    assert!(run_session.workflow_prompt_queues().iter().any(|queue| {
        queue.workflow_id() == workflow.id()
            && Some(queue.id())
                == result
                    .apply
                    .apply
                    .queue_ids
                    .get("fast_lane")
                    .map(String::as_str)
            && queue.alias() == "urgent"
            && queue.priority() == 9
    }));

    let (workflow_run, workflow_from_run, endpoint_from_run) = match result.invocation {
        crate::workflow_code::WorkflowCodeRunInvocation::Started {
            workflow_run,
            workflow,
            endpoint,
        } => (workflow_run, workflow, endpoint),
        crate::workflow_code::WorkflowCodeRunInvocation::Enqueued { .. } => {
            panic!("single generated-node workflow should start immediately")
        }
    };
    assert_eq!(workflow_from_run.id(), workflow.id());
    assert_eq!(
        endpoint_from_run.id(),
        result
            .apply
            .apply
            .endpoint_ids
            .get("entry")
            .expect("entry endpoint id should be reported")
    );
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint_from_run.id());
    assert_eq!(
        workflow_run.invocation_prompt(),
        Some("Use the workflow-code default invocation prompt.")
    );
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert!(run_session
        .workflow_runs()
        .iter()
        .any(|run| run.id() == workflow_run.id()));
    let durable_events = harness.with_app(|app| {
        app.durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load")
    });
    assert!(durable_events
        .iter()
        .any(|event| event.kind == "workflow_code.applied"
            && event.subject_id.as_deref() == Some(workflow.id())));
    let run_event = durable_events
        .iter()
        .find(|event| {
            event.kind == "workflow_code.run" && event.subject_id.as_deref() == Some(workflow.id())
        })
        .expect("workflow-code run should persist a durable audit event");
    assert_eq!(run_event.payload["session_id"], session.id());
    assert_eq!(run_event.payload["caller_user_id"], "local");
    assert_eq!(
        run_event.payload["controlled_by_metaagent_id"],
        serde_json::Value::Null
    );
    assert_eq!(run_event.payload["outcome"], "invoked");
    assert_eq!(run_event.payload["workflow_id"], workflow.id());
    assert_eq!(run_event.payload["endpoint_id"], endpoint_from_run.id());
    assert_eq!(run_event.payload["workflow_run_id"], workflow_run.id());

    let provider_run_id = harness.wait_for_active_provider_run(session.id());
    let runtime_mcp_auth_token = harness.with_app(|app| {
        app.providers()
            .get_run(&provider_run_id)
            .expect("active provider run should resolve")
            .runtime_mcp_auth_token()
            .expect("workflow provider should have runtime MCP auth token")
            .to_string()
    });
    let delivery_token = harness.with_app(|app| {
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve while reading workflow turn envelope");
        let run = session
            .workflow_run(workflow_run.id())
            .expect("workflow run should resolve while reading workflow turn envelope");
        run.node_runs()[0]
            .turn_envelope()
            .expect("workflow node run should have a delivery token")
            .delivery_token()
            .to_string()
    });

    let acked = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL,
            serde_json::json!({ "delivery_token": delivery_token.clone() }),
        )
        .expect("workflow turn ack should validate");
    assert!(acked.ok, "{:?}", acked.payload);

    let invalid_intermediate = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token.clone(),
                "workflow_output_json": "{\"value\":1841}"
            }),
        )
        .expect("invalid intermediate workflow output should return a validation result");
    assert!(
        invalid_intermediate.ok,
        "{:?}",
        invalid_intermediate.payload
    );
    assert_eq!(invalid_intermediate.payload["valid"], false);
    assert!(invalid_intermediate.payload["warning"]
        .as_str()
        .is_some_and(|warning| !warning.trim().is_empty()));

    let intermediate = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token.clone(),
                "workflow_output_json": "{\"event\":\"started\",\"status\":\"working\"}"
            }),
        )
        .expect("intermediate workflow output should validate");
    assert!(intermediate.ok, "{:?}", intermediate.payload);
    assert_eq!(intermediate.payload["valid"], true);

    let second_intermediate = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token.clone(),
                "workflow_output_json": "{\"event\":\"checked\",\"status\":\"still-working\"}"
            }),
        )
        .expect("second intermediate workflow output should validate in the same turn");
    assert!(second_intermediate.ok, "{:?}", second_intermediate.payload);
    assert_eq!(second_intermediate.payload["valid"], true);

    let invalid_final = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token.clone(),
                "workflow_output_json": "{\"value\":\"not-a-number\"}"
            }),
        )
        .expect("invalid final workflow output should return a validation result");
    assert!(invalid_final.ok, "{:?}", invalid_final.payload);
    assert_eq!(invalid_final.payload["valid"], false);
    assert!(invalid_final.payload["warning"]
        .as_str()
        .is_some_and(|warning| !warning.trim().is_empty()));

    let final_submission = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token,
                "workflow_output_json": "{\"value\":1842}"
            }),
        )
        .expect("final workflow output should validate");
    assert!(final_submission.ok, "{:?}", final_submission.payload);
    assert_eq!(final_submission.payload["valid"], true);

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow-code prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let completed_run = loop {
        let resolved = match harness
            .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        if resolved.status() == WorkflowRunStatus::Completed || Instant::now() >= deadline {
            break resolved;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let recent_history = if completed_run.status() != WorkflowRunStatus::Completed {
        harness.with_app_mut(|app| {
            crate::app::KernelSessionReadService::new(app)
                .session_history(session.id())
                .expect("session history should load")
                .into_iter()
                .rev()
                .take(8)
                .map(|entry| {
                    format!(
                        "{:?} {:?}: {}",
                        entry.kind, entry.provider_run_id, entry.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n---\n")
        })
    } else {
        String::new()
    };
    assert_eq!(
        completed_run.status(),
        WorkflowRunStatus::Completed,
        "workflow-code run should complete; failures: {:?}; recent history:\n{}",
        completed_run.failure_events(),
        recent_history
    );
    assert_eq!(completed_run.intermediate_outputs().len(), 2);
    let intermediate_events = completed_run
        .intermediate_outputs()
        .iter()
        .map(|intermediate| {
            assert!(intermediate.valid());
            assert_eq!(intermediate.warning(), None);
            serde_json::from_str::<serde_json::Value>(intermediate.output().message())
                .expect("intermediate output message should be JSON")
        })
        .collect::<Vec<_>>();
    assert_eq!(intermediate_events[0]["event"], "started");
    assert_eq!(intermediate_events[0]["status"], "working");
    assert_eq!(intermediate_events[1]["event"], "checked");
    assert_eq!(intermediate_events[1]["status"], "still-working");
    assert_eq!(completed_run.final_output_valid(), Some(true));
    assert_eq!(completed_run.final_output_warning(), None);
    let final_output = completed_run
        .final_output()
        .expect("completed workflow run should store final output");
    let final_json: serde_json::Value =
        serde_json::from_str(final_output.message()).expect("final output message should be JSON");
    assert_eq!(final_json["value"], 1842);

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}
