use super::super::command_executor::AgentRuntimeCommandExecutor;
use super::super::command_lane::run_agent_command_lane;
use super::*;
use crate::local::KernelConnectionClass;
use crate::runtime::command::KernelCommand;
use tokio::sync::mpsc;

#[tokio::test]
async fn kernel_access_revocation_refuses_an_already_queued_agent_mutation() {
    let worktree = TestWorktree::new("access-agent-queue");
    let mut daemon = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut daemon)
        .attach(AttachRequest::new(
            session.id(),
            "queue-holder",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    launch_dev_stub_provider(&mut daemon, session.id(), agent.id(), "sonnet");
    for prompt in ["active", "queued"] {
        let item = PromptQueueItem::new(
            daemon.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            prompt,
            PromptStatus::Queued,
        );
        daemon
            .prompt_owner_submit_prepared_prompt(session.id(), item, false)
            .unwrap();
    }
    let snapshot = crate::app::KernelSessionReadService::new(&daemon)
        .session_snapshot(session.id())
        .unwrap();
    let queued_id = snapshot.queued_prompts_for_agent(agent.id()).unwrap()[0]
        .id()
        .to_owned();
    let projection = daemon.session_state_projection_store();
    projection.update(snapshot.clone());
    let agent_projection = daemon.agent_runtime_projection_store();
    agent_projection.update_session(&snapshot);
    let prompts = daemon.prompt_state_owner();
    let app = Arc::new(Mutex::new(daemon));
    let state = owned_runtime_state(&app).await;
    let grant = state.insert_access_grant_for_test(session.id());
    let runtime = AgentRuntime::new(
        state.clone(),
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        projection,
        agent_projection,
        prompts,
        Default::default(),
    );
    let request = CancelQueuedPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: agent.id().into(),
        prompt_id: queued_id.clone(),
    };
    let local_request = LocalDaemonRequest::CancelQueuedPrompt(request.clone());
    state
        .authorize_external_request(&grant, &local_request)
        .unwrap();
    let mut command =
        KernelCommand::from_local_request("external-queued-cancel", None, None, &local_request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();

    // Install a real lane without its consumer. Dispatch must enqueue before
    // revocation; only then can the consumer start and authorize execution.
    let (tx, rx) = mpsc::channel(super::super::AGENT_COMMAND_QUEUE_LIMIT);
    runtime
        .lanes
        .lock()
        .await
        .insert(agent.id().into(), tx.clone());
    let queued_runtime = runtime.clone();
    let queued_request = request.clone();
    let pending = tokio::spawn(async move {
        queued_runtime
            .dispatch_prompt_cancel_queued(&command, queued_request)
            .await
    });
    timeout(Duration::from_secs(2), async {
        while tx.capacity() == super::super::AGENT_COMMAND_QUEUE_LIMIT {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    let executor = AgentRuntimeCommandExecutor::new(
        runtime
            .store
            .prompt_command_service(runtime.provider_runtime_lanes.clone()),
        runtime.session_projection.clone(),
        runtime.agent_runtime_projection.clone(),
        runtime.prompt_id_allocator.clone(),
    );
    let consumer = tokio::spawn(run_agent_command_lane(
        executor,
        state.clone(),
        agent.id().into(),
        rx,
    ));
    let error = timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("grant revoked or expired"),
        "{error}"
    );
    let after = state.session_snapshot(session.id()).await.unwrap();
    assert!(after
        .queued_prompts_for_agent(agent.id())
        .unwrap()
        .iter()
        .any(|p| p.id() == queued_id && p.status() == PromptStatus::Queued));

    // The queued mutation was valid, and ordinary terminal traffic still works.
    let terminal =
        KernelCommand::from_local_request("terminal-queued-cancel", None, None, &local_request);
    let result = runtime
        .dispatch_prompt_cancel_queued(&terminal, request)
        .await
        .unwrap();
    assert!(matches!(
        result,
        LocalDaemonResponse::QueuedPromptCancelled { .. }
    ));
    consumer.abort();
    let _ = consumer.await;
}

#[tokio::test]
async fn kernel_access_revocation_refuses_a_cold_prompt_waiting_for_app_lock() {
    let worktree = TestWorktree::new("access-cold-prompt");
    let mut daemon = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let agent = crate::app::KernelSessionService::new(&mut daemon)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub"))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut daemon)
        .attach(AttachRequest::new(
            session.id(),
            "cold-holder",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let projection = daemon.session_state_projection_store();
    let agent_projection = daemon.agent_runtime_projection_store();
    let prompts = daemon.prompt_state_owner();
    let app = Arc::new(Mutex::new(daemon));
    let state = owned_runtime_state(&app).await;
    let grant = state.insert_access_grant_for_test(session.id());
    let runtime = AgentRuntime::new(
        state.clone(),
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        projection,
        agent_projection,
        prompts,
        Default::default(),
    );
    let request = SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: Some(agent.id().into()),
        prompt: "revoked cold prompt".into(),
        attachments: vec![],
    };
    let local_request = LocalDaemonRequest::SubmitPrompt(request.clone());
    let mut command =
        KernelCommand::from_local_request("external-cold-submit", None, None, &local_request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let (tx, rx) = mpsc::channel(super::super::AGENT_COMMAND_QUEUE_LIMIT);
    runtime
        .lanes
        .lock()
        .await
        .insert(agent.id().into(), tx.clone());
    let executor = AgentRuntimeCommandExecutor::new(
        runtime
            .store
            .prompt_command_service(runtime.provider_runtime_lanes.clone()),
        runtime.session_projection.clone(),
        runtime.agent_runtime_projection.clone(),
        runtime.prompt_id_allocator.clone(),
    );
    // MP-08 / MP-10 / MP-11: the lane must retain only the selected handler.
    fn future_size<F: std::future::Future>(_: impl FnOnce() -> F) -> usize {
        std::mem::size_of::<F>()
    }
    assert!(
        future_size(
            || executor.execute(super::super::AgentCommand::SubmitPrompt {
                request: request.clone(),
                trace_id: "stack-budget".into(),
                operation_id: "stack-budget".into(),
                operation_fingerprint: "stack-budget".into(),
                response_mode: super::super::command_lane::PromptSubmitResponseMode::Full,
            })
        ) <= 1_024,
        "agent lane handler must fit the ordinary stack budget"
    );
    let locked_app = app.lock().await;
    let submission = runtime.dispatch_prompt_submit(&command, request.clone());
    tokio::pin!(submission);
    assert!(futures_util::poll!(&mut submission).is_pending());
    assert_eq!(tx.capacity(), super::super::AGENT_COMMAND_QUEUE_LIMIT - 1);
    let lane = run_agent_command_lane(executor, state.clone(), agent.id().into(), rx);
    tokio::pin!(lane);
    // Polling the lane consumes the command, passes authorization and reaches
    // the app mutex. There is no scheduling delay or timer in this interleaving.
    assert!(futures_util::poll!(&mut lane).is_pending());
    assert_eq!(tx.capacity(), super::super::AGENT_COMMAND_QUEUE_LIMIT);
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(locked_app);
    let result = timeout(Duration::from_secs(2), async {
        tokio::select! { result = &mut submission => result, _ = &mut lane => panic!("lane closed") }
    }).await.unwrap();
    let snapshot = state.session_snapshot(session.id()).await.unwrap();
    assert!(
        snapshot.active_prompt_for_agent(agent.id()).is_none(),
        "revoked prompt was admitted"
    );
    assert!(snapshot
        .queued_prompts_for_agent(agent.id())
        .map(|prompts| prompts.is_empty())
        .unwrap_or(true));
    assert!(
        app.lock()
            .await
            .providers()
            .get_run_for_agent(session.id(), agent.id())
            .is_none(),
        "revoked prompt launched a provider"
    );
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("grant revoked or expired"),
        "{error}"
    );

    // The same cold launch remains available to the ordinary terminal caller.
    let terminal =
        KernelCommand::from_local_request("terminal-cold-submit", None, None, &local_request);
    let terminal_submission = runtime.dispatch_prompt_submit(&terminal, request);
    tokio::pin!(terminal_submission);
    let response = timeout(Duration::from_secs(2), async {
        tokio::select! { result = &mut terminal_submission => result, _ = &mut lane => panic!("lane closed") }
    }).await.unwrap().unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::PromptSubmitted { .. }
    ));
    assert!(app
        .lock()
        .await
        .providers()
        .get_run_for_agent(session.id(), agent.id())
        .is_some());
}

mod remote_controls;
