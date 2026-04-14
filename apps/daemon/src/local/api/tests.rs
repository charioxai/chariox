use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::provider_output::{ProviderOutputPump, ProviderOutputPumpRequest};
use crate::attachment::ClientCapabilityLevel;
use crate::local::test_support::LocalRouterTestHarness;
use crate::provider::{LaunchProviderRequest, ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::session::{
    CreateSessionRequest, PromptSubmissionOutcome, WorkflowHandoffPayload, WorkflowNodeRunStatus,
    WorkflowOutputValidationPolicy, WorkflowTurnRuntimeState,
};
use crate::terminal::TerminalOutputKind;
use crate::transport::TransportService;
use crate::{DaemonApp, DaemonConfig, DaemonError};
use arroba_relay::{protocol::DaemonRegistration, RelayConfig, RelayServer};

use super::{
    AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasSessionRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, AttachToSessionRequest,
    CancelActivePromptRequest, CancelWorkflowRunRequest, CaptureScreenshotCapabilityRequest,
    CompletePromptRequest, CreateWorkflowEndpointRequest, CreateWorkflowRequest,
    CycleAgentFocusRequest, DeleteSessionRequest, DetachFromSessionRequest,
    EditFileCapabilityRequest, EndSessionRequest, FocusAgentRequest, GetDaemonHealthRequest,
    GetSessionStateRequest, GetWorkflowRunRequest, InspectGitCapabilityRequest,
    InvokeWorkflowEndpointRequest, LaunchProviderRunRequest, ListAgentsRequest,
    ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest, ListSessionsRequest,
    ListWorkflowRunsRequest, ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse,
    PollRuntimeNoticesRequest, ReadDirectoryTreeCapabilityRequest, ReadFileCapabilityRequest,
    RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest, ResolveSessionRequest,
    ResolveWorkflowRequest, ResumeWorkflowRunRequest, RunShellCapabilityRequest, SpawnAgentRequest,
    StoreTransferredFileCapabilityRequest, SubmitPromptRequest, UpdateSessionConfigRequest,
    UpdateWorkflowNodeInstructionsRequest,
};
use futures_util::SinkExt;
use tokio::sync::oneshot;
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn launch_slow_structured_run(app: &mut DaemonApp, session_id: &str, agent_id: &str) -> String {
    app.launch_provider(
        LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "default",
        )
        .with_agent_id(agent_id),
    )
    .expect("slow structured provider run should launch")
    .id()
    .to_string()
}

#[test]
fn local_request_api_supports_session_attach_and_end() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let ended = match harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .expect("end session should succeed")
    {
        LocalDaemonResponse::SessionEnded { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), attachment.id());
    assert_eq!(ended.id(), session.id());
    harness.with_app(|app| {
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    });
}

#[test]
fn structured_output_pump_applies_finished_jobs_from_other_runs() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-structured-output", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-structured-output".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let worker_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("worker".to_string()),
            provider: "slow-structured".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }))
        .expect("worker agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (background_run_id, requested_records) = harness.with_app_mut(|app| {
        let background_run_id = launch_slow_structured_run(app, session.id(), default_agent.id());
        let requested_run_id = launch_slow_structured_run(app, session.id(), worker_agent.id());
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                background_run_id.clone(),
                Ok(Some(ProviderPromptSignalBatch {
                    chunks: vec![ProviderPromptChunk {
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("background-chunk".to_string()),
                        bytes: b"background-run-output\n".to_vec(),
                    }],
                    ..ProviderPromptSignalBatch::default()
                })),
            );

        let recipient_attachment_ids = app.attachments().list_session_attachment_ids(session.id());
        let requested_records = ProviderOutputPump::new(app)
            .pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &requested_run_id,
                recipient_attachment_ids,
            })
            .expect("requested run pump should drain all finished structured jobs");
        (background_run_id, requested_records)
    });

    assert!(
        requested_records.is_empty(),
        "background run output should be buffered for recipients, not returned as requested-run output"
    );
    let buffered_records = harness.with_app_mut(|app| {
        app.terminal_mut()
            .drain_output_records(session.id(), attachment.id())
    });
    assert_eq!(buffered_records.len(), 1);
    assert_eq!(buffered_records[0].provider_run_id, background_run_id);
    assert_eq!(buffered_records[0].bytes, b"background-run-output\n");
}

#[test]
fn local_request_api_lists_live_remote_machines_and_kernels() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let (addr, shutdown_tx, server_thread) = runtime.block_on(async {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);
        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("relay runtime should build");
            runtime.block_on(async move {
                server
                    .run_until(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            });
        });
        (addr, shutdown_tx, server_thread)
    });

    runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut socket, _) = connect_async(&url)
            .await
            .expect("daemon should connect to relay");
        let register = DaemonRegistration {
            auth_token: "secret".to_string(),
            daemon_id: "daemon-1".to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: Some("workstation".to_string()),
            os_name: Some("macOS".to_string()),
            kernel_started_at_ms: 10,
            daemon_alias: Some("mbp".to_string()),
            kernel_alias: Some("default".to_string()),
            public_key: "public-key".to_string(),
            capabilities: vec!["kernel_ws".to_string()],
            available_providers: vec!["codex".to_string(), "opencode".to_string()],
            accepting_remote_leases: true,
            leased_agent_count: 2,
            local_session_count: 3,
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&arroba_relay::protocol::RelayEnvelope::DaemonRegister {
                    registration: register.clone(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("register frame should send");
    });

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    let harness = LocalRouterTestHarness::with_config(config);

    let machines = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachines(
            ListRemoteMachinesRequest,
        ))
        .expect("remote machines request should succeed")
    {
        LocalDaemonResponse::RemoteMachinesListed { machines } => machines,
        other => panic!("unexpected response: {other:?}"),
    };
    let machine = machines
        .iter()
        .find(|machine| machine.machine_id == "machine-1")
        .expect("registered machine should be listed");
    assert_eq!(machine.machine_alias.as_deref(), Some("machine 1 (macOS)"));
    assert_eq!(machine.available_providers, vec!["codex", "opencode"]);

    let kernels = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachineKernels(
            ListRemoteMachineKernelsRequest {
                machine_ref: "machine 1 (macOS)".to_string(),
            },
        ))
        .expect("remote machine kernels request should succeed")
    {
        LocalDaemonResponse::RemoteMachineKernelsListed { kernels, .. } => kernels,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(kernels.len(), 1);
    assert_eq!(kernels[0].kernel_id, "daemon-1");
    assert_eq!(kernels[0].available_providers, vec!["codex", "opencode"]);
    assert!(kernels[0].accepting_remote_leases);

    let _ = shutdown_tx.send(());
    server_thread.join().expect("relay thread should join");
}

#[test]
fn local_request_api_resolves_and_deletes_sessions_by_ref() {
    let harness = LocalRouterTestHarness::new();
    let (session, _agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "mai".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    let deleted = match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: session.id()[..8].to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("delete should succeed")
    {
        LocalDaemonResponse::SessionDeleted { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
    assert_eq!(deleted.id(), session.id());
    assert_eq!(deleted.alias(), Some("main"));
    assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
    assert!(matches!(
        harness.dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "main".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        })),
        Err(DaemonError::SessionNotFound { .. })
    ));
    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());
}

#[test]
fn local_request_api_aliases_sessions() {
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

    let aliased = match harness
        .dispatch(LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session.id().to_string(),
            alias: "alpha".to_string(),
        }))
        .expect("alias should succeed")
    {
        LocalDaemonResponse::SessionAliased { session } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased.alias(), Some("alpha"));

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "alpha".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("alias resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
}

#[test]
fn local_request_api_spawns_and_focuses_agents() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: "opencode".to_string(),
            model: Some("openai/gpt-5.4".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.agents().len(), 2);
    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state
            .agents()
            .iter()
            .map(|agent| agent.id())
            .collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == default_agent.id())
            .expect("default agent should still exist")
            .state(),
        crate::agent::AgentState::Idle
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should exist")
            .state(),
        crate::agent::AgentState::Focused
    );

    let focused_default = match harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should succeed")
    {
        LocalDaemonResponse::AgentFocused { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(focused_default.id(), default_agent.id());

    let cycled = match harness
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("cycle should succeed")
    {
        LocalDaemonResponse::AgentFocusCycled { agent } => {
            agent.expect("cycle should return a focused agent")
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cycled.id(), spawned.id());

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("list should succeed")
    {
        LocalDaemonResponse::AgentsListed { agents } => agents,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        listed
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should be listed")
            .state(),
        crate::agent::AgentState::Focused
    );
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
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
            provider: "opencode".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
                output_schema_ref: None,
                validation_policy: None,
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowEdge(
            RemoveWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                edge_id: edge.id().to_string(),
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
            },
        ))
        .expect("workflow node should be removed")
    {
        LocalDaemonResponse::WorkflowNodeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_invokes_lists_gets_and_cancels_workflow_runs() {
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
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
                alias: Some("entry".to_string()),
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
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff".to_string()),
            },
        ))
        .expect("workflow run invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");

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
    assert_eq!(format!("{:?}", resolved.status()), "Running");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow-backed prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

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

    let second_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff again".to_string()),
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
}

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
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
        DaemonApp::attachment_artifact_root(session.id(), &workflow_attachment_id, "transfers");
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

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("downstream workflow prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

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
fn local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes() {
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
                instructions: Some("# First node\nProduce a tiny JSON payload.\n".to_string()),
            },
        ))
        .expect("first node instructions should be updated");
    let _ = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: second_node.id().to_string(),
                instructions: Some("# Second node\nSummarize the handoff.\n".to_string()),
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
                output_schema_ref: None,
                validation_policy: None,
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
        .active_prompt()
        .expect("workflow invoke should create an active prompt");
    assert!(active_prompt
        .prompt()
        .contains("Endpoint prompt:\nkick off the ack flow"));
    assert!(active_prompt
        .prompt()
        .contains("Node instruction reference (daemon-managed):"));
    assert!(active_prompt.prompt().contains("`ack_workflow_turn`"));
    assert!(!active_prompt
        .prompt()
        .contains("Control mailbox (daemon-managed):"));

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

    let routed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let first_completed = routed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == first_run_id)
        .expect("first node run should remain");
    let first_envelope = first_completed
        .turn_envelope()
        .expect("first node run should retain its envelope");
    assert_eq!(
        first_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert!(first_envelope.rendered_prompt().is_none());
    assert!(first_envelope.handoff_payloads_json().is_none());
    assert_eq!(routed.messages().len(), 1);

    let second_active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node prompt should be active")
    });
    assert!(second_active_prompt
        .prompt()
        .contains("Workflow handoff payloads (JSON array):"));
    assert!(second_active_prompt
        .prompt()
        .contains("`ack_workflow_turn`"));

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
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
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
    let second_envelope = second_completed
        .turn_envelope()
        .expect("second node turn envelope should exist");
    assert_eq!(
        second_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert!(completed.messages().is_empty());
}

#[test]
fn local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns() {
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
                output_schema_ref: Some(schema_path.to_string_lossy().to_string()),
                validation_policy: Some(WorkflowOutputValidationPolicy::Warn),
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
                output_schema_ref: None,
                validation_policy: None,
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

    let after_warning = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("updated workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert!(after_warning.failure_events().iter().any(|event| {
        matches!(
            event.kind(),
            crate::session::WorkflowFailureKind::OutputValidationFailed
        ) && event.message().contains("output.message is not valid JSON")
    }));
    let second_active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node should be active")
    });
    assert!(second_active_prompt.prompt().contains("Control mailbox:"));
    assert!(second_active_prompt
        .prompt()
        .contains("output.message is not valid JSON"));
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
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
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

    let active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("first node should be active again")
    });
    assert!(active_prompt.prompt().contains("Control mailbox:"));
    assert!(active_prompt
        .prompt()
        .contains("output.message is not valid JSON"));
    assert!(active_prompt
        .prompt()
        .contains("Treat the control mailbox as authoritative runtime feedback"));
    assert!(active_prompt.prompt().contains("Outgoing edge contracts:"));
    assert!(active_prompt
        .prompt()
        .contains(schema_path.to_string_lossy().as_ref()));
    assert!(!active_prompt
        .prompt()
        .contains("Control mailbox (daemon-managed):"));
}

#[test]
fn local_request_api_resumes_stopped_active_workflow_node_runs() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-resume", "worktree-resume"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let _attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "resume-client".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attachment should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "resume-node");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("resume-flow".to_string()),
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
                prompt: Some("resume prompt".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };

    let cancelled = match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            },
        ))
        .expect("workflow run should stop")
    {
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        cancelled.status(),
        crate::session::WorkflowRunStatus::Stopped
    );
    assert_eq!(
        harness.with_app(|app| {
            app.sessions()
                .get_session(session.id())
                .expect("session should resolve")
                .active_prompt()
                .expect("workflow prompt should be cancelling")
                .status()
        }),
        crate::session::PromptStatus::Cancelling
    );
    harness.with_app_mut(|app| {
        app.finalize_active_prompt_cancellation(session.id(), agent.id(), None)
            .expect("workflow cancellation should finalize");
    });
    assert!(harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .is_none()
    }));
    let stopped_run = harness.with_app(|app| {
        app.sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve after cancellation")
            .clone()
    });
    assert!(stopped_run.failure_events().iter().any(|event| {
        matches!(
            event.kind(),
            crate::session::WorkflowFailureKind::RunStopped
        ) && event
            .message()
            .contains("workflow node run was stopped before validated completion")
    }));

    let resumed = match harness
        .dispatch(LocalDaemonRequest::ResumeWorkflowRun(
            ResumeWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            },
        ))
        .expect("workflow run should resume")
    {
        LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert!(matches!(
        resumed.status(),
        crate::session::WorkflowRunStatus::Waiting
            | crate::session::WorkflowRunStatus::Running
            | crate::session::WorkflowRunStatus::Completed
    ));
    let active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
    });
    if let Some(active_prompt) = active_prompt {
        assert!(active_prompt.prompt().contains("resume prompt"));
    }
    let resumed_run = resumed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_run.node_runs()[0].id())
        .expect("node run should remain");
    assert!(matches!(
        resumed_run.status(),
        crate::session::WorkflowNodeRunStatus::Ready
            | crate::session::WorkflowNodeRunStatus::Running
            | crate::session::WorkflowNodeRunStatus::Completed
    ));
    assert!(resumed_run
        .turn_envelope()
        .and_then(|envelope| envelope.rendered_prompt())
        .is_some());
}

#[test]
fn local_request_api_rejects_workflow_run_when_agent_lacks_required_control_capability() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-control", "worktree-control"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let unsupported_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("unsupported-node".to_string()),
            provider: "dev-invalid-pty".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }))
        .expect("agent spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("control-check".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), unsupported_agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint create should succeed")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("hello".to_string()),
            },
        ))
        .expect_err("workflow invoke should fail when controls are unsupported");
    assert!(matches!(
        error,
        DaemonError::WorkflowNodeControlUnsupported { operation, .. }
            if operation == "ack_workflow_turn"
    ));
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
    let branch_one_agent = harness.spawn_workflow_test_agent(session.id(), "branch-one");
    let branch_two_agent = harness.spawn_workflow_test_agent(session.id(), "branch-two");
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
    assert_eq!(active_prompt_count + queued_prompt_count, 1);
    assert_eq!(active_branch_agents.len(), 1);
    assert_eq!(
        after_entry
            .node_runs()
            .iter()
            .filter(|node_run| {
                node_run.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
            })
            .count(),
        1
    );

    harness.with_app_mut(|app| {
        app.complete_active_prompt(session.id(), active_branch_agents[0], None)
            .unwrap_or_else(|error| {
                panic!("first branch workflow prompt should complete: {error}")
            });
    });
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
    let remaining_active_branch_agents = [branch_one_agent.id(), branch_two_agent.id()]
        .into_iter()
        .filter(|agent_id| {
            session_after_first_branch
                .active_prompt_for_agent(agent_id)
                .is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_active_branch_agents.len(), 1);
    assert_eq!(session_after_first_branch.queued_prompts().len(), 0);

    harness.with_app_mut(|app| {
        app.complete_active_prompt(session.id(), remaining_active_branch_agents[0], None)
            .unwrap_or_else(|error| {
                panic!("second branch workflow prompt should complete: {error}")
            });
    });
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
fn detaching_one_attachment_keeps_the_session_open_for_others() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: first.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), first.id());
    assert_eq!(state.status().to_string(), "created");
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(second.id()).is_ok()));
}

#[test]
fn focusing_another_agent_during_a_prompt_keeps_the_working_run_active() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("default provider launch should succeed")
    });

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: "claude-code".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let _focused_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(spawned.id()),
        )
        .expect("spawned provider launch should succeed")
    });

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focusing default agent should succeed");

    let started = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "keep streaming while focus changes\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt should start");

    match started {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), default_agent.id());
            }
            _ => panic!("expected prompt to start immediately"),
        },
        _ => panic!("unexpected local response"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
        }))
        .expect("focusing spawned agent should succeed");

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(_default_run.id())
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_output = false;
    while Instant::now() < deadline {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        if !records.is_empty() {
            saw_output = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        saw_output,
        "expected background agent output to continue while unfocused"
    );

    harness.with_app_mut(|app| TransportService::pump_active_prompts(app));
    harness.with_app(|app| {
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
        assert_eq!(
            session_state.active_provider_run_id(),
            Some(_default_run.id())
        );
        assert!(
            session_state
                .active_prompt_for_agent(default_agent.id())
                .is_some(),
            "background prompt should remain owned by the original agent while unfocused"
        );
    });
}

#[test]
fn spawning_agent_during_active_prompt_keeps_snapshot_on_working_run() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("provider run should launch")
    });

    harness
        .with_app_mut(|app| {
            app.submit_prompt(
                session.id(),
                attachment.id(),
                Some(default_agent.id()),
                "keep working\n",
                Vec::new(),
            )
        })
        .expect("prompt should start");
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("observer".to_string()),
            provider: "claude-code".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(default_run.id()),
        "snapshots must keep the still-running provider visible for recovery and stream routing"
    );
}

#[test]
fn terminal_output_drain_survives_missing_focused_provider_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.terminal_mut().fan_out_output(
        session.id(),
        "provider-run-stale",
        Some(default_agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"late output\n",
    );

    let records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("draining buffered output should not require an active focused provider run");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"late output\n");
}

#[test]
fn attaching_the_same_client_replaces_its_stale_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_ne!(first.id(), second.id());
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(first.id()).is_err()));
}

#[test]
fn local_request_api_auto_launches_provider_run_for_prompt() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should auto-launch a provider run");

    match response {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            session,
        } => {
            assert_eq!(prompt.status(), crate::session::PromptStatus::Running);
            assert!(session.active_provider_run_id().is_some());
            assert!(session.active_prompt_for_agent(prompt_agent.id()).is_some());
        }
        other => panic!("unexpected local response: {other:?}"),
    }
}

#[test]
fn direct_prompt_completion_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("completion should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.target_agent_id(), prompt_agent.id());
            assert!(completion.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert!(session_state
        .active_prompt_for_agent(prompt_agent.id())
        .is_none());
}

#[test]
fn direct_prompt_cancel_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.target_agent_id(), prompt_agent.id());
            assert!(cancellation.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert_eq!(
        session_state
            .active_prompt_for_agent(prompt_agent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
}

#[test]
fn prompt_idle_fallback_completes_after_recorded_completion_without_response_text() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-completion-fallback",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("provider run should launch");
    let _started = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(default_agent.id()),
            "tool only\n",
            Vec::new(),
        )
        .expect("prompt should start");

    crate::transport::flow_control::mark_prompt_completion_recorded(&mut app, run.id());
    if let Some(state) = app.prompt_activity.get_mut(run.id()) {
        state.last_output_at =
            Some(Instant::now() - app.prompt_idle_timeout - Duration::from_millis(1));
    } else {
        panic!("prompt activity should exist for the active run");
    }

    crate::transport::flow_control::maybe_complete_active_prompt(&mut app, session.id(), run.id())
        .expect("idle fallback should settle the active prompt");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session_state
        .active_prompt_for_agent(default_agent.id())
        .is_none());
}

#[test]
fn local_request_api_rejects_invalid_provider_adapter() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: None,
                adapter_key: "missing-adapter".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect_err("unknown adapters should be rejected");

    match error {
        DaemonError::ProviderAdapterNotFound { adapter_key } => {
            assert_eq!(adapter_key, "missing-adapter")
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_exposes_queue_config_and_notices() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let a = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let b = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-b".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    });

    let first = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: a.id().to_string(),
            target_agent_id: None,
            prompt: "first".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let second = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: b.id().to_string(),
            target_agent_id: None,
            prompt: "second".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");
    let config = harness
        .dispatch(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");

    match first {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            session,
        } => {
            assert!(session.active_prompt().is_some());
        }
        _ => panic!("unexpected first prompt response"),
    }
    match second {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { .. },
            session,
        } => {
            assert_eq!(session.queued_prompts().len(), 1);
        }
        _ => panic!("unexpected second prompt response"),
    }
    match config {
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            assert_eq!(config.version(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected config response"),
    }

    let notices = harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
            },
        ))
        .expect("notice polling should succeed");
    match notices {
        LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
        _ => panic!("unexpected notices response"),
    }

    let state = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed");
    match state {
        LocalDaemonResponse::SessionState { session } => {
            assert_eq!(session.queued_prompts().len(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected state response"),
    }

    let completed = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("complete prompt should succeed");
    match completed {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert!(completion.started_next.is_some())
        }
        _ => panic!("unexpected completion response"),
    }
}

#[test]
fn local_request_api_can_cancel_an_active_prompt() {
    let harness = LocalRouterTestHarness::new();

    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");

    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "first prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "second prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");

    let response = harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should succeed");

    match response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(
                cancellation.prompt.status(),
                crate::session::PromptStatus::Cancelling
            );
            assert!(cancellation.started_next.is_none());
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_runs_shell_command_capability() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-shell".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf capability".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect("shell capability should succeed");

    match response {
        LocalDaemonResponse::ShellCommandCompleted { result } => {
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, "capability");
        }
        _ => panic!("unexpected shell response"),
    }
}

#[test]
fn local_request_api_rejects_shell_command_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-denied-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf denied".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect_err("automation-only attachment should not run shell commands");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_rejects_file_capability_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-file-local-api-denied-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    std::fs::write(worktree_root.join("notes.txt"), "hello").expect("file should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("notes.txt"),
        }))
        .expect_err("automation-only attachment should not read files");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_reads_directory_tree_file_and_git_status() {
    let worktree_root = std::env::temp_dir().join("arroba-capability-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&worktree_root)
        .output()
        .expect("git init should work");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-capability".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let tree = harness
        .dispatch(LocalDaemonRequest::ReadDirectoryTree(
            ReadDirectoryTreeCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: None,
                max_depth: 2,
            },
        ))
        .expect("tree read should succeed");
    let file = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
        }))
        .expect("file read should succeed");
    let edit = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect("file edit should succeed");
    let git = harness
        .dispatch(LocalDaemonRequest::InspectGit(
            InspectGitCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                working_directory: None,
            },
        ))
        .expect("git inspect should succeed");

    match tree {
        LocalDaemonResponse::DirectoryTreeRead { result } => {
            assert!(result
                .entries
                .iter()
                .any(|entry| entry.relative_path == "README.md"));
        }
        _ => panic!("unexpected tree response"),
    }
    match file {
        LocalDaemonResponse::FileRead { result } => assert_eq!(result.contents, "before"),
        _ => panic!("unexpected file response"),
    }
    match edit {
        LocalDaemonResponse::FileEdited { result } => {
            assert_eq!(result.bytes_written, 5);
            assert_eq!(result.old_size, 6);
            assert_eq!(result.new_size, 5);
            assert!(result.changed);
        }
        _ => panic!("unexpected edit response"),
    }
    match git {
        LocalDaemonResponse::GitInspected { result } => assert!(result.status.contains("main")),
        _ => panic!("unexpected git response"),
    }
}

#[test]
fn local_request_api_rejects_conflicting_workspace_write_claims() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-claim-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-workspace-claim".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _claim = harness.with_app_mut(|app| {
        app.workspace_coordinator()
            .acquire_worktree_write_claim(
                session.workspace_id().to_string(),
                worktree_root.display().to_string(),
                "other-session",
                Some("other-attachment".to_string()),
                "file_edit",
            )
            .expect("existing claim should acquire")
    });

    let health = harness
        .dispatch(LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest))
        .expect("health should be available while claim is active");
    match health {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(
                projection
                    .workspace_coordination
                    .active_operation_claims
                    .len(),
                1
            );
        }
        _ => panic!("unexpected health response"),
    }

    let error = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect_err("conflicting write should be rejected");

    match error {
        DaemonError::WorkspaceClaimConflict {
            requested_session_id,
            existing_session_id,
            ..
        } => {
            assert_eq!(requested_session_id, session.id());
            assert_eq!(existing_session_id, "other-session");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_rejects_cross_session_provider_prompt_workspace_claims() {
    let harness = LocalRouterTestHarness::new();
    let session_1 = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("first session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent_1 = harness.spawn_workflow_test_agent(session_1.id(), "claim-1");
    let attachment_1 = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session_1.id().to_string(),
                client_id: "client-provider-claim-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_1.id().to_string(),
            attachment_id: attachment_1.id().to_string(),
            target_agent_id: Some(agent_1.id().to_string()),
            prompt: "first prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start")
    {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { .. } => {}
            _ => panic!("expected first prompt to start"),
        },
        _ => panic!("unexpected local response"),
    }

    let health = harness
        .dispatch(LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest))
        .expect("health should be available while provider prompt is active");
    match health {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(
                projection
                    .workspace_coordination
                    .active_operation_claims
                    .iter()
                    .filter(|claim| claim.operation == "provider_prompt")
                    .count(),
                1
            );
        }
        _ => panic!("unexpected health response"),
    }

    let session_2 = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("second session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent_2 = harness.spawn_workflow_test_agent(session_2.id(), "claim-2");
    let attachment_2 = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session_2.id().to_string(),
                client_id: "client-provider-claim-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_2.id().to_string(),
            attachment_id: attachment_2.id().to_string(),
            target_agent_id: Some(agent_2.id().to_string()),
            prompt: "second prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect_err("cross-session provider prompt should be rejected");
    match error {
        DaemonError::WorkspaceClaimConflict {
            requested_session_id,
            existing_session_id,
            ..
        } => {
            assert_eq!(requested_session_id, session_2.id());
            assert_eq!(existing_session_id, session_1.id());
        }
        other => panic!("unexpected error: {other}"),
    }

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_1.id().to_string(),
        }))
        .expect("first prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    assert!(harness.with_app(|app| {
        app.workspace_coordinator()
            .active_claims()
            .into_iter()
            .all(|claim| claim.operation != "provider_prompt")
    }));
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
    harness.with_app_mut(|app| {
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
        .expect("interactive provider run should launch");
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
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
fn local_request_api_returns_structured_screenshot_unavailable_result() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", std::env::temp_dir().display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-screenshot".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::CaptureScreenshot(
            CaptureScreenshotCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("screenshot request should succeed with unavailable result");
    std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

    match response {
        LocalDaemonResponse::ScreenshotCaptured { result } => {
            assert_eq!(
                result.status,
                crate::capability::ScreenshotStatus::Unavailable
            );
        }
        _ => panic!("unexpected screenshot response"),
    }
}

#[test]
fn local_request_api_stores_transferred_file_under_session_artifacts() {
    let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    let source = worktree_root.join("artifact.txt");
    std::fs::write(&source, "artifact").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-transfer".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::StoreTransferredFile(
            StoreTransferredFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                source_path: source,
                display_name: None,
            },
        ))
        .expect("transfer should succeed");

    match response {
        LocalDaemonResponse::FileTransferred { result } => {
            assert!(result
                .stored_path
                .to_string_lossy()
                .contains("arroba-session-artifacts"));
            assert_eq!(result.bytes, 8);
        }
        _ => panic!("unexpected transfer response"),
    }
}
