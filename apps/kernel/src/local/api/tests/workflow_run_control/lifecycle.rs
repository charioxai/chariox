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
fn local_request_api_serializes_concurrent_workflow_launch_admission() {
    run_workflow_run_lifecycle_large_stack_test(
        "local-request-api-serializes-concurrent-workflow-launch-admission",
        local_request_api_serializes_concurrent_workflow_launch_admission_inner,
    );
}

#[test]
fn stopping_workflow_dispatches_next_queued_workflow_prompt() {
    run_workflow_run_lifecycle_large_stack_test(
        "stopping-workflow-dispatches-next-queued-workflow-prompt",
        stopping_workflow_dispatches_next_queued_workflow_prompt_inner,
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
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
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
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
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
    match harness
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
        .expect("second workflow should enqueue")
    {
        LocalDaemonResponse::WorkflowPromptEnqueued { .. } => {}
        _ => panic!("unexpected local response"),
    }
    match harness
        .dispatch(LocalDaemonRequest::PauseWorkflowRun(
            PauseWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: first_run.id().to_string(),
            },
        ))
        .expect("first workflow should pause")
    {
        LocalDaemonResponse::WorkflowRunPaused { .. } => {}
        _ => panic!("unexpected local response"),
    }
    harness.wait_for_session_where(
        session.id(),
        "paused workflow prompt should settle before stop",
        |session| session.active_prompt_for_agent(agent.id()).is_none(),
    );
    match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: first_run.id().to_string(),
            },
        ))
        .expect("first workflow should stop")
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
    assert_eq!(advanced.workflow_runs().len(), 2);
    assert_eq!(
        advanced.workflow_runs()[1].status(),
        WorkflowRunStatus::Running
    );
}

fn local_request_api_serializes_concurrent_workflow_launch_admission_inner() {
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
    assert_eq!(started, 1, "exactly one concurrent invoke should start");
    assert_eq!(enqueued, INVOCATION_COUNT - 1);

    let workflow_runs = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        workflow_runs.len(),
        1,
        "concurrent admission created extra runs"
    );
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
