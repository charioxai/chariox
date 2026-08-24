use super::*;

#[test]
fn local_request_api_invokes_lists_gets_and_cancels_workflow_runs() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-invokes-lists-gets-and-cancels-workflow-runs",
        || local_request_api_invokes_lists_gets_and_cancels_workflow_runs_inner("dev-stub"),
    );
}

#[test]
fn local_request_api_settles_structured_workflow_prompt_cancellation() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-settles-structured-workflow-prompt-cancellation",
        || local_request_api_invokes_lists_gets_and_cancels_workflow_runs_inner("slow-structured"),
    );
}

#[test]
fn local_request_api_enqueues_into_a_disabled_workflow_queue_without_launching() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-enqueues-into-a-disabled-workflow-queue-without-launching",
        local_request_api_enqueues_into_a_disabled_workflow_queue_without_launching_inner,
    );
}

#[test]
fn local_request_api_queues_concurrent_invocations_for_one_endpoint() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-queues-concurrent-invocations-for-one-endpoint",
        local_request_api_queues_concurrent_invocations_for_one_endpoint_inner,
    );
}

#[test]
fn local_request_api_serializes_two_workflows_sharing_an_agent() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-serializes-two-workflows-sharing-an-agent",
        local_request_api_serializes_two_workflows_sharing_an_agent_inner,
    );
}

#[test]
fn local_request_api_runs_independent_workflows_concurrently() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-runs-independent-workflows-concurrently",
        local_request_api_runs_independent_workflows_concurrently_inner,
    );
}

#[test]
fn stopping_workflow_dispatches_next_queued_workflow_prompt() {
    run_workflow_run_lifecycle_large_stack_test(
        "stopping-workflow-dispatches-next-queued-workflow-prompt",
        stopping_workflow_dispatches_next_queued_workflow_prompt_inner,
    );
}

#[test]
fn local_request_api_two_workflow_multi_node_collision_drill() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-two-workflow-multi-node-collision-drill",
        local_request_api_two_workflow_multi_node_collision_drill_inner,
    );
}

fn run_workflow_run_lifecycle_large_stack_test(name: &str, test: impl FnOnce() + Send + 'static) {
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(test)
        .expect("workflow run lifecycle large-stack test thread should spawn");
    if let Err(payload) = handle.join() {
        std::panic::resume_unwind(payload);
    }
}

fn local_request_api_invokes_lists_gets_and_cancels_workflow_runs_inner(provider: &str) {
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
            account_profile: None,
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
    match harness
        .dispatch(LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
            SetWorkflowNodeCanCompleteRunRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node.id().to_string(),
                can_complete_workflow_run: true,
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node completion setting should update")
    {
        LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: provider.to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect("provider run should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());
    let provider_ready_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let running = harness.with_app(|app| {
            app.providers()
                .get_run_for_agent(session.id(), agent.id())
                .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Running)
        });
        if running {
            break;
        }
        assert!(
            Instant::now() < provider_ready_deadline,
            "structured workflow test provider should become running"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("workflow run invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    let workflow_run = wait_for_workflow_run_status(
        &harness,
        session.id(),
        workflow_run.id(),
        &[WorkflowRunStatus::Running, WorkflowRunStatus::Failed],
    );

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
                cursor: None,
                limit: None,
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } => workflow_runs,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), workflow_run.id());

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
    assert_eq!(resolved.id(), workflow_run.id());
    assert!(matches!(
        resolved.status(),
        WorkflowRunStatus::Running | WorkflowRunStatus::Failed
    ));

    if resolved.status() == WorkflowRunStatus::Running {
        harness.complete_workflow_test_prompt(session.id(), "workflow-backed prompt");
    }

    let completed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("settled workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert!(matches!(
        completed.status(),
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed
    ));

    let second_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff again".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("second workflow run invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let cancelled = match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: second_run.id().to_string(),
            },
        ))
        .expect("workflow run should cancel")
    {
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(cancelled.id(), second_run.id());
    assert_eq!(format!("{:?}", cancelled.status()), "Stopped");

    let deadline = Instant::now() + Duration::from_secs(3);
    if provider == "slow-structured" {
        loop {
            let prompt_settled = harness.with_app(|app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("workflow session should resolve")
                    .active_prompt_for_agent(agent.id())
                    .is_none()
            });
            if prompt_settled {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "stopping a structured workflow node turn should settle its active prompt"
            );
            thread::sleep(Duration::from_millis(10));
        }
    } else {
        let provider_run_id = harness.with_app(|app| {
            app.providers()
                .get_run_for_agent(session.id(), agent.id())
                .expect("workflow provider run should resolve")
                .id()
                .to_string()
        });
        loop {
            let interrupted = harness.with_app(|app| {
                app.terminal().input_records().iter().any(|record| {
                    record.provider_run_id == provider_run_id && record.bytes == b"\x03"
                })
            });
            if interrupted {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "stopping a PTY workflow node turn should send Ctrl-C"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    let (cancelled_again, refreshed_session) = match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: second_run.id().to_string(),
            },
        ))
        .expect("stopping a workflow run twice should succeed")
    {
        LocalDaemonResponse::WorkflowRunCancelled {
            workflow_run,
            session,
        } => (workflow_run, session),
        _ => panic!("unexpected local response"),
    };
    assert_eq!(cancelled_again.status(), WorkflowRunStatus::Stopped);
    if provider == "slow-structured" {
        assert!(
            refreshed_session
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "refresh after an idempotent stop should keep the structured prompt settled"
        );
    }
}

fn local_request_api_enqueues_into_a_disabled_workflow_queue_without_launching_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-disabled-queue", "worktree-disabled-queue"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "disabled-queue-agent");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("disabled-queue-workflow".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowPromptQueue(
            UpdateWorkflowPromptQueueRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
                queue_ref: "default".to_string(),
                alias: None,
                priority: None,
                enabled: Some(false),
            },
        ))
        .expect("default workflow queue should be disabled")
    {
        LocalDaemonResponse::WorkflowPromptQueueUpdated { queue, .. } => {
            assert!(!queue.enabled());
        }
        _ => panic!("unexpected local response"),
    }

    let queued_prompt = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("hold this prompt".to_string()),
                queue_ref: Some("default".to_string()),
                publication_invocation: None,
            },
        ))
        .expect("disabled workflow queue invocation should enqueue")
    {
        LocalDaemonResponse::WorkflowPromptEnqueued { queued_prompt, .. } => queued_prompt,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(queued_prompt.prompt(), Some("hold this prompt"));
    assert_eq!(
        queued_prompt.status(),
        crate::session::WorkflowQueuedPromptStatus::Queued
    );

    let workflow_runs = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
                cursor: None,
                limit: None,
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } => workflow_runs,
        _ => panic!("unexpected local response"),
    };
    assert!(workflow_runs.is_empty());

    let queued_prompts = match harness
        .dispatch(LocalDaemonRequest::ListQueuedWorkflowPrompts(
            ListQueuedWorkflowPromptsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("queued workflow prompts should list")
    {
        LocalDaemonResponse::QueuedWorkflowPromptsListed { queued_prompts } => queued_prompts,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(queued_prompts, vec![queued_prompt]);
}

fn stopping_workflow_dispatches_next_queued_workflow_prompt_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-queued-after-stop", "worktree-queued-after-stop"),
        ))
        .expect("session should create")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "queued-after-stop-agent");
    match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "slow-structured".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect("provider should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("queued-after-stop".to_string()),
        }))
        .expect("workflow should create")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should create")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let first_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("first active workflow".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("first workflow should start")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let parked_prompt = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("second queued workflow".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("second workflow should park in the endpoint queue")
    {
        // The endpoint's primary runtime instance is busy with the first run and
        // max_instances is 1, so the second concurrent invocation must park in
        // the workflow-owned queue instead of creating a run immediately.
        LocalDaemonResponse::WorkflowPromptEnqueued { queued_prompt, .. } => queued_prompt,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(parked_prompt.workflow_id(), workflow.id());
    assert_eq!(parked_prompt.endpoint_id(), endpoint.id());
    assert_eq!(
        parked_prompt.queue_id(),
        &format!("{0}:default", workflow.id())
    );
    assert_eq!(
        parked_prompt.status(),
        crate::session::WorkflowQueuedPromptStatus::Queued
    );
    assert_eq!(parked_prompt.workflow_run_id(), None);
    let queued_before_stop = harness.with_app(|app| {
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available");
        (
            session
                .workflow_queued_prompts()
                .iter()
                .any(|prompt| prompt.id() == parked_prompt.id()),
            session
                .queued_prompts_for_agent(agent.id())
                .map(|queue| queue.is_empty())
                .unwrap_or(true),
        )
    });
    assert!(
        queued_before_stop.0,
        "second invocation should wait in the workflow-owned queue while the shared agent runs the first workflow"
    );
    assert!(
        queued_before_stop.1,
        "shared agent must not receive the second workflow delivery before its instance frees"
    );
    match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: first_run.id().to_string(),
            },
        ))
        .expect("first workflow should stop and release the shared agent")
    {
        LocalDaemonResponse::WorkflowRunCancelled { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        harness.pump_transport_runtime();
        let delivered = harness.with_app(|app| {
            app.sessions()
                .get_session(session.id())
                .expect("session should remain available")
                .active_prompt_for_agent(agent.id())
                .is_some_and(|prompt| {
                    prompt.prompt().contains("second queued workflow")
                        && prompt.durable_delivery_phase()
                            == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                })
        });
        if delivered {
            break;
        }
        if Instant::now() >= deadline {
            let diagnostic = harness.with_app(|app| {
                let session = app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should remain available");
                let active = session.active_prompt_for_agent(agent.id()).cloned();
                let provider = app
                    .providers()
                    .get_run_for_agent(session.id(), agent.id());
                format!(
                    "active={active:?}, provider={provider:?}, workflow_runs={:?}, workflow_queue={:?}",
                    session.workflow_runs(),
                    session.workflow_queued_prompts(),
                )
            });
            panic!(
                "stopping the active workflow must deliver the promoted queued workflow prompt: {diagnostic}"
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    let advanced = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
    });
    assert!(advanced.workflow_queued_prompts().is_empty());
    assert_eq!(advanced.workflow_runs().len(), 1);
    assert_eq!(
        advanced.workflow_runs()[0].status(),
        WorkflowRunStatus::Running
    );
    // Workflow and run identity survive queueing: the promoted run belongs to
    // the same workflow/endpoint and carries the parked prompt's identity.
    assert_eq!(advanced.workflow_runs()[0].workflow_id(), workflow.id());
    assert_eq!(advanced.workflow_runs()[0].endpoint_id(), endpoint.id());
    assert_eq!(
        advanced.workflow_runs()[0].queue_item_id(),
        Some(parked_prompt.id())
    );
    assert_eq!(
        advanced.workflow_runs()[0].invocation_prompt(),
        Some("second queued workflow")
    );
    let (first_page, next_cursor) = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            crate::local::ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: None,
                cursor: None,
                limit: Some(1),
            },
        ))
        .expect("workflow run history should remain queryable")
    {
        LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs,
            next_cursor,
        } => (workflow_runs, next_cursor),
        _ => panic!("unexpected workflow run history response"),
    };
    assert_eq!(first_page.len(), 1);
    let second_page = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            crate::local::ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: None,
                cursor: next_cursor,
                limit: Some(1),
            },
        ))
        .expect("next workflow run history page should remain queryable")
    {
        LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs,
            next_cursor,
        } => {
            assert!(next_cursor.is_none());
            workflow_runs
        }
        _ => panic!("unexpected workflow run history response"),
    };
    assert_eq!(second_page.len(), 1);
    assert_ne!(first_page[0].id(), second_page[0].id());
}

fn local_request_api_queues_concurrent_invocations_for_one_endpoint_inner() {
    const INVOCATION_COUNT: usize = 12;

    let harness = std::sync::Arc::new(LocalRouterTestHarness::new());
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-concurrent-launch", "worktree-concurrent-launch"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "concurrent-launch-agent");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("concurrent-launch-workflow".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    harness.launch_workflow_test_provider(session.id(), agent.id());

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(INVOCATION_COUNT));
    let mut handles = Vec::with_capacity(INVOCATION_COUNT);
    for index in 0..INVOCATION_COUNT {
        let harness = std::sync::Arc::clone(&harness);
        let barrier = std::sync::Arc::clone(&barrier);
        let session_id = session.id().to_string();
        let workflow_id = workflow.id().to_string();
        let endpoint_id = endpoint.id().to_string();
        handles.push(
            std::thread::Builder::new()
                .name(format!("concurrent-workflow-launch-{index}"))
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    barrier.wait();
                    harness.dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
                        InvokeWorkflowEndpointRequest {
                            session_id,
                            workflow_ref: workflow_id,
                            endpoint_ref: endpoint_id,
                            prompt: Some(format!("concurrent invocation {index}")),
                            queue_ref: None,
                            publication_invocation: None,
                        },
                    ))
                })
                .expect("concurrent workflow launch thread should spawn"),
        );
    }

    let responses = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
        })
        .collect::<Vec<_>>();
    let errors = responses
        .iter()
        .filter_map(|response| response.as_ref().err().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "concurrent invokes failed: {errors:?}");
    let started = responses
        .iter()
        .filter(|response| matches!(response, Ok(LocalDaemonResponse::WorkflowRunInvoked { .. })))
        .count();
    let enqueued = responses
        .iter()
        .filter(|response| {
            matches!(
                response,
                Ok(LocalDaemonResponse::WorkflowPromptEnqueued { .. })
            )
        })
        .count();
    assert_eq!(started, 1);
    assert_eq!(enqueued, INVOCATION_COUNT - 1);

    let workflow_runs = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
                cursor: None,
                limit: None,
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } => workflow_runs,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(workflow_runs.len(), 1);
    let queued_prompts = match harness
        .dispatch(LocalDaemonRequest::ListQueuedWorkflowPrompts(
            ListQueuedWorkflowPromptsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("queued workflow prompts should list")
    {
        LocalDaemonResponse::QueuedWorkflowPromptsListed { queued_prompts } => queued_prompts,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(queued_prompts.len(), INVOCATION_COUNT - 1);
    assert!(queued_prompts.iter().all(|prompt| {
        prompt.workflow_id() == workflow.id()
            && prompt.endpoint_id() == endpoint.id()
            && prompt.workflow_run_id().is_none()
    }));
    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
    });
    let active = session_state
        .active_prompt_for_agent(agent.id())
        .expect("one workflow prompt should be active");
    assert!(session_state
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queue| queue.is_empty()));
    assert_eq!(active.workflow_run_id(), Some(workflow_runs[0].id()));
    assert_eq!(
        active.workflow_node_run_id(),
        Some(workflow_runs[0].node_runs()[0].id()),
    );
}

// Live two-workflow collision drill: both multi-node workflows share the entry
// agent, collide in time, and produce compact goal-audit outputs. Proves the
// second workflow-owned delivery queues while the shared agent is busy, starts
// only after it becomes idle, and that workflow/run identity survives a hot
// state round trip (the same serialization the kernel restart restores from).
fn local_request_api_two_workflow_multi_node_collision_drill_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-concurrency-audit-drill",
                "worktree-concurrency-audit-drill",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    // Worktree-concurrency boundary: workflow node turns serialize per worktree
    // through workspace write claims, so every node that must run concurrently
    // needs its own worktree. The shared agent stays on the session worktree —
    // both workflows deliberately collide on it — while the alpha and beta
    // downstream agents get explicit isolated worktrees (the same pattern as
    // `local_request_api_runs_independent_workflows_concurrently`), so the
    // alpha handoff can bind to a provider run while the promoted beta
    // delivery holds the shared agent.
    let shared_agent = harness.spawn_workflow_test_agent(session.id(), "audit-shared-agent");
    let alpha_worktree = std::env::temp_dir()
        .join("chariox-collision-drill-alpha")
        .join(session.id());
    let beta_worktree = std::env::temp_dir()
        .join("chariox-collision-drill-beta")
        .join(session.id());
    std::fs::create_dir_all(&alpha_worktree).expect("alpha drill worktree should exist");
    std::fs::create_dir_all(&beta_worktree).expect("beta drill worktree should exist");
    let alpha_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "audit-alpha-agent",
        Some(&alpha_worktree.to_string_lossy()),
    );
    let beta_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "audit-beta-agent",
        Some(&beta_worktree.to_string_lossy()),
    );
    assert_eq!(
        shared_agent.worktree_id(),
        None,
        "the colliding shared agent must inherit the session worktree"
    );
    assert_ne!(
        alpha_agent.worktree_id(),
        shared_agent.worktree_id(),
        "the alpha downstream agent needs an isolated worktree so its handoff does not serialize behind the beta delivery"
    );
    assert_ne!(
        beta_agent.worktree_id(),
        shared_agent.worktree_id(),
        "the beta downstream agent needs an isolated worktree so its handoff does not serialize behind the alpha chain"
    );
    harness.launch_workflow_test_provider(session.id(), alpha_agent.id());
    harness.launch_workflow_test_provider(session.id(), beta_agent.id());
    eprintln!(
        "drill: session {} with shared agent {}",
        session.id(),
        shared_agent.id()
    );
    match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(shared_agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "slow-structured".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect("shared agent provider should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());

    let build_workflow = |alias: &str, second_agent: &str| {
        let workflow = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some(alias.to_string()),
            }))
            .expect("drill workflow should create")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let entry = harness.add_workflow_test_node(session.id(), workflow.id(), shared_agent.id());
        let downstream = harness.add_workflow_test_node(session.id(), workflow.id(), second_agent);
        harness.add_workflow_test_edge(session.id(), workflow.id(), entry.id(), downstream.id());
        let endpoint = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: entry.id().to_string(),
                    alias: Some("entry".to_string()),
                    expected_workflow_revision: None,
                },
            ))
            .expect("drill endpoint should create")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        (workflow, endpoint)
    };
    let (alpha_workflow, alpha_endpoint) = build_workflow("goal-audit-alpha", alpha_agent.id());
    let (beta_workflow, beta_endpoint) = build_workflow("goal-audit-beta", beta_agent.id());

    // Collide in time: invoke both workflows back to back on the busy shared
    // agent. Each carries a useful, compact goal-audit task.
    let invoke = |workflow_id: &str, endpoint_id: &str, prompt: String| {
        harness
            .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.to_string(),
                    endpoint_ref: endpoint_id.to_string(),
                    prompt: Some(prompt),
                    queue_ref: None,
                    publication_invocation: None,
                },
            ))
            .expect("drill invocation should dispatch")
    };
    let alpha_invocation = invoke(
        alpha_workflow.id(),
        alpha_endpoint.id(),
        "[goal-audit] alpha lane: summarize the shared-agent queue-collision outcome as compact JSON"
            .to_string(),
    );
    let beta_invocation = invoke(
        beta_workflow.id(),
        beta_endpoint.id(),
        "[goal-audit] beta lane: record the queued second delivery and its start ordering as compact JSON"
            .to_string(),
    );
    let alpha_run = match alpha_invocation {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("alpha drill invocation should start immediately"),
    };
    let beta_run_created = match beta_invocation {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("beta drill run should be created for the distinct endpoint pool"),
    };
    eprintln!(
        "drill: collided invocations accepted; alpha run {}, beta run {}",
        alpha_run.id(),
        beta_run_created.id()
    );

    // The shared agent is busy with the alpha entry node: the beta delivery
    // must queue behind it instead of dispatching.
    let parked = harness.with_app(|app| {
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available");
        (
            session_state
                .active_prompt_for_agent(shared_agent.id())
                .and_then(|prompt| prompt.workflow_run_id())
                == Some(alpha_run.id()),
            session_state
                .queued_prompts_for_agent(shared_agent.id())
                .map(|queue| {
                    queue
                        .iter()
                        .any(|prompt| prompt.workflow_run_id() == Some(beta_run_created.id()))
                })
                .unwrap_or(false),
        )
    });
    assert!(
        parked.0,
        "alpha entry node should own the shared agent after the collision"
    );
    assert!(
        parked.1,
        "beta workflow delivery should queue behind the shared agent"
    );

    // Workflow/run identity survives the hot-state round trip that a kernel
    // restart restores from.
    let encoded = serde_json::to_value(
        harness
            .with_app(|app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("session should serialize")
            })
            .clone(),
    )
    .expect("session should serialize");
    let restored: crate::session::RuntimeSession =
        serde_json::from_value(encoded).expect("session hot state should restore");
    let restored_beta_queued = restored
        .queued_prompts_for_agent(shared_agent.id())
        .expect("restored shared agent queue should resolve")
        .iter()
        .find(|prompt| prompt.workflow_run_id() == Some(beta_run_created.id()))
        .cloned();
    assert!(
        restored_beta_queued.is_some(),
        "the queued beta workflow delivery should survive restart restoration"
    );
    let restored_beta_run = restored
        .workflow_runs()
        .iter()
        .find(|run| run.id() == beta_run_created.id())
        .cloned()
        .expect("beta run identity should survive restart restoration");
    assert_eq!(restored_beta_run.workflow_id(), beta_workflow.id());
    assert_eq!(restored_beta_run.endpoint_id(), beta_endpoint.id());
    let restored_alpha_instance = restored
        .workflow_runtime_instances()
        .iter()
        .find(|instance| instance.active_run_id() == Some(alpha_run.id()))
        .cloned()
        .expect("alpha primary instance binding should survive restart restoration");
    assert!(restored_alpha_instance.primary());

    // Finish the alpha entry turn on the shared agent. Completing it frees the
    // shared agent, which immediately promotes the queued beta delivery while
    // the alpha chain continues on the alpha-only agent.
    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        shared_agent.id(),
        "alpha goal-audit entry",
    );
    let alpha_after_entry = wait_for_workflow_run_matching(
        &harness,
        session.id(),
        alpha_run.id(),
        "alpha entry node should complete and route its handoff",
        |run| {
            run.node_runs()
                .first()
                .is_some_and(|node| node.status() == WorkflowNodeRunStatus::Completed)
        },
    );
    let shared_idle_at_ms = alpha_after_entry
        .node_runs()
        .first()
        .and_then(|node| node.completed_at_ms())
        .expect("alpha entry node should record a completion time");

    // Finish the alpha chain on the alpha-only agent while beta owns the
    // shared agent. Focusing the downstream agent delivers the routed handoff.
    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        alpha_agent.id(),
        "alpha goal-audit result",
    );
    let alpha_final = wait_for_workflow_run_matching(
        &harness,
        session.id(),
        alpha_run.id(),
        "alpha drill run should complete",
        |run| run.status() == WorkflowRunStatus::Completed,
    );
    let alpha_message = alpha_final
        .node_runs()
        .last()
        .and_then(|node| node.completion())
        .and_then(|completion| completion.output())
        .expect("alpha terminal node should carry its goal-audit output")
        .message();
    assert!(
        alpha_message.contains("alpha goal-audit result"),
        "alpha output should carry the audit payload, got: {alpha_message}"
    );

    // The beta delivery may only start once the shared agent became idle.
    let beta_started_session = harness.wait_for_session_where(
        session.id(),
        "beta run should start after the shared agent freed",
        |state| {
            state
                .workflow_runs()
                .iter()
                .any(|run| run.id() == beta_run_created.id() && run.started_at_ms().is_some())
        },
    );
    let beta_running = beta_started_session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == beta_run_created.id())
        .cloned()
        .expect("beta run should resolve");
    assert_eq!(beta_running.workflow_id(), beta_workflow.id());
    assert_eq!(beta_running.endpoint_id(), beta_endpoint.id());
    assert_eq!(
        beta_running.invocation_prompt(),
        Some(
            "[goal-audit] beta lane: record the queued second delivery and its start ordering as compact JSON"
        )
    );
    assert!(
        beta_running
            .started_at_ms()
            .expect("beta run should have started")
            >= shared_idle_at_ms,
        "the beta delivery must not start before the shared agent became idle",
    );
    eprintln!(
        "drill: beta started at {} after the shared agent freed at {}",
        beta_running.started_at_ms().unwrap_or(0),
        shared_idle_at_ms,
    );

    // Finish the beta chain.
    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        shared_agent.id(),
        "beta goal-audit entry",
    );
    harness.wait_for_session_where(
        session.id(),
        "beta audit node should receive the routed handoff",
        |state| {
            state
                .active_prompt_for_agent(beta_agent.id())
                .is_some_and(|prompt| {
                    prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                })
        },
    );
    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        beta_agent.id(),
        "beta goal-audit result",
    );
    let beta_final = wait_for_workflow_run_matching(
        &harness,
        session.id(),
        beta_run_created.id(),
        "beta drill run should complete",
        |run| run.status() == WorkflowRunStatus::Completed,
    );
    let beta_message = beta_final
        .node_runs()
        .last()
        .and_then(|node| node.completion())
        .and_then(|completion| completion.output())
        .expect("beta terminal node should carry its goal-audit output")
        .message();
    assert!(
        beta_message.contains("beta goal-audit result"),
        "beta output should carry the audit payload, got: {beta_message}"
    );

    // Exactly one durable run per workflow, with disjoint identities. Terminal
    // runs leave the active session projection and remain queryable through
    // workflow history, which is the product-facing source for completed runs.
    let list_runs = |workflow_id: &str| match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow_id.to_string()),
                cursor: None,
                limit: None,
            },
        ))
        .expect("durable workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } => workflow_runs,
        _ => panic!("unexpected workflow run history response"),
    };
    assert_eq!(list_runs(alpha_workflow.id()).len(), 1);
    assert_eq!(list_runs(beta_workflow.id()).len(), 1);
    assert_ne!(alpha_final.id(), beta_final.id());
    eprintln!(
        "drill: complete; alpha run {}, beta run {}; identities preserved across queueing and restart restore",
        alpha_final.id(),
        beta_final.id()
    );
    std::fs::remove_dir_all(alpha_worktree).expect("alpha drill worktree should clean up");
    std::fs::remove_dir_all(beta_worktree).expect("beta drill worktree should clean up");
}

fn local_request_api_serializes_two_workflows_sharing_an_agent_inner() {
    let harness = std::sync::Arc::new(LocalRouterTestHarness::new());
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-shared-agent-workflows",
                "worktree-shared-agent-workflows",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "shared-workflow-agent");
    match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "slow-structured".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect("provider should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());

    let create_workflow = |alias: &str| {
        let workflow = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some(alias.to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
        let endpoint = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow endpoint should create")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        (workflow, endpoint)
    };
    let (first_workflow, first_endpoint) = create_workflow("shared-agent-first");
    let (second_workflow, second_endpoint) = create_workflow("shared-agent-second");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for (workflow, endpoint, prompt) in [
        (&first_workflow, &first_endpoint, "first shared workflow"),
        (&second_workflow, &second_endpoint, "second shared workflow"),
    ] {
        let harness = std::sync::Arc::clone(&harness);
        let barrier = std::sync::Arc::clone(&barrier);
        let session_id = session.id().to_string();
        let workflow_id = workflow.id().to_string();
        let endpoint_id = endpoint.id().to_string();
        handles.push(
            std::thread::Builder::new()
                .name(format!("invoke-{prompt}"))
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    barrier.wait();
                    harness.dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
                        InvokeWorkflowEndpointRequest {
                            session_id,
                            workflow_ref: workflow_id,
                            endpoint_ref: endpoint_id,
                            prompt: Some(prompt.to_string()),
                            queue_ref: None,
                            publication_invocation: None,
                        },
                    ))
                })
                .expect("workflow invocation thread should spawn"),
        );
    }
    let responses = handles
        .into_iter()
        .map(|handle| handle.join().expect("workflow invocation should not panic"))
        .collect::<Result<Vec<_>, _>>()
        .expect("workflow invocations should succeed");
    assert!(responses
        .iter()
        .all(|response| matches!(response, LocalDaemonResponse::WorkflowRunInvoked { .. })));

    let state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
    });
    let active = state
        .active_prompt_for_agent(agent.id())
        .expect("one workflow prompt should be active");
    let queued = state
        .queued_prompts_for_agent(agent.id())
        .expect("the other workflow prompt should queue on the shared agent")
        .front()
        .expect("one prompt should be queued");
    assert_eq!(state.workflow_runs().len(), 2);
    assert_eq!(
        state
            .queued_prompts_for_agent(agent.id())
            .expect("agent queue should exist")
            .len(),
        1,
    );
    let active_run_id = active
        .workflow_run_id()
        .expect("active prompt should retain its workflow run");
    let queued_run_id = queued
        .workflow_run_id()
        .expect("queued prompt should retain its workflow run");
    let active_workflow_id = state
        .workflow_runs()
        .iter()
        .find(|run| run.id() == active_run_id)
        .expect("active run should resolve")
        .workflow_id();
    let queued_workflow_id = state
        .workflow_runs()
        .iter()
        .find(|run| run.id() == queued_run_id)
        .expect("queued run should resolve")
        .workflow_id();
    assert_ne!(active_workflow_id, queued_workflow_id);

    match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: active_run_id.to_string(),
            },
        ))
        .expect("active workflow should stop")
    {
        LocalDaemonResponse::WorkflowRunCancelled { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        harness.pump_transport_runtime();
        let promoted = harness.with_app(|app| {
            app.sessions()
                .get_session(session.id())
                .expect("session should remain available")
                .active_prompt_for_agent(agent.id())
                .is_some_and(|prompt| {
                    prompt.workflow_run_id() == Some(queued_run_id)
                        && prompt.durable_delivery_phase()
                            == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                })
        });
        if promoted {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the queued workflow prompt was not promoted after the shared agent became idle",
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn local_request_api_runs_independent_workflows_concurrently_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-independent-workflows", "worktree-root"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let first_worktree = std::env::temp_dir()
        .join("chariox-independent-workflow-first")
        .join(session.id());
    let second_worktree = std::env::temp_dir()
        .join("chariox-independent-workflow-second")
        .join(session.id());
    std::fs::create_dir_all(&first_worktree).expect("first worktree should exist");
    std::fs::create_dir_all(&second_worktree).expect("second worktree should exist");
    let first_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "independent-first",
        Some(&first_worktree.to_string_lossy()),
    );
    let second_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "independent-second",
        Some(&second_worktree.to_string_lossy()),
    );
    harness.launch_workflow_test_provider(session.id(), first_agent.id());
    harness.launch_workflow_test_provider(session.id(), second_agent.id());

    let create_workflow = |alias: &str, agent_id: &str| {
        let workflow = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some(alias.to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent_id);
        let endpoint = match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow endpoint should create")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        (workflow, endpoint)
    };
    let (first_workflow, first_endpoint) = create_workflow("independent-first", first_agent.id());
    let (second_workflow, second_endpoint) =
        create_workflow("independent-second", second_agent.id());

    let invoke = |workflow_ref: &str, endpoint_ref: &str, prompt: &str| match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_ref.to_string(),
                endpoint_ref: endpoint_ref.to_string(),
                prompt: Some(prompt.to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("workflow invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        other => panic!("workflow should start immediately, got {other:?}"),
    };
    let first_run = invoke(
        first_workflow.id(),
        first_endpoint.id(),
        "run first workflow",
    );
    let second_run = invoke(
        second_workflow.id(),
        second_endpoint.id(),
        "run second workflow",
    );

    let state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
    });
    let first_prompt = state
        .active_prompt_for_agent(first_agent.id())
        .expect("first workflow should own its agent");
    let second_prompt = state
        .active_prompt_for_agent(second_agent.id())
        .expect("second workflow should own its agent");
    assert_eq!(first_prompt.workflow_run_id(), Some(first_run.id()));
    assert_eq!(second_prompt.workflow_run_id(), Some(second_run.id()));
    assert!(state.workflow_queued_prompts().is_empty());
}

fn wait_for_workflow_run_status(
    harness: &LocalRouterTestHarness,
    session_id: &str,
    workflow_run_id: &str,
    statuses: &[WorkflowRunStatus],
) -> crate::session::WorkflowRun {
    for _ in 0..80 {
        let workflow_run = match harness
            .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session_id.to_string(),
                workflow_run_ref: workflow_run_id.to_string(),
            }))
            .expect("workflow run should resolve while waiting for status")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        if statuses
            .iter()
            .any(|status| workflow_run.status() == *status)
        {
            return workflow_run;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("workflow run `{workflow_run_id}` did not reach expected status");
}

fn wait_for_workflow_run_matching(
    harness: &LocalRouterTestHarness,
    session_id: &str,
    workflow_run_id: &str,
    reason: &str,
    predicate: impl Fn(&crate::session::WorkflowRun) -> bool,
) -> crate::session::WorkflowRun {
    let mut last = None;
    for _ in 0..200 {
        let workflow_run = harness.get_workflow_test_run(session_id, workflow_run_id);
        if predicate(&workflow_run) {
            return workflow_run;
        }
        last = Some(workflow_run);
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!(
        "workflow run `{workflow_run_id}` did not reach expected state ({reason}); last observation: {last:?}"
    );
}
