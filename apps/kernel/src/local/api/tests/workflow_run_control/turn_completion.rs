use super::*;

#[test]
fn local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes() {
    run_workflow_turn_completion_large_stack_test(
        "local-request-api-acks-workflow-turn",
        local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes_inner,
    );
}

#[test]
fn local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns() {
    run_workflow_turn_completion_large_stack_test(
        "local-request-api-inlines-mailbox-content",
        local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns_inner,
    );
}

fn run_workflow_turn_completion_large_stack_test(name: &str, test: fn()) {
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(test)
        .expect("workflow turn completion large-stack test thread should spawn");
    if let Err(payload) = handle.join() {
        std::panic::resume_unwind(payload);
    }
}

fn local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes_inner(
) {
    const FIRST_PRIVATE_INSTRUCTIONS: &str =
        "# First node\nProduce a tiny JSON payload.\nUPSTREAM_PRIVATE_INSTRUCTION_TOKEN\n";
    const SECOND_PRIVATE_INSTRUCTIONS: &str =
        "# Second node\nSummarize the handoff.\nDOWNSTREAM_PRIVATE_INSTRUCTION_TOKEN\n";
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-ack", "worktree-ack"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let first_agent = harness.spawn_workflow_test_agent(session.id(), "first");
    let second_agent = harness.spawn_workflow_test_agent(session.id(), "second");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("ack-flow".to_string()),
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
        .expect("first node should be added")
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
        .expect("second node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let _ = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: first_node.id().to_string(),
                instructions: Some(FIRST_PRIVATE_INSTRUCTIONS.to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("first node instructions should be updated");
    let _ = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: second_node.id().to_string(),
                instructions: Some(SECOND_PRIVATE_INSTRUCTIONS.to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("second node instructions should be updated");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                handoff_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
                source_side: None,
                target_side: None,
            },
        ))
        .expect("edge should be added");
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
        .expect("endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let (workflow_run, invoke_session) = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("kick off the ack flow".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run,
            session,
            ..
        } => (workflow_run, session),
        _ => panic!("unexpected local response"),
    };
    let active_prompt = invoke_session
        .active_prompt_for_agent(first_agent.id())
        .expect("workflow invoke should create an active prompt");
    let active_mechanics = workflow_mechanics_text(active_prompt);
    assert_eq!(active_prompt.prompt(), "kick off the ack flow");
    assert!(active_mechanics.contains("<node-instruction-reference>"));
    assert!(active_mechanics.contains("`ack_workflow_turn`"));
    assert!(!active_mechanics.contains("Control mailbox (daemon-managed):"));
    assert!(active_mechanics.contains("UPSTREAM_PRIVATE_INSTRUCTION_TOKEN"));
    assert!(!active_prompt
        .prompt()
        .contains("DOWNSTREAM_PRIVATE_INSTRUCTION_TOKEN"));
    assert!(!active_mechanics.contains("DOWNSTREAM_PRIVATE_INSTRUCTION_TOKEN"));

    let first_run_id = workflow_run.node_runs()[0].id().to_string();
    let first_token = "workflow-ack:".to_string() + &first_run_id;
    match harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: first_run_id.clone(),
                delivery_token: first_token,
            },
        ))
        .expect("workflow turn ack should succeed")
    {
        LocalDaemonResponse::WorkflowTurnAcknowledged { workflow_run, .. } => {
            let envelope = workflow_run.node_runs()[0]
                .turn_envelope()
                .expect("first turn envelope should exist");
            assert_eq!(envelope.state(), WorkflowTurnRuntimeState::Acknowledged);
        }
        _ => panic!("unexpected local response"),
    }

    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderReasoning,
            Some("thinking:first".to_string()),
            Vec::new(),
            b"First node is reasoning about the handoff.",
        );
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"first finished\",\"output\":{\"message\":\"{\\\"value\\\":1}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("first workflow prompt should complete");

    let _ = harness.wait_for_session_where(
        session.id(),
        "second workflow node prompt should become active after provider launch",
        |session| session.active_prompt_for_agent(second_agent.id()).is_some(),
    );
    let routed = harness.wait_for_workflow_test_run_where(
        session.id(),
        workflow_run.id(),
        "second workflow node run should become active after first completion",
        |workflow_run| workflow_run.active_node_run_id().is_some(),
    );
    let first_completed = routed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == first_run_id)
        .expect("first node run should remain");
    assert_eq!(first_completed.thinking_traces().len(), 1);
    assert_eq!(
        first_completed.thinking_traces()[0].message(),
        "First node is reasoning about the handoff."
    );
    let first_envelope = first_completed
        .turn_envelope()
        .expect("first node run should retain its envelope");
    assert_eq!(
        first_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert!(first_envelope.rendered_prompt().is_none());
    assert!(first_envelope.handoff_payloads_json().is_none());
    assert_eq!(routed.messages().len(), 2);
    let invocation_message = routed
        .messages()
        .iter()
        .find(|message| message.message_type() == "invocation")
        .expect("workflow invocation message should remain durable");
    assert_eq!(invocation_message.source_node_run_id(), None);
    assert_eq!(invocation_message.target_node_id(), first_node.id());
    assert_eq!(
        invocation_message.consumed_by_node_run_id(),
        Some(first_run_id.as_str())
    );
    let handoff_message = routed
        .messages()
        .iter()
        .find(|message| message.message_type() == "handoff")
        .expect("downstream handoff message should be recorded");
    assert_eq!(
        handoff_message.source_node_run_id(),
        Some(first_run_id.as_str())
    );
    assert_eq!(handoff_message.target_node_id(), second_node.id());

    let second_active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node prompt should be active")
    });
    let second_mechanics = workflow_mechanics_text(&second_active_prompt);
    assert!(second_active_prompt
        .prompt()
        .contains("\"message_type\":\"handoff\""));
    assert!(!second_active_prompt
        .prompt()
        .contains("<workflow-handoff-payloads>"));
    assert!(second_mechanics.contains("`ack_workflow_turn`"));
    assert!(second_mechanics.contains("DOWNSTREAM_PRIVATE_INSTRUCTION_TOKEN"));
    assert!(!second_active_prompt
        .prompt()
        .contains("UPSTREAM_PRIVATE_INSTRUCTION_TOKEN"));
    assert!(!second_mechanics.contains("UPSTREAM_PRIVATE_INSTRUCTION_TOKEN"));
    assert!(
        second_active_prompt
            .prompt()
            .contains("\"message_type\":\"handoff\""),
        "unexpected visible workflow handoff: {:?}",
        second_active_prompt.prompt()
    );
    let second_history_prompts = harness.with_app(|app| {
        app.operational_history_store()
            .load_session_history_entries(session.id(), Some(second_agent.id()))
            .expect("second agent operational history should load")
    });
    let second_history_prompt = second_history_prompts
        .iter()
        .find(|entry| entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt)
        .expect("second agent workflow prompt should be recorded");
    assert!(second_history_prompt
        .text
        .contains("\"message_type\":\"handoff\""));
    assert!(!second_history_prompt
        .text
        .contains("<workflow-handoff-payloads>"));
    assert!(!second_history_prompt.text.contains("`ack_workflow_turn`"));

    let second_run_id = routed
        .active_node_run_id()
        .expect("second node should be active")
        .to_string();
    let second_token = "workflow-ack:".to_string() + &second_run_id;
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: second_run_id.clone(),
                delivery_token: second_token,
            },
        ))
        .expect("second workflow turn ack should succeed");
    let second_provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &second_provider_run_id,
            TerminalOutputKind::ProviderReasoning,
            Some("thinking:second".to_string()),
            Vec::new(),
            b"Second node is reasoning about the final answer.",
        );
        app.fan_out_output(
            session.id(),
            &second_provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"second finished\",\"output\":{\"message\":\"{\\\"done\\\":true}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("second workflow prompt should complete");
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
    let second_completed = completed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == second_run_id)
        .expect("second node should complete");
    assert_eq!(second_completed.thinking_traces().len(), 1);
    assert_eq!(
        second_completed.thinking_traces()[0].message(),
        "Second node is reasoning about the final answer."
    );
    let second_envelope = second_completed
        .turn_envelope()
        .expect("second node turn envelope should exist");
    assert_eq!(
        second_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert_eq!(completed.messages().len(), 2);
    assert!(completed
        .messages()
        .iter()
        .all(|message| message.consumed_by_node_run_id().is_some()));
    assert_eq!(
        completed
            .messages()
            .iter()
            .find(|message| message.message_type() == "invocation")
            .and_then(|message| message.consumed_by_node_run_id()),
        Some(first_run_id.as_str())
    );
    assert_eq!(
        completed
            .messages()
            .iter()
            .find(|message| message.message_type() == "handoff")
            .and_then(|message| message.consumed_by_node_run_id()),
        Some(second_run_id.as_str())
    );
}

fn local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-mailbox", "worktree-mailbox"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let first_agent = harness.spawn_workflow_test_agent(session.id(), "loop-a");
    let second_agent = harness.spawn_workflow_test_agent(session.id(), "loop-b");
    harness.launch_workflow_test_provider(session.id(), second_agent.id());
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("mailbox-flow".to_string()),
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
        .expect("first node should be added")
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
        .expect("second node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let schema_path = std::env::temp_dir().join(format!(
        "arroba-mailbox-schema-{}.json",
        crate::session::unix_epoch_ms()
    ));
    fs::write(
            &schema_path,
            "{\n  \"type\": \"object\",\n  \"required\": [\"ok\"],\n  \"properties\": {\"ok\": {\"type\": \"boolean\"}}\n}\n",
        )
        .expect("schema file should be written");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                handoff_schema_ref: Some(schema_path.to_string_lossy().to_string()),
                validation_policy: Some(WorkflowHandoffValidationPolicy::Warn),
                expected_workflow_revision: None,
                source_side: None,
                target_side: None,
            },
        ))
        .expect("first edge should be added");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: second_node.id().to_string(),
                to_node_id: first_node.id().to_string(),
                handoff_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
                source_side: None,
                target_side: None,
            },
        ))
        .expect("second edge should be added");
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
        .expect("endpoint should be created")
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
                prompt: Some("start loop".to_string()),
                queue_ref: None,
                publication_invocation: None,
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: node_run_id.clone(),
                delivery_token: format!("workflow-ack:{node_run_id}"),
            },
        ))
        .expect("ack should succeed");
    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"warn branch\",\"output\":{\"message\":\"not-json\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("warning workflow prompt should complete");

    let _ = harness.wait_for_session_where(
        session.id(),
        "second node should become active after warning handoff",
        |session| session.active_prompt_for_agent(second_agent.id()).is_some(),
    );
    let after_warning = harness.wait_for_workflow_test_run_where(
        session.id(),
        workflow_run.id(),
        "second workflow node run should become active after warning handoff",
        |workflow_run| workflow_run.active_node_run_id().is_some(),
    );
    assert!(after_warning.failure_events().iter().any(|event| {
        matches!(
            event.kind(),
            crate::session::WorkflowFailureKind::OutputValidationFailed
        ) && event.message().contains("output.message is not valid JSON")
    }));
    let second_active_prompt = harness
        .wait_for_session_where(
            session.id(),
            "second node active prompt should be visible",
            |session| session.active_prompt_for_agent(second_agent.id()).is_some(),
        )
        .active_prompt_for_agent(second_agent.id())
        .expect("second node should be active")
        .clone();
    let second_mechanics = workflow_mechanics_text(&second_active_prompt);
    assert!(second_mechanics.contains("<control-mailbox>"));
    assert!(second_mechanics.contains("output.message is not valid JSON"));
    let first_completed = after_warning
        .node_runs()
        .iter()
        .find(|run| run.id() == node_run_id)
        .expect("first node run should remain");
    assert_eq!(
        first_completed
            .turn_envelope()
            .expect("turn envelope should remain")
            .state(),
        WorkflowTurnRuntimeState::Acknowledged
    );
    assert!(first_completed
        .turn_envelope()
        .expect("turn envelope should remain")
        .rendered_prompt()
        .is_some());

    let second_run_id = after_warning
        .active_node_run_id()
        .expect("second node should now be active")
        .to_string();
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: second_run_id.clone(),
                delivery_token: format!("workflow-ack:{second_run_id}"),
            },
        ))
        .expect("second node ack should succeed");
    let second_provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &second_provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"loop back\",\"output\":{\"message\":\"{\\\"ok\\\":true}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("second node prompt should complete");

    let active_prompt = harness
        .wait_for_session_where(
            session.id(),
            "first node should become active again",
            |session| session.active_prompt_for_agent(first_agent.id()).is_some(),
        )
        .active_prompt_for_agent(first_agent.id())
        .expect("first node should be active again")
        .clone();
    let active_mechanics = workflow_mechanics_text(&active_prompt);
    assert!(active_mechanics.contains("<control-mailbox>"));
    assert!(active_mechanics.contains("output.message is not valid JSON"));
    assert!(
        active_mechanics.contains("Treat the control mailbox as authoritative runtime feedback")
    );
    assert!(active_mechanics.contains("<outgoing-edge-contracts>"));
    assert!(active_mechanics.contains(schema_path.to_string_lossy().as_ref()));
    assert!(!active_mechanics.contains("Control mailbox (daemon-managed):"));
}

fn workflow_mechanics_text(prompt: &crate::session::PromptQueueItem) -> &str {
    if prompt.hidden_system_context().is_empty() {
        prompt.prompt()
    } else {
        prompt.hidden_system_context()
    }
}
