use super::*;
use crate::local::{
    CreateWorkflowPublicationRequest, ExportWorkflowPublicationPackageRequest,
    RegisterWorkflowPublicationEndpointRequest,
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
    required: ["value"],
    properties: {
      value: { type: "number" }
    },
    additionalProperties: false
  }
})
workflow.define({
  alias: "scripted_run_flow",
  maxConcurrent: 4,
  runOutputSchema: finalOutput,
  intermediateOutputSchema: progressOutput
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
                endpoint: Some("entry".to_string()),
                queue_ref: None,
                prompt: "Explain the smallest useful scripted workflow run.".to_string(),
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
    assert_eq!(
        workflow.intermediate_output_schema_ref(),
        result
            .apply
            .apply
            .schema_refs
            .get("progress")
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
                    .get("default")
                    .map(String::as_str)
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
        Some("Explain the smallest useful scripted workflow run.")
    );
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert!(run_session
        .workflow_runs()
        .iter()
        .any(|run| run.id() == workflow_run.id()));

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

    let intermediate = harness
        .dispatch_runtime_tool(
            &runtime_mcp_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token.clone(),
                "workflow_output_json": "{\"value\":1841}"
            }),
        )
        .expect("intermediate workflow output should validate");
    assert!(intermediate.ok, "{:?}", intermediate.payload);
    assert_eq!(intermediate.payload["valid"], true);

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
    assert_eq!(completed_run.intermediate_outputs().len(), 1);
    let intermediate = &completed_run.intermediate_outputs()[0];
    assert!(intermediate.valid());
    assert_eq!(intermediate.warning(), None);
    let intermediate_json: serde_json::Value =
        serde_json::from_str(intermediate.output().message())
            .expect("intermediate output message should be JSON");
    assert_eq!(intermediate_json["value"], 1841);
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
        .watchdog_ids
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
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan."
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
        workflow: crate::workflow_code::WorkflowCodeWorkflow {
            alias: Some("invalid_import".to_string()),
            flush_agent_context_before_run: None,
            max_concurrent: None,
            run_output_schema: None,
            intermediate_output_schema: None,
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
        watchdogs: Vec::new(),
    };
    let source = "workflow.define({ alias: \"invalid_import\" })";
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "invalid-import".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: source.to_string(),
        source_sha256: workflow_code_test_sha256_hex(source.as_bytes()),
        source_bytes: source.len() as u64,
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
    let metaagent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("meta".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: true,
        }))
        .expect("metaagent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
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
                        watchdogs: vec![source_watchdog.clone()],
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
        }
        _ => panic!("unexpected local response"),
    }
}
