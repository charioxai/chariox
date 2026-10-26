//! MP-08/MP-09/MP-10/MP-11 A04: a slice's private relay connector projects a
//! completion on its own single-threaded runtime. The harvest of the worker's
//! git observations must leave that runtime free to poll the relay, or the
//! answer never arrives and the home waits out the harvest timeout.
use super::*;
use crate::history::HistoryEventKind;
use crate::transport::relay_client::ConnectedWorkerRig;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteGitObservation};

#[test]
fn completion_harvest_answers_on_a_current_thread_connector_runtime() {
    std::thread::Builder::new()
        .name("harvest-current-thread".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(harvest_over_current_thread_runtime())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn harvest_over_current_thread_runtime() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://fixture.invalid".into());
    config.relay_token = Some("home-token".into());
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-harvest-current-thread",
            "worktree-harvest-current-thread",
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-harvest-current-thread",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-harvest-current-thread",
        attachment.id(),
        agent.id(),
        "remote prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap()
    else {
        panic!("remote prompt should start")
    };
    let (binding, provider_run_id, _output, completion) =
        actual_worker_output_then_completion(session.id(), agent.id(), prompt.id());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let mut rig =
        ConnectedWorkerRig::connect(&runtime.owned.relay_state, &config, "ws://fixture.invalid")
            .await;
    runtime
        .owned
        .agent_store
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: rig.worker_kernel_id().to_string(),
                active_worker_provider_run_id: Some(provider_run_id.clone()),
                ..binding
            },
        )
        .unwrap();
    let observation = RemoteGitObservation {
        kind: HistoryEventKind::GitCommitDetected,
        content: Some("slice commit".into()),
        metadata: Default::default(),
        context: crate::history::HistoryEventTurnContext {
            session_id: Some(session.id().to_string()),
            agent_id: Some(agent.id().to_string()),
            ..Default::default()
        },
        candidate_agent_ids: vec![agent.id().to_string()],
        candidate_prompt_ids: vec![],
        candidate_turn_ids: vec![],
        attribution_confidence: None,
    };
    let authority =
        crate::runtime::relay_peer_authority::test_projection_authority(rig.worker_kernel_id());
    let projected = runtime.project_remote_runtime_projection_event(authority, completion);
    let worker = rig.answer(|_| RelayPeerResponse::LeasedGitObserved {
        provider_run_id: provider_run_id.clone(),
        git_observations: vec![observation],
        workspace_live_sync_change: None,
    });
    let (projected, request) = tokio::join!(projected, worker);
    projected.expect("the completion projects");
    assert!(matches!(
        request,
        RelayPeerRequest::ObserveLeasedGitAfter { provider_run_id: run, .. } if run == provider_run_id
    ));
    let events = runtime
        .owned
        .operational_history_store
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().to_string()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.content.as_deref() == Some("slice commit")),
        "the worker's git observation reaches the home history"
    );
}
