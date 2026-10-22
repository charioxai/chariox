// MP-08 / MP-10 / MP-11: a worker run number must never own a local turn.
use super::*;
use crate::app::{ActivePromptState, ActiveTurnState, DaemonApp};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
    RuntimeProviderRun,
};

#[test]
fn worker_completion_preserves_a_local_turn_with_the_same_run_number() {
    assert_home_run_identity(false);
}

#[test]
fn native_worker_prompt_uses_its_home_projection_instead_of_a_local_run() {
    assert_home_run_identity(true);
}

fn assert_home_run_identity(native: bool) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let workspace = std::env::current_dir().unwrap();
    let workspace = workspace.to_str().unwrap();
    let (local_session, local_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            workspace, workspace,
        ))
        .unwrap();
    let local_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            local_session.id(),
            "local-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let local_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                local_session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(local_agent.id()),
        )
        .unwrap();
    let local_prompt = PromptQueueItem::new(
        "local-prompt",
        local_attachment.id(),
        local_agent.id(),
        "local work remains active",
        PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(local_session.id(), local_prompt, false)
        .unwrap();
    crate::transport::flow_control::note_prompt_started(&mut app, local_run.id());
    let local_turn = app.active_turn_store().get(local_run.id()).unwrap();

    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            workspace, workspace,
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "remote-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: Some(local_run.id().into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    assert_ne!(session.id(), local_session.id());
    assert_ne!(agent.id(), local_agent.id());
    let projected_id = crate::provider::projected_leased_provider_run_id("leased", local_run.id());
    let request = LaunchProviderRequest::new(
        "worker-session",
        "dev-stub",
        "dev-stub",
        "default",
        "default",
    )
    .with_agent_id("worker-agent")
    .with_client_interface(if native {
        ProviderClientInterface::NativeTui
    } else {
        ProviderClientInterface::Chariox
    });
    let mut worker_run = RuntimeProviderRun::new(
        local_run.id(),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "worker".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    worker_run.mark_running();
    let mut remote_prompt_id = None;
    if !native {
        app.prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "remote-prompt",
                attachment.id(),
                agent.id(),
                "remote work",
                PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
        let remote_prompt = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .expect("the remote prompt must be admitted before completion");
        assert_eq!(remote_prompt.status(), PromptStatus::Running);
        remote_prompt_id = Some(remote_prompt.id().to_string());
        app.prompt_activity.write().insert(
            projected_id.clone(),
            ActivePromptState {
                last_output_at: None,
                saw_response_content: false,
                completion_recorded: false,
                settlement_requested: false,
                active_tool_ids: Default::default(),
            },
        );
        app.active_turn_store().start(ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            remote_prompt.id().to_string(),
            projected_id.clone(),
        ));
    }
    let outcome = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel-1"),
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: session.id().into(),
                home_agent_id: agent.id().into(),
                provider_run_id: local_run.id().into(),
                provider_run: Some(worker_run),
                prompts: if native {
                    vec![RelayProjectedPrompt {
                        prompt_id: "worker-prompt".into(),
                        text: "native worker work".into(),
                    }]
                } else {
                    vec![]
                },
                output_chunks: vec![RelayProjectedOutputChunk {
                    kind: TerminalOutputKind::ProviderOutput,
                    merge_key: Some("worker-output".into()),
                    bytes: b"worker answer".to_vec(),
                }],
                notices: vec!["worker notice".into()],
                completions: if native {
                    vec![]
                } else {
                    vec![RelayProjectedCompletion {
                        message_id: "worker-completion".into(),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                        home_prompt_id: remote_prompt_id.clone(),
                        provider_termination: None,
                    }]
                },
            },
        )
        .unwrap();
    assert!(outcome.accepted);
    let retained = app
        .active_turn_store()
        .get(local_run.id())
        .expect("worker completion cleared the unrelated local turn");
    assert_eq!(retained.prompt_id, local_turn.prompt_id);
    assert_eq!(retained.trace_id, local_turn.trace_id);
    assert!(app.prompt_activity.read().contains_key(local_run.id()));
    if native {
        let remote_turn = app
            .active_turn_store()
            .get(&projected_id)
            .expect("native worker prompt must own its projected run");
        assert_eq!(remote_turn.session_id, session.id());
        assert_eq!(remote_turn.agent_id, agent.id());
    } else {
        assert!(app.active_turn_store().get(&projected_id).is_none());
        assert!(!app.prompt_activity.read().contains_key(&projected_id));
        assert!(app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .is_none());
    }
    assert!(app
        .terminal()
        .output_records()
        .iter()
        .filter(|r| r.session_id == session.id())
        .all(|r| r.provider_run_id == projected_id));
}
