use super::*;

#[test]
fn local_request_api_routes_and_schedules_downstream_workflow_nodes() {
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

    let first_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("planner".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        }))
        .expect("first workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let second_agent = match harness
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
        }))
        .expect("second workflow agent should spawn")
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

    let first_node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: first_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let second_node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: second_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("second workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("route this workflow".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");
    assert_eq!(workflow_run.node_runs().len(), 1);
    let workflow_attachment_id =
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id());
    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("workflow invoke should activate a provider run")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"planner finished draft plan\",\"output\":{\"message\":\"Please review the attached generated plan and provide approval feedback.\"}}\n```\n",
        );
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            b"{\"tool\":\"rg\",\"status\":\"ok\"}\n",
        );
    });
    let workflow_transfer_root =
        crate::app::attachment_artifact_root(session.id(), &workflow_attachment_id, "transfers");
    std::fs::create_dir_all(&workflow_transfer_root).expect("workflow transfer root should exist");
    let workflow_artifact_path = workflow_transfer_root.join("generated-plan.md");
    std::fs::write(&workflow_artifact_path, "# generated plan\n")
        .expect("workflow artifact should be written");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("entry workflow prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let routed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("routed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", routed.status()), "Running");
    assert_eq!(routed.node_runs().len(), 2);
    assert_eq!(routed.messages().len(), 2);
    assert_eq!(
        routed.active_node_run_id(),
        Some(routed.node_runs()[1].id())
    );
    assert_eq!(routed.node_runs()[1].node_id(), second_node.id());
    let completed_entry = routed
        .node_runs()
        .iter()
        .find(|node_run| node_run.node_id() == first_node.id())
        .expect("completed entry node should remain on the run");
    assert_eq!(format!("{:?}", completed_entry.status()), "Completed");
    assert!(completed_entry
        .summary()
        .is_some_and(|summary| summary.contains("planner finished draft plan")));
    let completion = completed_entry
        .completion()
        .expect("completed entry node should retain a generic completion snapshot");
    assert_eq!(completion.summary(), "planner finished draft plan");
    let output = completion
        .output()
        .expect("completed entry node should retain explicit downstream output");
    assert_eq!(
        output.message(),
        "Please review the attached generated plan and provide approval feedback."
    );
    assert_eq!(output.artifacts().len(), 1);
    assert_eq!(output.artifacts()[0].kind(), "transfer");
    assert_eq!(output.artifacts()[0].display_name(), "generated-plan.md");
    assert_eq!(
        output.artifacts()[0].path(),
        workflow_artifact_path.to_string_lossy()
    );
    let handoff_message = routed
        .messages()
        .iter()
        .find(|message| message.source_node_run_id() == Some(completed_entry.id()))
        .expect("downstream handoff message should exist");
    let handoff_payload: WorkflowHandoffPayload =
        serde_json::from_str(handoff_message.handoff_payload())
            .expect("handoff payload should deserialize");
    let handoff_completion = handoff_payload
        .completion()
        .expect("handoff payload should carry the generic completion snapshot");
    assert_eq!(handoff_completion.summary(), "planner finished draft plan");
    let handoff_output = handoff_completion
        .output()
        .expect("handoff payload should carry explicit downstream output");
    assert_eq!(
        handoff_output.message(),
        "Please review the attached generated plan and provide approval feedback."
    );
    assert_eq!(handoff_output.artifacts().len(), 1);
    assert_eq!(
        handoff_output.artifacts()[0].display_name(),
        "generated-plan.md"
    );

    harness.complete_workflow_test_prompt(session.id(), "downstream workflow prompt");

    let completed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("completed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", completed.status()), "Completed");
    assert_eq!(completed.node_runs().len(), 2);
    assert_eq!(
        completed
            .node_runs()
            .iter()
            .map(|node_run| format!("{:?}", node_run.status()))
            .collect::<Vec<_>>(),
        vec!["Completed".to_string(), "Completed".to_string()]
    );
}

#[test]
fn local_request_api_waits_for_all_join_inputs_before_scheduling_downstream_node() {
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

    let entry_agent = harness.spawn_workflow_test_agent(session.id(), "entry");
    let branch_one_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "branch-one",
        Some("worktree-branch-one"),
    );
    let branch_two_agent = harness.spawn_workflow_test_agent_with_worktree(
        session.id(),
        "branch-two",
        Some("worktree-branch-two"),
    );
    let join_agent = harness.spawn_workflow_test_agent(session.id(), "join");

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("join".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let entry_node = harness.add_workflow_test_node(session.id(), workflow.id(), entry_agent.id());
    let branch_one_node =
        harness.add_workflow_test_node(session.id(), workflow.id(), branch_one_agent.id());
    let branch_two_node =
        harness.add_workflow_test_node(session.id(), workflow.id(), branch_two_agent.id());
    let join_node = harness.add_workflow_test_node(session.id(), workflow.id(), join_agent.id());
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        entry_node.id(),
        branch_one_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        entry_node.id(),
        branch_two_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        branch_one_node.id(),
        join_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        branch_two_node.id(),
        join_node.id(),
    );

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: entry_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("run the join drill".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };

    harness.complete_workflow_test_prompt(session.id(), "entry workflow prompt");
    let after_entry = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(after_entry.node_runs().len(), 3);
    let session_after_entry = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve after entry")
            .clone()
    });
    let active_branch_agents = [branch_one_agent.id(), branch_two_agent.id()]
        .into_iter()
        .filter(|agent_id| {
            session_after_entry
                .active_prompt_for_agent(agent_id)
                .is_some()
        })
        .collect::<Vec<_>>();
    let active_prompt_count = session_after_entry
        .prompt_states()
        .values()
        .filter(|state| state.active_prompt().is_some())
        .count();
    let queued_prompt_count = session_after_entry
        .prompt_states()
        .values()
        .map(|state| state.queued_prompts().len())
        .sum::<usize>();
    assert!(
        active_prompt_count >= 1,
        "expected at least one branch prompt to be active after entry completed"
    );
    assert_eq!(active_prompt_count + queued_prompt_count, 2);
    assert_eq!(active_branch_agents.len(), 2);
    assert_eq!(
        after_entry
            .node_runs()
            .iter()
            .filter(|node_run| {
                node_run.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
            })
            .count(),
        0
    );

    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        active_branch_agents[0],
        "first branch workflow prompt",
    );
    let after_first_branch = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(after_first_branch.node_runs().len(), 3);
    assert!(after_first_branch
        .node_runs()
        .iter()
        .all(|node_run| node_run.node_id() != join_node.id()));
    let buffered_join_messages = after_first_branch
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(buffered_join_messages.len(), 1);
    assert!(buffered_join_messages[0]
        .consumed_by_node_run_id()
        .is_none());
    let session_after_first_branch = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve after first branch")
            .clone()
    });
    let remaining_active_branch_agents = active_branch_agents
        .iter()
        .copied()
        .filter(|agent_id| {
            session_after_first_branch
                .active_prompt_for_agent(agent_id)
                .is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_active_branch_agents.len(), 1);
    assert_eq!(session_after_first_branch.queued_prompts().len(), 0);

    harness.complete_workflow_test_prompt_for_agent(
        session.id(),
        remaining_active_branch_agents[0],
        "second branch workflow prompt",
    );
    let after_second_branch = harness.get_workflow_test_run(session.id(), workflow_run.id());
    let join_runs = after_second_branch
        .node_runs()
        .iter()
        .filter(|node_run| node_run.node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(join_runs.len(), 1);
    let join_run = join_runs[0];
    let join_messages = after_second_branch
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(join_messages.len(), 2);
    assert!(join_messages
        .iter()
        .all(|message| message.consumed_by_node_run_id() == Some(join_run.id())));

    harness.complete_workflow_test_prompt(session.id(), "join workflow prompt");
    let completed = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(format!("{:?}", completed.status()), "Completed");
    assert_eq!(completed.node_runs().len(), 4);
}

#[test]
fn workflow_node_dispatch_blocks_and_retries_on_workspace_claim_release() {
    let harness = LocalRouterTestHarness::new();
    let (interactive_session, interactive_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("interactive session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let interactive_attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: interactive_session.id().to_string(),
                client_id: "client-workflow-claim-owner".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("interactive attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let interactive_provider_run_id = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                interactive_session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(interactive_agent.id()),
        )
        .expect("interactive provider run should launch")
        .id()
        .to_string()
    });
    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: interactive_session.id().to_string(),
            attachment_id: interactive_attachment.id().to_string(),
            target_agent_id: Some(interactive_agent.id().to_string()),
            prompt: "hold the worktree".to_string(),
            attachments: Vec::new(),
        }))
        .expect("interactive prompt should start")
    {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { .. } => {}
            _ => panic!("expected interactive prompt to start"),
        },
        _ => panic!("unexpected local response"),
    }
    harness.with_app_mut(|app| {
        let claim = app
            .workspace_coordinator()
            .acquire_worktree_write_claim(
                "workspace-1".to_string(),
                "worktree-shared".to_string(),
                interactive_session.id().to_string(),
                Some("interactive-test-claim".to_string()),
                "interactive_prompt_test",
            )
            .expect("interactive test claim should acquire");
        app.prompt_workspace_claim_store()
            .insert(interactive_provider_run_id.clone(), claim);
    });

    let workflow_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("workflow session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let workflow_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: workflow_session.id().to_string(),
            alias: Some("workflow-worker".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: workflow_session.id().to_string(),
            alias: Some("blocked".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: workflow_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: workflow_agent.id().to_string(),
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
                session_id: workflow_session.id().to_string(),
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
    let blocked_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: workflow_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("background work".to_string()),
            },
        ))
        .expect("workflow invoke should block instead of fail")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        blocked_run.status(),
        crate::session::WorkflowRunStatus::Waiting
    );
    assert_eq!(
        blocked_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
    );

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: interactive_session.id().to_string(),
        }))
        .expect("interactive prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    let retried_run = loop {
        let workflow_run = match harness
            .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: workflow_session.id().to_string(),
                workflow_run_ref: blocked_run.id().to_string(),
            }))
            .expect("workflow run should resolve after retry")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        if workflow_run.status() == crate::session::WorkflowRunStatus::Running
            || Instant::now() >= deadline
        {
            break workflow_run;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        retried_run.status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert_eq!(
        retried_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Running
    );
    assert_eq!(
        retried_run.active_node_run_id(),
        Some(retried_run.node_runs()[0].id())
    );
}

#[test]
fn workflow_run_cancel_retries_other_runs_blocked_on_released_claim() {
    let harness = LocalRouterTestHarness::new();
    let first_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("first workflow session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let first_agent = harness.spawn_workflow_test_agent(first_session.id(), "first-worker");
    let first_workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: first_session.id().to_string(),
            alias: Some("first".to_string()),
        }))
        .expect("first workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let first_node =
        harness.add_workflow_test_node(first_session.id(), first_workflow.id(), first_agent.id());
    let first_endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: first_session.id().to_string(),
                workflow_ref: first_workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let first_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: first_session.id().to_string(),
                workflow_ref: first_workflow.id().to_string(),
                endpoint_ref: first_endpoint.id().to_string(),
                prompt: Some("hold the workflow claim".to_string()),
            },
        ))
        .expect("first workflow should start")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        first_run.status(),
        crate::session::WorkflowRunStatus::Running
    );

    let blocked_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("blocked workflow session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let blocked_agent = harness.spawn_workflow_test_agent(blocked_session.id(), "blocked-worker");
    let blocked_workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: blocked_session.id().to_string(),
            alias: Some("blocked".to_string()),
        }))
        .expect("blocked workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let blocked_node = harness.add_workflow_test_node(
        blocked_session.id(),
        blocked_workflow.id(),
        blocked_agent.id(),
    );
    let blocked_endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: blocked_session.id().to_string(),
                workflow_ref: blocked_workflow.id().to_string(),
                entry_node_id: blocked_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("blocked workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let blocked_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: blocked_session.id().to_string(),
                workflow_ref: blocked_workflow.id().to_string(),
                endpoint_ref: blocked_endpoint.id().to_string(),
                prompt: Some("wait for claim release".to_string()),
            },
        ))
        .expect("blocked workflow invoke should wait on workspace claim")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        blocked_run.status(),
        crate::session::WorkflowRunStatus::Waiting
    );
    assert_eq!(
        blocked_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
    );

    match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: first_session.id().to_string(),
                workflow_run_ref: first_run.id().to_string(),
            },
        ))
        .expect("first workflow run should cancel")
    {
        LocalDaemonResponse::WorkflowRunCancelled { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    let retried_run = loop {
        let workflow_run = harness.get_workflow_test_run(blocked_session.id(), blocked_run.id());
        if workflow_run.status() == crate::session::WorkflowRunStatus::Running
            || Instant::now() >= deadline
        {
            break workflow_run;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        retried_run.status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert_eq!(
        retried_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Running
    );
}
