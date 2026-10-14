// MP-08 / MP-10 / MP-11: explicit worker failure cannot become home success.
use super::*;
use crate::git_observer::CompletedTurnSettlementStatus;

#[test]
fn mp08_mp10_failed_without_termination_marks_home_error() {
    assert_failed_projection(None);
}

#[test]
fn mp08_mp10_failed_without_termination_fails_workflow_with_valid_output() {
    assert_failed_projection(Some(true));
}

#[test]
fn mp08_mp10_failed_without_termination_fails_workflow_without_output() {
    assert_failed_projection(Some(false));
}

fn assert_failed_projection(workflow_output: Option<bool>) {
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("home", "home"))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: Some("run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "failure-test",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut prompt = PromptQueueItem::new(
        "failed-home-turn",
        attachment.id(),
        agent.id(),
        "research",
        PromptStatus::Queued,
    );
    let workflow = workflow_output.map(|valid_output| {
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("failure".into()))
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .unwrap();
        app.sessions_mut()
            .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), Some("entry".into()))
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("research".into()),
            )
            .unwrap();
        let node_run = run.node_runs()[0].id().to_string();
        let token = format!("workflow-ack:{node_run}");
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                run.id(),
                &node_run,
                token.clone(),
                "research".into(),
                None,
                None,
            )
            .unwrap();
        app.sessions_mut()
            .start_workflow_node_run(session.id(), run.id(), &node_run)
            .unwrap();
        app.sessions_mut()
            .mark_workflow_turn_dispatched(session.id(), run.id(), &node_run)
            .unwrap();
        app.sessions_mut()
            .ack_workflow_turn(session.id(), run.id(), &node_run, &token)
            .unwrap();
        if valid_output {
            app.sessions_mut()
                .submit_workflow_run_final_output(
                    session.id(),
                    run.id(),
                    &node_run,
                    crate::session::WorkflowOutputPayload::new(
                        r#"{"summary":"already produced"}"#,
                        Vec::new(),
                    ),
                    true,
                    None,
                )
                .unwrap();
        }
        prompt = prompt.clone().with_workflow_context(run.id(), &node_run);
        (run.id().to_string(), node_run)
    });
    let PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap()
    else {
        panic!("MP-08 failure fixture must admit an active prompt");
    };
    assert_eq!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .unwrap()
            .unwrap()
            .id(),
        prompt.id()
    );
    app.agents
        .set_remote_execution_active_worker_provider_run_id(agent.id(), Some("run".into()))
        .unwrap();
    crate::transport::flow_control::note_prompt_started(
        &mut app,
        &crate::provider::projected_leased_provider_run_id("leased", "run"),
    );
    let outcome = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel-1"),
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: session.id().into(),
                home_agent_id: agent.id().into(),
                provider_run_id: "run".into(),
                provider_run: None,
                prompts: vec![],
                output_chunks: vec![],
                notices: vec![],
                completions: vec![RelayProjectedCompletion {
                    message_id: "failed-worker-turn".into(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                    home_prompt_id: Some(prompt.id().into()),
                    settlement_status: Some(CompletedTurnSettlementStatus::Failed),
                    provider_termination: None,
                }],
            },
        )
        .unwrap();
    assert!(outcome.accepted);
    assert_eq!(outcome.completions.len(), 1);
    assert!(
        outcome.provider_failed,
        "MP-08 explicit Failed is failure even without optional termination"
    );
    assert_eq!(
        app.agents.get_agent(agent.id()).unwrap().state(),
        crate::agent::AgentState::Error
    );
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .unwrap()
        .is_none());
    assert_eq!(
        app.completed_git_turn_snapshot_store()
            .latest_projection_for_agent(session.id(), agent.id())
            .unwrap()
            .settlement_status,
        CompletedTurnSettlementStatus::Failed
    );
    if let Some((workflow_id, node_id)) = workflow {
        let snapshot = app.sessions().get_session(session.id()).unwrap();
        let run = snapshot.workflow_run(&workflow_id).unwrap();
        assert_eq!(run.status(), crate::session::WorkflowRunStatus::Failed);
        assert_eq!(
            run.node_runs()
                .iter()
                .find(|n| n.id() == node_id)
                .unwrap()
                .status(),
            crate::session::WorkflowNodeRunStatus::Failed
        );
        assert_eq!(
            run.failure_events().last().unwrap().kind(),
            crate::session::WorkflowFailureKind::ProviderFailure
        );
    }
}
