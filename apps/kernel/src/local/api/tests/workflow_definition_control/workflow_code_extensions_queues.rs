use super::*;

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
