use super::*;
use crate::local::{
    CreateWorkflowPublicationRequest, CreateWorkflowScheduleRequest,
    ExportWorkflowPublicationPackageRequest, InstallSkillRequest, RegisterEnvironmentRequest,
    RegisterScriptRequest, RegisterWorkflowPublicationEndpointRequest,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn find_node_for_workflow_code_local_api_test() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("NODE") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ]);
    candidates.into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn find_python_for_workflow_code_local_api_test() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PYTHON") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/python3"),
        PathBuf::from("/usr/local/bin/python3"),
        PathBuf::from("/usr/bin/python3"),
        PathBuf::from("python3"),
    ]);
    candidates.into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn node_supports_workflow_code_typescript(node: &std::path::Path) -> bool {
    std::process::Command::new(node)
        .arg("--no-warnings")
        .arg("--input-type=module")
        .arg("-e")
        .arg("const mod = await import('node:module'); if (typeof mod.stripTypeScriptTypes !== 'function') process.exit(1)")
        .status()
        .is_ok_and(|status| status.success())
}

fn workflow_code_test_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
        "arroba-workflow-code-run-{}",
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

#[test]
fn local_request_api_rejects_ambiguous_workflow_code_run_endpoint_without_applying() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping ambiguous workflow-code run local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-ambiguous-run-{}",
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
workflow.define({ alias: "ambiguous_run_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "ambiguous-worker", provider: "dev-stub", model: "default" }),
  instructions: "Complete the workflow run.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry_a", alias: "entry-a" })
workflow.endpoint(worker, { handle: "entry_b", alias: "entry-b" })
"#;

    let inline_error = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCode(
            crate::local::RunWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: None,
                queue_ref: None,
                prompt: "Run without selecting an endpoint.".to_string(),
            },
        ))
        .expect_err("ambiguous inline workflow-code run should fail before applying");
    assert!(
        format!("{inline_error:?}").contains("workflow-code defines 2 endpoints"),
        "{inline_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected inline run");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("ambiguous_run_flow")));
        }
        _ => panic!("unexpected local response"),
    }

    let missing_endpoint_error = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCode(
            crate::local::RunWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: Some("missing_entry".to_string()),
                queue_ref: None,
                prompt: "Run with a missing endpoint handle.".to_string(),
            },
        ))
        .expect_err("missing inline workflow-code endpoint handle should fail before applying");
    assert!(
        format!("{missing_endpoint_error:?}")
            .contains("workflow-code endpoint handle `missing_entry` is not defined"),
        "{missing_endpoint_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected missing endpoint run");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("ambiguous_run_flow")));
        }
        _ => panic!("unexpected local response"),
    }

    let artifact_name = format!("ambiguous-run-{}", crate::session::unix_epoch_ms());
    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: artifact_name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("ambiguous endpoint artifact should save because it is valid to apply");
    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let artifact_error = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCodeArtifact(
            crate::local::RunWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: artifact_name,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: None,
                queue_ref: None,
                prompt: "Run artifact without selecting an endpoint.".to_string(),
            },
        ))
        .expect_err("ambiguous artifact workflow-code run should fail before applying");
    assert!(
        format!("{artifact_error:?}").contains("workflow-code defines 2 endpoints"),
        "{artifact_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected artifact run");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("ambiguous_run_flow")));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_unknown_workflow_code_run_queue_without_applying() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping unknown workflow-code queue local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-missing-queue-{}",
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
workflow.define({ alias: "missing_queue_run_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "missing-queue-worker", provider: "dev-stub", model: "default" }),
  instructions: "Complete the workflow run.",
  canCompleteWorkflowRun: true
})
workflow.queue({ handle: "fast_lane", alias: "urgent", priority: 5 })
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let inline_error = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCode(
            crate::local::RunWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: Some("missing_queue".to_string()),
                prompt: "Run with a missing queue handle.".to_string(),
            },
        ))
        .expect_err("missing inline workflow-code queue handle should fail before applying");
    assert!(
        format!("{inline_error:?}")
            .contains("workflow-code queue handle `missing_queue` is not defined"),
        "{inline_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected inline queue run");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("missing_queue_run_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after_inline = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after_inline
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("missing-queue-worker")));

    let artifact_name = format!("missing-queue-run-{}", crate::session::unix_epoch_ms());
    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: artifact_name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("missing queue artifact should save because it is valid to apply");
    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let artifact_error = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCodeArtifact(
            crate::local::RunWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: artifact_name,
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: Some("missing_queue".to_string()),
                prompt: "Run artifact with a missing queue handle.".to_string(),
            },
        ))
        .expect_err("missing artifact workflow-code queue handle should fail before applying");
    assert!(
        format!("{artifact_error:?}")
            .contains("workflow-code queue handle `missing_queue` is not defined"),
        "{artifact_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected artifact queue run");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("missing_queue_run_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after_artifact = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after_artifact
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("missing-queue-worker")));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_duplicate_workflow_code_edges_without_applying() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping duplicate workflow-code edge local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-duplicate-edge-{}",
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
workflow.define({ alias: "duplicate_edge_flow" })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "duplicate-edge-planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan."
})
const reviewer = workflow.node({
  handle: "reviewer",
  agent: workflow.newAgent({ alias: "duplicate-edge-reviewer", provider: "dev-stub", model: "default" }),
  instructions: "Review.",
  canCompleteWorkflowRun: true
})
workflow.edge(planner, reviewer, { handle: "plan_to_review" })
workflow.edge(planner, reviewer, { handle: "plan_to_review_again" })
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
        .expect("duplicate edge workflow-code validate should return diagnostics");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(!result.validation.ok);
            assert!(result
                .validation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "duplicate_edge"
                    && diagnostic.handle.as_deref() == Some("plan_to_review_again")));
        }
        _ => panic!("unexpected local response"),
    }

    let apply_error = harness
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
        .expect_err("duplicate edge workflow-code apply should fail before applying");
    assert!(
        format!("{apply_error:?}").contains("duplicate_edge"),
        "{apply_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected duplicate edge apply");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("duplicate_edge_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("duplicate-edge-planner")
            || agent.alias() == Some("duplicate-edge-reviewer")));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_duplicate_workflow_code_endpoint_aliases_without_applying() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping duplicate workflow-code endpoint alias local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-duplicate-endpoint-{}",
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
workflow.define({ alias: "duplicate_endpoint_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "duplicate-endpoint-worker", provider: "dev-stub", model: "default" }),
  instructions: "Complete.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry_a", alias: "entry" })
workflow.endpoint(worker, { handle: "entry_b", alias: "ENTRY" })
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
        .expect("duplicate endpoint workflow-code validate should return diagnostics");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(!result.validation.ok);
            assert!(result
                .validation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "duplicate_endpoint_alias"
                    && diagnostic.handle.as_deref() == Some("entry_b")));
        }
        _ => panic!("unexpected local response"),
    }

    let apply_error = harness
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
        .expect_err("duplicate endpoint alias workflow-code apply should fail before applying");
    assert!(
        format!("{apply_error:?}").contains("duplicate_endpoint_alias"),
        "{apply_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected duplicate endpoint apply");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("duplicate_endpoint_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("duplicate-endpoint-worker")));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_workflow_code_over_runtime_queue_limit_without_applying() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping workflow-code runtime queue limit local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-queue-limit-{}",
        crate::session::unix_epoch_ms()
    ));
    let worktree_root = workspace_root.join("worktree");
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let mut config = crate::DaemonConfig::for_tests();
    config.user_config.workflow.max_queues_per_workflow = Some(2);
    config.user_config.workflow.code = Some(crate::config::UserWorkflowCodeConfig {
        max_queues: Some(4),
        ..crate::config::UserWorkflowCodeConfig::default()
    });
    let harness = LocalRouterTestHarness::with_config(config);
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
workflow.define({ alias: "queue_limit_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "queue-limit-worker", provider: "dev-stub", model: "default" }),
  instructions: "Complete.",
  canCompleteWorkflowRun: true
})
workflow.queue({ handle: "urgent", alias: "urgent", priority: 5 })
workflow.queue({ handle: "slow", alias: "slow", priority: -5 })
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
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
        .expect("over-limit workflow-code validate should return diagnostics");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(!result.validation.ok);
            assert!(result.validation.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "limit_exceeded"
                    && diagnostic
                        .message
                        .contains("queues count 3 exceeds configured limit 2")
            }));
        }
        _ => panic!("unexpected local response"),
    }

    let apply_error = harness
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
        .expect_err("over-limit workflow-code apply should fail before applying");
    assert!(
        format!("{apply_error:?}").contains("limit_exceeded"),
        "{apply_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected queue-limit apply");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("queue_limit_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after
        .agents()
        .iter()
        .any(|agent| agent.alias() == Some("queue-limit-worker")));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_workflow_code_over_session_agent_limit_without_spawning() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping workflow-code session agent limit local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-agent-limit-{}",
        crate::session::unix_epoch_ms()
    ));
    let worktree_root = workspace_root.join("worktree");
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let mut config = crate::DaemonConfig::for_tests();
    config.user_config.workflow.session_default_max_agents = Some(1);
    let harness = LocalRouterTestHarness::with_config(config);
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
workflow.define({ alias: "agent_limit_flow" })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "agent-limit-planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan."
})
const reviewer = workflow.node({
  handle: "reviewer",
  agent: workflow.newAgent({ alias: "agent-limit-reviewer", provider: "dev-stub", model: "default" }),
  instructions: "Review.",
  canCompleteWorkflowRun: true
})
workflow.edge(planner, reviewer, { handle: "plan_to_review" })
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
        .expect("over-agent-limit workflow-code validate should return diagnostics");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(!result.validation.ok);
            assert!(result
                .validation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "session_agent_limit_exceeded"));
        }
        _ => panic!("unexpected local response"),
    }

    let apply_error = harness
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
        .expect_err("over-agent-limit workflow-code apply should fail before spawning");
    assert!(
        format!("{apply_error:?}").contains("session_agent_limit_exceeded"),
        "{apply_error:?}"
    );

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed after rejected agent-limit apply");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert!(!workflows
                .iter()
                .any(|workflow| workflow.alias() == Some("agent_limit_flow")));
        }
        _ => panic!("unexpected local response"),
    }
    let session_after = harness.with_app(|app| {
        crate::app::KernelSessionReadService::new(app)
            .session_snapshot(session.id())
            .expect("session snapshot should load")
    });
    assert!(!session_after.agents().iter().any(|agent| {
        agent.alias() == Some("agent-limit-planner")
            || agent.alias() == Some("agent-limit-reviewer")
    }));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_applies_workflow_code_extensions_to_generated_agents() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code extension local API test because node is not available");
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-extension-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let skill_dir = workspace_root.join("workflow-code-skill");
    std::fs::create_dir_all(&skill_dir).expect("test skill directory should be created");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: workflow-code-skill\ndescription: Workflow-code generated agent extension fixture.\n---\nUse this skill only in workflow-code local API tests.\n",
    )
    .expect("test skill should be written");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-extension"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    match harness
        .dispatch(LocalDaemonRequest::InstallSkill(InstallSkillRequest {
            workspace_id: Some(workspace_root.display().to_string()),
            source_path: skill_dir,
        }))
        .expect("test skill should install")
    {
        LocalDaemonResponse::SkillInstalled { skill, .. } => {
            assert_eq!(skill.name, "workflow-code-skill");
        }
        _ => panic!("unexpected local response"),
    }

    let source = r#"
workflow.define({ alias: "scripted_extension_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "extension-worker", provider: "dev-stub", model: "default" }),
  publicLabel: "Worker",
  instructions: "Use the granted skill if needed.",
  canCompleteWorkflowRun: true,
  extensions: [
    { kind: "skill", name: "workflow-code-skill" }
  ]
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

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
        .expect("workflow-code with satisfied extension requirement should apply");
    let LocalDaemonResponse::WorkflowCodeApplied {
        result,
        session: applied_session,
    } = applied
    else {
        panic!("unexpected local response");
    };
    assert!(result.compile.validation.ok);
    let worker_agent_id = result
        .apply
        .agent_ids
        .get("worker")
        .expect("worker agent id should be reported");
    assert!(applied_session.agents().iter().any(|agent| {
        agent.id() == worker_agent_id
            && agent.alias() == Some("extension-worker")
            && agent.has_extension_grant(
                crate::extension::ExtensionKind::Skill,
                "workflow-code-skill",
            )
    }));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_applies_workflow_code_script_extensions_to_generated_agents() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping workflow-code script extension local API test because node is not available"
        );
        return;
    };
    let Some(python_path) = find_python_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping workflow-code script extension local API test because python3 is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-script-extension-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let script_path = workspace_root.join("workflow_code_script.py");
    std::fs::write(
        &script_path,
        r#"
def run(value: str = "ok") -> dict:
    """Return a deterministic workflow-code script extension result."""
    return {"value": value}

def test_run():
    assert run("test")["value"] == "test"
"#,
    )
    .expect("test script should be written");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace_root.display().to_string(),
                "worktree-script-extension",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    match harness
        .dispatch(LocalDaemonRequest::RegisterEnvironment(
            RegisterEnvironmentRequest {
                workspace_id: Some(workspace_root.display().to_string()),
                config: crate::script::ArrobaEnvironmentConfig {
                    name: "workflow-code-python".to_string(),
                    runtime: crate::script::ArrobaEnvironmentRuntime::Python {
                        python: python_path,
                    },
                },
            },
        ))
        .expect("test environment should register")
    {
        LocalDaemonResponse::EnvironmentRegistered { environment, .. } => {
            assert_eq!(environment.name, "workflow-code-python");
        }
        _ => panic!("unexpected local response"),
    }
    match harness
        .dispatch(LocalDaemonRequest::RegisterScript(RegisterScriptRequest {
            workspace_id: Some(workspace_root.display().to_string()),
            source_path: script_path,
            environment: "workflow-code-python".to_string(),
            name: Some("workflow-code-script".to_string()),
        }))
        .expect("test script should register")
    {
        LocalDaemonResponse::ScriptRegistered { script, .. } => {
            assert_eq!(script.name, "workflow-code-script");
        }
        _ => panic!("unexpected local response"),
    }

    let source = r#"
workflow.define({ alias: "scripted_script_extension_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "script-extension-worker", provider: "dev-stub", model: "default" }),
  publicLabel: "Worker",
  instructions: "Use the granted script extension if needed.",
  canCompleteWorkflowRun: true,
  extensions: [
    { kind: "script", name: "workflow-code-script", environment: "workflow-code-python" }
  ]
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

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
        .expect("workflow-code with registered script extension should apply");
    let LocalDaemonResponse::WorkflowCodeApplied {
        result,
        session: applied_session,
    } = applied
    else {
        panic!("unexpected local response");
    };
    assert!(result.compile.validation.ok);
    let worker_agent_id = result
        .apply
        .agent_ids
        .get("worker")
        .expect("worker agent id should be reported");
    let worker = applied_session
        .agents()
        .iter()
        .find(|agent| agent.id() == worker_agent_id)
        .expect("generated worker should be present");
    let grant = worker
        .extension_grants()
        .iter()
        .find(|grant| {
            grant.matches(
                &crate::extension::ExtensionKind::Script,
                "workflow-code-script",
            )
        })
        .expect("generated worker should receive the script extension grant");
    assert_eq!(grant.environment.as_deref(), Some("workflow-code-python"));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_applies_workflow_code_queues_and_watchdogs() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping workflow-code queue/watchdog local API test because node is not available"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-queues-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-queues"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
workflow.define({ alias: "queued_watchdog_flow" })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "queue-planner", provider: "dev-stub", model: "default" }),
  publicLabel: "Planner",
  instructions: "Process queued watchdog work.",
  canCompleteWorkflowRun: true
})
const entry = workflow.endpoint(planner, { handle: "entry", alias: "entry" })
const urgent = workflow.queue({
  handle: "urgent",
  alias: "urgent",
  priority: 7,
  enabled: true
})
workflow.watchdog(entry, {
  handle: "entry_watchdog",
  queue: urgent,
  enabled: false,
  intervalSeconds: 90,
  invocationPrompt: "Check whether urgent scripted workflow work needs attention.",
  policy: "queue",
  maxWakeups: 3
})
"#;

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
    let LocalDaemonResponse::WorkflowCodeApplied {
        result,
        session: applied_session,
    } = applied
    else {
        panic!("unexpected local response");
    };
    assert!(result.compile.validation.ok);
    assert!(result.apply.warnings.iter().any(|warning| {
        warning.code == "canvas_auto_layout_applied" && warning.handle.is_none()
    }));

    let workflow = applied_session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == result.apply.workflow_id)
        .expect("generated workflow should be returned");
    assert_eq!(workflow.alias(), Some("queued_watchdog_flow"));
    let urgent_queue_id = result
        .apply
        .queue_ids
        .get("urgent")
        .expect("urgent queue id should be reported");
    let entry_endpoint_id = result
        .apply
        .endpoint_ids
        .get("entry")
        .expect("entry endpoint id should be reported");
    let watchdog_id = result
        .apply
        .schedule_ids
        .get("entry_watchdog")
        .expect("watchdog id should be reported");

    let urgent_queue = applied_session
        .workflow_prompt_queues()
        .iter()
        .find(|queue| queue.id() == urgent_queue_id)
        .expect("urgent queue should exist in the session");
    assert_eq!(urgent_queue.workflow_id(), workflow.id());
    assert_eq!(urgent_queue.alias(), "urgent");
    assert_eq!(urgent_queue.priority(), 7);
    assert!(urgent_queue.enabled());

    let watchdog = applied_session
        .workflow_watchdogs()
        .iter()
        .find(|watchdog| watchdog.id() == watchdog_id)
        .expect("scripted watchdog should exist in the session");
    assert_eq!(watchdog.workflow_id(), workflow.id());
    assert_eq!(watchdog.endpoint_id(), entry_endpoint_id);
    assert_eq!(watchdog.queue_id(), Some(urgent_queue_id.as_str()));
    assert!(!watchdog.enabled());
    assert_eq!(watchdog.interval_seconds(), 90);
    assert_eq!(
        watchdog.invocation_prompt(),
        "Check whether urgent scripted workflow work needs attention."
    );
    assert_eq!(
        watchdog.policy(),
        crate::session::WorkflowWatchdogPolicy::Queue
    );
    assert_eq!(watchdog.max_wakeups(), Some(3));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_workflow_code_validate_checks_target_provider_rebindings() {
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
workflow.define({ alias: "portable_flow" })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "missing-provider", model: "default" }),
  publicLabel: "Planner",
  instructions: "Plan.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

    let missing_provider = harness
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
        .expect("workflow-code validate should return diagnostics");
    match missing_provider {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(!result.validation.ok);
            let node_handle = result.definition.nodes[0].handle.as_str();
            assert!(result
                .validation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unavailable_provider"
                    && diagnostic.handle.as_deref() == Some(node_handle)));
        }
        _ => panic!("unexpected local response"),
    }

    let rebound = harness
        .dispatch(LocalDaemonRequest::ValidateWorkflowCode(
            crate::local::ValidateWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: None,
                provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
                    node: "planner".to_string(),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }],
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("workflow-code validate should accept provider rebinding");
    match rebound {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
            match &result.definition.nodes[0].agent {
                crate::workflow_code::WorkflowCodeAgentBinding::Create(agent) => {
                    assert_eq!(agent.provider, "dev-stub");
                }
                crate::workflow_code::WorkflowCodeAgentBinding::Existing(_) => {
                    panic!("planner should be a generated agent")
                }
            }
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
}

#[test]
fn local_request_api_persists_workflow_code_artifacts() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code artifact local API test because node is not available");
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-artifact-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    std::fs::create_dir_all(workspace_root.join("schemas"))
        .expect("temporary schema directory should be created");
    let schema_path = workspace_root.join("schemas/final.json");
    std::fs::write(
        &schema_path,
        r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("temporary schema file should be written");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-artifact"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = format!("toy-{}", crate::session::unix_epoch_ms());
    let source = r#"
workflow.define({ alias: "artifact_flow" })
const final = workflow.schemaFromFile({
  handle: "final",
  path: "schemas/final.json",
  alias: "Final output"
})
workflow.define({ runOutputSchema: final })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "codex", model: "gpt-5" }),
  instructions: "Plan."
})
workflow.queue({
  handle: "fast_lane",
  alias: "urgent",
  priority: 9,
  enabled: true
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;
    let updated_source = source.replace("artifact_flow", "artifact_flow_updated");

    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("workflow-code artifact should create");
    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert_eq!(artifact.metadata.name, name);
            assert!(artifact.metadata.validation.ok);
            assert!(artifact.metadata.path.starts_with(&workspace_root));
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow")
            );
            assert_eq!(
                artifact.definition.workflow.run_output_schema.as_deref(),
                Some("final")
            );
            assert_eq!(
                artifact.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
        }
        _ => panic!("unexpected local response"),
    }

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(artifacts.iter().any(|artifact| artifact.name == name));
        }
        _ => panic!("unexpected local response"),
    }

    let loaded = harness
        .dispatch(LocalDaemonRequest::GetWorkflowCodeArtifact(
            crate::local::GetWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should load");
    match loaded {
        LocalDaemonResponse::WorkflowCodeArtifact { artifact } => {
            assert_eq!(artifact.source, source);
        }
        _ => panic!("unexpected local response"),
    }

    let updated = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeArtifact(
            crate::local::UpdateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: updated_source.clone(),
            },
        ))
        .expect("workflow-code artifact should update");
    match updated {
        LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact } => {
            assert_eq!(artifact.source, updated_source);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
        }
        _ => panic!("unexpected local response"),
    }

    let package = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeArtifact(
            crate::local::ExportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should export")
    {
        LocalDaemonResponse::WorkflowCodeArtifactExported { package } => {
            assert_eq!(package.name, name);
            assert_eq!(package.source, updated_source);
            assert_eq!(
                package.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert_eq!(
                package.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
            package
        }
        _ => panic!("unexpected local response"),
    };

    let package_alias = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodePackage(
            crate::local::ExportWorkflowCodePackageRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                target: None,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code package alias should export")
    {
        LocalDaemonResponse::WorkflowCodePackageExported { package } => {
            assert_eq!(package.name, name);
            assert_eq!(package.source, updated_source);
            package
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(package_alias.source_sha256, package.source_sha256);
    assert_eq!(package_alias.definition_sha256, package.definition_sha256);

    let inline_source = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
            crate::local::ExportWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                    name: name.clone(),
                },
                format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code inline source should export")
    {
        LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(inline_source.name, name);
    assert_eq!(inline_source.source, updated_source);
    assert!(inline_source.files.is_empty());

    let directory_source = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
            crate::local::ExportWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                    name: name.clone(),
                },
                format: crate::workflow_code::WorkflowCodeSourceExportFormat::Directory,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code directory source should export")
    {
        LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(directory_source.source_path, "workflow.js");
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "workflow.js"));
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "manifest.json"));
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "schemas/final-output.json"));
    let export_root = workspace_root.join("workflow-code-source-export");
    for file in &directory_source.files {
        let path = export_root.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("source export parent should create");
        }
        std::fs::write(path, &file.contents).expect("source export file should write");
    }
    let recompiled = crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
        &node_path,
        &directory_source.source,
        directory_source.language,
        &crate::config::WorkflowCodeLimitsConfig::default(),
        Some(&export_root),
    )
    .expect("directory workflow-code source export should recompile");
    assert!(recompiled.validation.ok);
    assert_eq!(
        recompiled.definition.workflow.alias.as_deref(),
        Some("artifact_flow_updated")
    );
    assert_eq!(
        recompiled.definition.workflow.run_output_schema.as_deref(),
        Some("final")
    );
    std::fs::remove_file(&schema_path)
        .expect("source schema file should be removable before portable import");

    let deleted = harness
        .dispatch(LocalDaemonRequest::DeleteWorkflowCodeArtifact(
            crate::local::DeleteWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should delete");
    match deleted {
        LocalDaemonResponse::WorkflowCodeArtifactDeleted {
            name: deleted,
            path,
        } => {
            assert_eq!(deleted, name);
            assert!(!path.exists());
        }
        _ => panic!("unexpected local response"),
    }

    let imported_name = format!("{name}-imported");
    let imported = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodeArtifact(
            crate::local::ImportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                package,
                name: Some(imported_name.clone()),
                overwrite: false,
                node_path: node_path.display().to_string(),
            },
        ))
        .expect("workflow-code artifact package should import");
    match imported {
        LocalDaemonResponse::WorkflowCodeArtifactImported { artifact } => {
            assert_eq!(artifact.metadata.name, imported_name);
            assert_eq!(artifact.source, updated_source);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert_eq!(
                artifact.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let package_imported_name = format!("{name}-package-imported");
    let package_imported = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodePackage(
            crate::local::ImportWorkflowCodePackageRequest {
                session_id: session.id().to_string(),
                package: package_alias,
                name: Some(package_imported_name.clone()),
                overwrite: false,
                node_path: node_path.display().to_string(),
            },
        ))
        .expect("workflow-code package alias should import");
    match package_imported {
        LocalDaemonResponse::WorkflowCodePackageImported { artifact } => {
            assert_eq!(artifact.metadata.name, package_imported_name);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let provider_rebindings = vec![crate::workflow_code::WorkflowCodeProviderRebinding {
        node: "planner".to_string(),
        provider: "dev-stub".to_string(),
        model: Some("default".to_string()),
        effort: None,
        account_profile: None,
    }];
    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCodeArtifact(
            crate::local::ApplyWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: imported_name.clone(),
                provider_rebindings: provider_rebindings.clone(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("imported workflow-code artifact should apply with provider rebinding");
    match applied {
        LocalDaemonResponse::WorkflowCodeApplied {
            result,
            session: applied_session,
        } => {
            assert_eq!(
                result.apply.schema_refs.get("final").map(String::as_str),
                applied_session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == result.apply.workflow_id)
                    .and_then(|workflow| workflow.run_output_schema_ref())
            );
            let planner_agent_id = result
                .apply
                .agent_ids
                .get("planner")
                .expect("planner agent id should be reported");
            assert!(applied_session.agents().iter().any(|agent| {
                agent.id() == planner_agent_id
                    && agent.provider() == "dev-stub"
                    && agent.model() == Some("default")
            }));
            let live_source = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        },
                        format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ))
                .expect("live workflow source should export")
            {
                LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
                _ => panic!("unexpected local response"),
            };
            assert_eq!(live_source.source_path, "workflow.js");
            assert!(live_source.source.contains("workflow.newAgent"));
            let live_recompiled =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &live_source.source,
                    live_source.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("live workflow source export should recompile");
            assert!(live_recompiled.validation.ok);
            assert_eq!(
                live_recompiled.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert!(live_recompiled
                .definition
                .nodes
                .iter()
                .any(|node| node.handle == "planner"));
            assert!(live_recompiled
                .definition
                .endpoints
                .iter()
                .any(|endpoint| endpoint.handle == "entry" && endpoint.entry_node == "planner"));
            assert!(matches!(
                &live_recompiled.definition.nodes[0].agent,
                crate::workflow_code::WorkflowCodeAgentBinding::Create(agent)
                    if agent.provider == "dev-stub"
            ));
            let workflow_package = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodePackage(
                    crate::local::ExportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        name: "workflow-package".to_string(),
                        target: Some(crate::local::WorkflowCodePackageExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        }),
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ))
                .expect("existing workflow should export as workflow-code package")
            {
                LocalDaemonResponse::WorkflowCodePackageExported { package } => package,
                _ => panic!("unexpected local response"),
            };
            assert_eq!(workflow_package.name, "workflow-package");
            assert!(workflow_package.source.contains("defineWorkflow"));
            workflow_package
                .validate_integrity()
                .expect("workflow package integrity should validate");
            let workflow_package_compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &workflow_package.source,
                    workflow_package.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("workflow package source should recompile");
            assert!(workflow_package_compile.validation.ok);
            assert_eq!(
                workflow_package_compile
                    .definition
                    .workflow
                    .alias
                    .as_deref(),
                Some("artifact_flow_updated")
            );
            let live_existing_agent_source = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        },
                        format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::ExistingAgents,
                    },
                ))
                .expect("live workflow source should export with existing agents")
            {
                LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
                _ => panic!("unexpected local response"),
            };
            assert!(live_existing_agent_source
                .source
                .contains("workflow.existingAgent"));
            let live_existing_agent_recompiled =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &live_existing_agent_source.source,
                    live_existing_agent_source.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("live workflow existing-agent source export should recompile");
            assert!(matches!(
                &live_existing_agent_recompiled.definition.nodes[0].agent,
                crate::workflow_code::WorkflowCodeAgentBinding::Existing(existing)
                    if existing.agent_ref == *planner_agent_id
            ));
        }
        _ => panic!("unexpected local response"),
    }

    let run = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCodeArtifact(
            crate::local::RunWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: imported_name.clone(),
                provider_rebindings,
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: Some("fast_lane".to_string()),
                prompt: "Run the portable imported artifact.".to_string(),
            },
        ))
        .expect("imported workflow-code artifact should run with provider rebinding");
    let run_result = match run {
        LocalDaemonResponse::WorkflowCodeRun { result, session } => {
            let planner_agent_id = result
                .apply
                .apply
                .agent_ids
                .get("planner")
                .expect("planner run agent id should be reported");
            assert!(session.agents().iter().any(|agent| {
                agent.id() == planner_agent_id
                    && agent.provider() == "dev-stub"
                    && agent.model() == Some("default")
            }));
            let queue_id = result
                .apply
                .apply
                .queue_ids
                .get("fast_lane")
                .expect("script queue handle should map to a runtime queue id");
            assert!(session.workflow_prompt_queues().iter().any(|queue| {
                queue.id() == queue_id && queue.alias() == "urgent" && queue.priority() == 9
            }));
            result
        }
        _ => panic!("unexpected local response"),
    };
    assert!(matches!(
        run_result.invocation,
        crate::workflow_code::WorkflowCodeRunInvocation::Started { .. }
    ));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_create_without_persisting() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping invalid workflow-code artifact create test because node is unavailable"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-invalid-create-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let invalid_source = r#"
workflow.define({ alias: "invalid_artifact" })
workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
"#;
    let name = "invalid-create";

    let error = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: invalid_source.to_string(),
            },
        ))
        .expect_err("invalid workflow-code artifact should not create");

    assert!(format!("{error}").contains("missing_endpoint"));
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(!artifacts.iter().any(|artifact| artifact.name == name));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_update_without_overwriting() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping invalid workflow-code artifact update test because node is unavailable"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-invalid-update-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = "invalid-update";
    let valid_source = r#"
workflow.define({ alias: "valid_artifact" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;
    let invalid_source = valid_source.replace(
        r#"workflow.endpoint(worker, { handle: "entry", alias: "entry" })"#,
        "",
    );

    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: valid_source.to_string(),
            },
        ))
        .expect("valid workflow-code artifact should create");
    let error = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeArtifact(
            crate::local::UpdateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: invalid_source,
            },
        ))
        .expect_err("invalid workflow-code artifact update should fail");

    assert!(format!("{error}").contains("missing_endpoint"));
    let loaded = harness
        .dispatch(LocalDaemonRequest::GetWorkflowCodeArtifact(
            crate::local::GetWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
            },
        ))
        .expect("workflow-code artifact should load");
    match loaded {
        LocalDaemonResponse::WorkflowCodeArtifact { artifact } => {
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("valid_artifact")
            );
            assert_eq!(artifact.source, valid_source);
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_import() {
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-invalid-import-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let definition = crate::workflow_code::WorkflowCodeDefinition {
        schema_version: crate::workflow_code::WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: crate::workflow_code::WorkflowCodeWorkflow {
            alias: Some("invalid_import".to_string()),
            prompt: None,
            flush_agent_context_before_run: None,
            max_concurrent: None,
            run_output_schema: None,
        },
        schemas: Vec::new(),
        nodes: vec![crate::workflow_code::WorkflowCodeNodeDefinition {
            handle: "worker".to_string(),
            agent: crate::workflow_code::WorkflowCodeAgentBinding::Create(
                crate::workflow_code::WorkflowCodeAgentCreate {
                    alias: Some("worker".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                },
            ),
            public_label: None,
            instructions: None,
            can_complete_workflow_run: Some(true),
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: None,
            max_turns: None,
            extensions: Vec::new(),
            canvas: None,
        }],
        edges: Vec::new(),
        endpoints: Vec::new(),
        queues: Vec::new(),
        schedules: Vec::new(),
    };
    let source = "workflow.define({ alias: \"invalid_import\" })";
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "invalid-import".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: source.to_string(),
        source_sha256: workflow_code_test_sha256_hex(source.as_bytes()),
        source_bytes: source.len() as u64,
        definition_sha256: crate::workflow_code::workflow_code_definition_sha256_hex(&definition),
        validation: definition
            .validate_with_limits(&crate::config::WorkflowCodeLimitsConfig::default()),
        definition,
        exported_at_ms: crate::session::unix_epoch_ms(),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodeArtifact(
            crate::local::ImportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                package,
                name: None,
                overwrite: false,
                node_path: "node".to_string(),
            },
        ))
        .expect_err("invalid workflow-code package should not import");

    assert!(format!("{error}").contains("missing_endpoint"));
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(!artifacts
                .iter()
                .any(|artifact| artifact.name == "invalid-import"));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_workflow_code_artifact_import_with_definition_hash_mismatch() {
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-hash-mismatch-import-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-mismatch"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let definition = crate::workflow_code::WorkflowCodeDefinition {
        schema_version: crate::workflow_code::WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: crate::workflow_code::WorkflowCodeWorkflow {
            alias: Some("mismatch_import".to_string()),
            prompt: None,
            flush_agent_context_before_run: None,
            max_concurrent: None,
            run_output_schema: None,
        },
        schemas: Vec::new(),
        nodes: vec![crate::workflow_code::WorkflowCodeNodeDefinition {
            handle: "worker".to_string(),
            agent: crate::workflow_code::WorkflowCodeAgentBinding::Create(
                crate::workflow_code::WorkflowCodeAgentCreate {
                    alias: Some("worker".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                },
            ),
            public_label: None,
            instructions: None,
            can_complete_workflow_run: Some(true),
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: None,
            max_turns: None,
            extensions: Vec::new(),
            canvas: None,
        }],
        edges: Vec::new(),
        endpoints: vec![crate::workflow_code::WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "worker".to_string(),
            alias: Some("entry".to_string()),
            canvas: None,
        }],
        queues: Vec::new(),
        schedules: Vec::new(),
    };
    let source = "workflow.define({ alias: \"mismatch_import\" })";
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "hash-mismatch-import".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: source.to_string(),
        source_sha256: workflow_code_test_sha256_hex(source.as_bytes()),
        source_bytes: source.len() as u64,
        definition_sha256: "not-the-definition-hash".to_string(),
        validation: definition
            .validate_with_limits(&crate::config::WorkflowCodeLimitsConfig::default()),
        definition,
        exported_at_ms: crate::session::unix_epoch_ms(),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodeArtifact(
            crate::local::ImportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                package,
                name: None,
                overwrite: false,
                node_path: "node".to_string(),
            },
        ))
        .expect_err("definition hash mismatch should not import");

    assert!(format!("{error}").contains("definition sha256 mismatch"));
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(!artifacts
                .iter()
                .any(|artifact| artifact.name == "hash-mismatch-import"));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_creates_typescript_workflow_code_artifact() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code TypeScript artifact test because node is not available");
        return;
    };
    if !node_supports_workflow_code_typescript(&node_path) {
        eprintln!(
            "skipping workflow-code TypeScript artifact test because Node.js cannot strip TypeScript"
        );
        return;
    }

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-workflow-code-ts", "worktree-workflow-code-ts"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = format!("ts-flow-{}", crate::session::unix_epoch_ms());
    let source = r#"
type ProviderName = "dev-stub";
const provider: ProviderName = "dev-stub";
const final = workflow.schema({
  handle: "final",
  schema: {
    type: "object",
    required: ["answer"],
    properties: { answer: { type: "string" } },
    additionalProperties: false
  }
})
workflow.define({ alias: "typescript_artifact_flow", runOutputSchema: final })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "ts-worker", provider, model: "default" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::TypeScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("TypeScript workflow-code artifact should create");

    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert_eq!(artifact.metadata.name, name);
            assert_eq!(
                artifact.metadata.language,
                crate::workflow_code::WorkflowCodeLanguage::TypeScript
            );
            assert!(artifact.metadata.validation.ok);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("typescript_artifact_flow")
            );
            assert_eq!(
                artifact.definition.workflow.run_output_schema.as_deref(),
                Some("final")
            );
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_applies_inline_typescript_workflow_code() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping inline workflow-code TypeScript test because node is not available");
        return;
    };
    if !node_supports_workflow_code_typescript(&node_path) {
        eprintln!(
            "skipping inline workflow-code TypeScript test because Node.js cannot strip TypeScript"
        );
        return;
    }

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-inline-workflow-code-ts",
                "worktree-inline-workflow-code-ts",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
type ProviderName = "dev-stub";
const provider: ProviderName = "dev-stub";
workflow.define({ alias: "inline_typescript_flow" });
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "inline-ts-worker", provider, model: "default" }),
  canCompleteWorkflowRun: true
});
workflow.endpoint(worker, { handle: "entry", alias: "entry" });
"#;

    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
            crate::local::ApplyWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: Some(crate::workflow_code::WorkflowCodeLanguage::TypeScript),
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("inline TypeScript workflow-code should apply");

    let LocalDaemonResponse::WorkflowCodeApplied { result, session } = applied else {
        panic!("unexpected local response");
    };
    assert!(result.compile.validation.ok);
    assert_eq!(
        result.compile.definition.workflow.alias.as_deref(),
        Some("inline_typescript_flow")
    );
    assert!(session
        .workflows()
        .iter()
        .any(|workflow| workflow.id() == result.apply.workflow_id));
}

#[test]
fn local_request_api_exports_agent_app_publication_package() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-agent-app", "worktree-agent-app"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("shopper".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("shopping".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("add".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                queue_ref: Some("default".to_string()),
                alias: Some("shopping-app".to_string()),
                kind: Some("ingress".to_string()),
                route: Some("/add/*".to_string()),
                methods: vec!["GET".to_string()],
                transport: Some(serde_json::json!({ "kind": "human_http" })),
                parser: Some(serde_json::json!({
                    "kind": "regex",
                    "source": "path",
                    "pattern": "^/add/(?<prompt>.+)$"
                })),
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("workflow publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let assets_root = std::env::temp_dir().join(format!(
        "arroba-agent-app-assets-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(assets_root.join("assets")).expect("asset directory should be created");
    std::fs::write(
        assets_root.join("index.html"),
        "<!doctype html><main>shop</main>",
    )
    .expect("index asset should write");
    std::fs::write(assets_root.join("assets/catalog.json"), "{\"items\":[]}")
        .expect("nested asset should write");

    let exported = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: session.id().to_string(),
                publication_ref: publication.id().to_string(),
                kernel_url: Some("ws://127.0.0.1:43118".to_string()),
                agent_app: Some(serde_json::json!({
                    "enabled": true,
                    "assets": {
                        "public_dir": "app",
                        "index": "index.html"
                    },
                    "routes": [{
                        "path": "/add/*",
                        "hook_id": format!("{}-hook", publication.id()),
                        "prompt_source": "path_tail",
                        "response": "streaming_shell",
                        "required_role": "public",
                        "manipulation": {
                            "level": "state_and_overlay",
                            "scope": "session",
                            "allowed_actions": ["cart.search", "cart.add"]
                        }
                    }],
                    "replicas": {
                        "count": 1,
                        "per_caller_ordering": true
                    },
                    "persistent_patch": {
                        "enabled": false
                    }
                })),
                agent_app_assets_dir: Some(assets_root.to_string_lossy().to_string()),
            },
        ))
        .expect("agent app publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_version,
            package_files,
            ..
        } => {
            assert_eq!(package_version, 2);
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let publication_json = package_json_file(&exported, "publication.json");
    assert_eq!(publication_json["package_version"], serde_json::json!(2));
    assert_eq!(publication_json["kind"], serde_json::json!("ingress"));
    assert_eq!(
        publication_json["agent_app"]["routes"][0]["path"],
        serde_json::json!("/add/*")
    );
    assert_eq!(
        package_text_file(&exported, "app/index.html"),
        "<!doctype html><main>shop</main>"
    );
    assert_eq!(
        package_text_file(&exported, "app/assets/catalog.json"),
        "{\"items\":[]}"
    );
    std::fs::remove_dir_all(assets_root).expect("asset directory should clean up");
}

#[test]
fn local_request_api_validates_publication_transport_options() {
    let harness = LocalRouterTestHarness::new();
    let graph = create_publication_test_graph(&harness, "publication-validation");

    let api_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                alias: Some("api-default-route".to_string()),
                kind: Some("ingress".to_string()),
                route: None,
                methods: vec!["POST".to_string()],
                transport: Some(serde_json::json!({ "kind": "api_sse_json" })),
                parser: Some(serde_json::json!({ "kind": "json" })),
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: Some(30_000),
                poll_ms: Some(250),
            },
        ))
        .expect("api publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };

    let exported = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: api_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("api publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let publication_json = package_json_file(&exported, "publication.json");
    assert_eq!(publication_json["kind"], serde_json::json!("ingress"));
    assert_eq!(
        publication_json["hooks"][0]["route"],
        serde_json::json!("/invoke")
    );
    assert_eq!(
        publication_json["hooks"][0]["queue_ref"],
        serde_json::json!("default")
    );
    assert_eq!(
        publication_json["hooks"][0]["methods"],
        serde_json::json!(["POST"])
    );
    assert_eq!(
        publication_json["hooks"][0]["parser"],
        serde_json::json!({ "kind": "json" })
    );
    assert_eq!(
        publication_json["hooks"][0]["mode"],
        serde_json::json!("async")
    );
    let mcp_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                alias: Some("mcp-defaults".to_string()),
                kind: Some("ingress".to_string()),
                route: None,
                methods: Vec::new(),
                transport: Some(serde_json::json!({ "kind": "mcp" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("mcp publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let exported_mcp = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: mcp_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("mcp publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let mcp_json = package_json_file(&exported_mcp, "publication.json");
    assert_eq!(mcp_json["hooks"][0]["route"], serde_json::json!("/mcp"));
    assert_eq!(mcp_json["hooks"][0]["methods"], serde_json::json!(["POST"]));
    assert_eq!(mcp_json["hooks"][0]["mode"], serde_json::json!("sync"));
    assert!(mcp_json["hooks"][0].get("parser").is_none());

    let schedule_without_watchdog = harness.dispatch(
        LocalDaemonRequest::CreateWorkflowPublication(CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("schedule-without-watchdog".to_string()),
            kind: Some("schedule_only".to_string()),
            route: None,
            methods: Vec::new(),
            transport: None,
            parser: None,
            input_schema: None,
            trace_exposure: None,
            mode: None,
            sync_timeout_ms: None,
            poll_ms: None,
        }),
    );
    assert!(schedule_without_watchdog
        .expect_err("schedule_only publication without enabled schedule should fail")
        .to_string()
        .contains("require an enabled schedule"));

    let conflicting_kind_and_transport = harness.dispatch(
        LocalDaemonRequest::CreateWorkflowPublication(CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("conflicting-publication-kind".to_string()),
            kind: Some("ingress".to_string()),
            route: None,
            methods: Vec::new(),
            transport: Some(serde_json::json!({ "kind": "schedule_only" })),
            parser: None,
            input_schema: None,
            trace_exposure: None,
            mode: None,
            sync_timeout_ms: None,
            poll_ms: None,
        }),
    );
    assert!(conflicting_kind_and_transport
        .expect_err("ingress publication with schedule_only transport should fail")
        .to_string()
        .contains("ingress publications must use an ingress transport"));

    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowSchedule(
            CreateWorkflowScheduleRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                trigger: crate::session::WorkflowScheduleTrigger::interval(300),
                invocation_prompt: "scheduled prompt".to_string(),
                overlap_policy: crate::session::WorkflowScheduleOverlapPolicy::Skip,
                max_runs_configured: false,
                max_runs: None,
            },
        ))
        .expect("workflow schedule should be created");
    let schedule_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                alias: Some("schedule-only".to_string()),
                kind: Some("schedule_only".to_string()),
                route: None,
                methods: Vec::new(),
                transport: None,
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("schedule_only publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let exported_schedule = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: schedule_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("schedule_only publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let schedule_json = package_json_file(&exported_schedule, "publication.json");
    assert_eq!(schedule_publication.kind(), "schedule_only");
    assert_eq!(schedule_json["kind"], serde_json::json!("schedule_only"));
    assert_eq!(
        schedule_json["hooks"][0]["transport"],
        serde_json::json!("schedule_only")
    );
    assert!(schedule_json["hooks"][0].get("route").is_none());
    assert!(schedule_json["hooks"][0].get("methods").is_none());
    assert!(schedule_json["hooks"][0].get("parser").is_none());
    assert!(schedule_json["hooks"][0].get("mode").is_none());

    let api_sync = harness.dispatch(LocalDaemonRequest::CreateWorkflowPublication(
        CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("api-sync".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/api".to_string()),
            methods: vec!["POST".to_string()],
            transport: Some(serde_json::json!({ "kind": "api_sse_json" })),
            parser: Some(serde_json::json!({ "kind": "json" })),
            input_schema: None,
            trace_exposure: None,
            mode: Some("sync".to_string()),
            sync_timeout_ms: None,
            poll_ms: None,
        },
    ));
    assert!(api_sync
        .expect_err("api_sse_json sync mode should fail")
        .to_string()
        .contains("api_sse_json publications always use async"));

    let mcp_parser = harness.dispatch(LocalDaemonRequest::CreateWorkflowPublication(
        CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("mcp-json".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/mcp".to_string()),
            methods: vec!["POST".to_string()],
            transport: Some(serde_json::json!({ "kind": "mcp" })),
            parser: Some(serde_json::json!({ "kind": "json" })),
            input_schema: None,
            trace_exposure: None,
            mode: Some("sync".to_string()),
            sync_timeout_ms: None,
            poll_ms: None,
        },
    ));
    assert!(mcp_parser
        .expect_err("mcp parser override should fail")
        .to_string()
        .contains("mcp publications read input from MCP tool arguments"));

    let websocket_custom_route = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id,
                workflow_ref: graph.workflow_id,
                endpoint_ref: graph.endpoint_id,
                queue_ref: Some("default".to_string()),
                alias: Some("custom-ws".to_string()),
                kind: Some("ingress".to_string()),
                route: Some("/socket".to_string()),
                methods: Vec::new(),
                transport: Some(serde_json::json!({ "kind": "websocket_json" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("websocket_json custom route should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(websocket_custom_route.route(), Some("/socket"));
}

#[test]
fn workflow_node_add_rejects_metaagents() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-meta-workflow", "worktree-meta-workflow"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let metaagent = harness.spawn_workflow_test_agent(session.id(), "meta");
    let metaagent = harness.with_app_mut(|app| {
        app.agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("test agent should enter meta mode")
    });
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("graph".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: metaagent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("metaagent workflow node should be rejected");
    assert!(
        error
            .to_string()
            .contains("metaagents cannot be added as workflow nodes"),
        "unexpected error: {error}"
    );
}

struct PublicationTestGraph {
    session_id: String,
    workflow_id: String,
    endpoint_id: String,
}

fn create_publication_test_graph(
    harness: &LocalRouterTestHarness,
    label: &str,
) -> PublicationTestGraph {
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(&format!("workspace-{label}"), &format!("worktree-{label}")),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some(format!("agent-{label}")),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some(format!("workflow-{label}")),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some(format!("endpoint-{label}")),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    PublicationTestGraph {
        session_id: session.id().to_string(),
        workflow_id: workflow.id().to_string(),
        endpoint_id: endpoint.id().to_string(),
    }
}

fn package_text_file(files: &[crate::local::WorkflowPublicationPackageFile], path: &str) -> String {
    let file = files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("package file `{path}` should exist"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.content_base64)
        .expect("package file should decode");
    String::from_utf8(bytes).expect("package file should be UTF-8")
}

fn package_json_file(
    files: &[crate::local::WorkflowPublicationPackageFile],
    path: &str,
) -> serde_json::Value {
    serde_json::from_str(&package_text_file(files, path)).expect("package JSON should parse")
}

#[test]
fn local_request_api_manages_workflows_endpoints_and_graph_edits() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed")
    {
        LocalDaemonResponse::WorkflowsListed { workflows } => workflows,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveWorkflow(
            ResolveWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: "review".to_string(),
            },
        ))
        .expect("workflow resolve should succeed")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(resolved.id(), workflow.id());

    let node_a = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let duplicate_node = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("duplicate workflow node should be rejected");
    assert!(matches!(
        duplicate_node,
        DaemonError::WorkflowNodeConflict { .. }
    ));

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                instructions: Some("You are the reviewer.".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node instructions should update")
    {
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, .. } => {
            assert_eq!(node.instructions(), Some("You are the reviewer."));
        }
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer-2".to_string()),
            provider: Some("opencode".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let node_b = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: spawned.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("second workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node_a.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(endpoint.entry_node_id(), node_a.id());

    let aliased_workflow = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            alias: "qa".to_string(),
            expected_workflow_revision: None,
        }))
        .expect("workflow alias should succeed")
    {
        LocalDaemonResponse::WorkflowAliased { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_workflow.alias(), Some("qa"));

    let aliased_endpoint = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflowEndpoint(
            AliasWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                alias: "start".to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint alias should succeed")
    {
        LocalDaemonResponse::WorkflowEndpointAliased { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_endpoint.alias(), Some("start"));

    let edge = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: node_a.id().to_string(),
                to_node_id: node_b.id().to_string(),
                handoff_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
                source_side: Some(crate::session::WorkflowEdgeEndpointSide::Right),
                target_side: Some(crate::session::WorkflowEdgeEndpointSide::Left),
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        edge.source_side(),
        Some(crate::session::WorkflowEdgeEndpointSide::Right)
    );
    assert_eq!(
        edge.target_side(),
        Some(crate::session::WorkflowEdgeEndpointSide::Left)
    );

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
            UpdateWorkflowCanvasLayoutRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                base_layout_revision: None,
                patches: vec![
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_a.id().to_string(),
                        x: 120,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_b.id().to_string(),
                        x: 420,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::EndpointPosition {
                        endpoint_id: endpoint.id().to_string(),
                        x: 180,
                        y: 36,
                    },
                ],
            },
        ))
        .expect("workflow canvas layout should update")
    {
        LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout, workflow, ..
        } => {
            assert_eq!(layout.revision, 1);
            assert_eq!(
                layout.nodes.get(node_a.id()).map(|point| point.x),
                Some(120)
            );
            assert_eq!(
                workflow
                    .canvas_layout()
                    .and_then(|stored| stored.endpoints.get(endpoint.id()))
                    .map(|point| point.y),
                Some(36)
            );
        }
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowEdge(
            RemoveWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                edge_id: edge.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be removed")
    {
        LocalDaemonResponse::WorkflowEdgeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowNode(
            RemoveWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be removed")
    {
        LocalDaemonResponse::WorkflowNodeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_materializes_workflow_publication_as_hidden_runtime_session() {
    let harness = LocalRouterTestHarness::new();
    let source_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("source session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: source_session.id().to_string(),
            alias: Some("published_worker".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("source workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: source_session.id().to_string(),
            alias: Some("publishable".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: source_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: source_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let (workflow, endpoint) = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: source_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated {
            workflow, endpoint, ..
        } => (workflow, endpoint),
        _ => panic!("unexpected local response"),
    };
    let source_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: source_session.id().to_string(),
            },
        ))
        .expect("source state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source_queue = source_state
        .workflow_prompt_queues()
        .iter()
        .find(|queue| queue.workflow_id() == workflow.id() && queue.alias() == "default")
        .expect("source workflow should have a default queue")
        .clone();
    let source_watchdog = crate::session::WorkflowWatchdogDefinition::new(
        "watchdog-1",
        workflow.id(),
        endpoint.id(),
        60,
        "publication watchdog",
        crate::session::WorkflowWatchdogPolicy::Queue,
        Some(1),
    );
    match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: source_session.id().to_string(),
            workspace_id: None,
        }))
        .expect("source session should be deletable before materialization")
    {
        LocalDaemonResponse::SessionDeleted { .. } => {}
        _ => panic!("unexpected local response"),
    };

    let runtime_owner_user_id = "published-runtime-user";
    let materialized = match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::MaterializeWorkflowPublication(
                MaterializeWorkflowPublicationRequest {
                    publication_id: "publication-1".to_string(),
                    snapshot: WorkflowPublicationSnapshot {
                        schema_version: 1,
                        captured_at_ms: Some(42),
                        source_session: Some(WorkflowPublicationSourceSessionSnapshot {
                            id: Some(source_session.id().to_string()),
                            alias: source_session.alias().map(str::to_string),
                            workspace_id: source_session.workspace_id().to_string(),
                            worktree_id: source_session.worktree_id().to_string(),
                        }),
                        workflow: workflow.clone(),
                        endpoint: Some(endpoint.clone()),
                        queues: vec![source_queue],
                        schedules: vec![source_watchdog.clone()],
                        agents: vec![source_agent.clone()],
                    },
                },
            ),
        )
        .expect("publication should materialize")
    {
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            session,
            agent_id_map,
            ..
        } => {
            assert_eq!(
                agent_id_map.get(source_agent.id()).map(String::as_str),
                session
                    .workflows()
                    .first()
                    .and_then(|workflow| workflow.nodes().first())
                    .map(|node| node.agent_id())
            );
            session
        }
        _ => panic!("unexpected local response"),
    };
    assert!(materialized.is_hidden());
    assert_eq!(materialized.owner_user_id(), runtime_owner_user_id);
    assert!(materialized.has_member(runtime_owner_user_id));
    assert_ne!(materialized.id(), source_session.id());
    assert_eq!(materialized.workflows().len(), 1);
    assert_eq!(materialized.workflow_publications().len(), 1);
    assert_eq!(
        materialized.workflow_publications()[0].id(),
        "publication-1"
    );
    assert_eq!(
        materialized.workflow_publications()[0].endpoint_id(),
        endpoint.id()
    );
    assert_eq!(materialized.workflow_watchdogs().len(), 1);
    assert_eq!(
        materialized.workflow_watchdogs()[0].invocation_prompt(),
        source_watchdog.invocation_prompt()
    );
    assert_eq!(
        materialized.workflow_watchdogs()[0].endpoint_id(),
        endpoint.id()
    );
    assert_eq!(materialized.agents().len(), 1);
    assert_ne!(materialized.agents()[0].id(), source_agent.id());
    assert_eq!(
        materialized.agents()[0].owner_user_id(),
        runtime_owner_user_id
    );
    let materialized_workflow = materialized
        .workflows()
        .first()
        .expect("materialized workflow should exist");
    assert_eq!(
        materialized_workflow.nodes()[0].owner_user_id(),
        runtime_owner_user_id
    );
    assert_eq!(
        materialized_workflow.nodes()[0].created_by_user_id(),
        runtime_owner_user_id
    );
    assert_eq!(
        materialized_workflow.endpoints()[0].owner_user_id(),
        runtime_owner_user_id
    );

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list sessions should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());

    let hidden_state = match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: materialized.id().to_string(),
            }),
        )
        .expect("hidden runtime session should still load by id")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert!(hidden_state.is_hidden());

    match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(
                RegisterWorkflowPublicationEndpointRequest {
                    session_id: materialized.id().to_string(),
                    publication_ref: "publication-1".to_string(),
                    local_url: "http://127.0.0.1:3000/".to_string(),
                    runtime_session_id: Some(materialized.id().to_string()),
                    ttl_ms: None,
                },
            ),
        )
        .expect("materialized publication endpoint should register")
    {
        LocalDaemonResponse::WorkflowPublicationEndpointRegistered {
            publication,
            open_url,
            ..
        } => {
            assert_eq!(publication.id(), "publication-1");
            assert_eq!(publication.open_url(), Some(open_url.as_str()));
            assert_eq!(publication.viewer_url(), Some(open_url.as_str()));
        }
        _ => panic!("unexpected local response"),
    }
}
