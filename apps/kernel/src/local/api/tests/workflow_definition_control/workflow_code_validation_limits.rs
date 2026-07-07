use super::*;

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
