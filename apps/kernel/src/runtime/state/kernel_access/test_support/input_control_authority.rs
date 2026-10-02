use super::session_authority::{external_command, runtime};
use super::worker_spy::WorkerSpy;
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::runtime::router::CommandRouter;
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::Mutex;
use tokio::time::timeout;

macro_rules! meta_regression {
    ($name:ident, $abort:expr, $discovery:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            revoked_meta_control($abort, $discovery).await;
        }
    };
}
meta_regression!(kernel_access_meta_pause_rechecks_app_wait, false, false);
meta_regression!(kernel_access_meta_abort_rechecks_app_wait, true, false);
meta_regression!(
    kernel_access_meta_pause_rechecks_discovery_wait,
    false,
    true
);
meta_regression!(kernel_access_meta_abort_rechecks_discovery_wait, true, true);

fn remote_binding(worker: &WorkerSpy) -> crate::agent::RemoteAgentBinding {
    crate::agent::RemoteAgentBinding {
        worker_kernel_id: worker.id.clone(),
        worker_machine_id: "fixture-machine".into(),
        execution_lease_id: "lease".into(),
        leased_agent_id: "leased-agent".into(),
        active_worker_provider_run_id: Some("worker-run".into()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    }
}

fn provider_run(session: &str, agent: &str, worktree: &std::path::Path) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new(session, "dev-stub", "dev-stub", "default", "default")
        .with_agent_id(agent);
    let mut run = RuntimeProviderRun::new(
        "input-control-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run
}

async fn revoked_meta_control(abort: bool, discovery: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-meta-control");
    let worker = WorkerSpy::new(discovery);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("meta-control-fixture".into());
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, meta) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    app.agents_mut()
        .activate_agent_meta_mode(meta.id(), None)
        .unwrap();
    app.agents_mut()
        .bind_remote_execution(meta.id(), remote_binding(&worker))
        .unwrap();
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), meta.id(), "Hold task")
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "meta-control-source",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "active-meta-control",
        attachment.id(),
        meta.id(),
        "Active work",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap();
    let run = provider_run(session.id(), meta.id(), worktree.path());
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let grant = state.insert_access_grant_for_test(session.id());
    let before = state.owned.session_store.get_session(session.id()).unwrap();
    let active = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&before, meta.id())
        .unwrap();
    let request = if abort {
        LocalDaemonRequest::AbortMetaagentTask(crate::local::AbortMetaagentTaskRequest {
            session_id: session.id().into(),
            metaagent_id: meta.id().into(),
            reason: None,
        })
    } else {
        LocalDaemonRequest::PauseMetaagentTask(crate::local::PauseMetaagentTaskRequest {
            session_id: session.id().into(),
            metaagent_id: meta.id().into(),
        })
    };
    let guard = if discovery {
        None
    } else {
        Some(app.lock().await)
    };
    let result = if discovery {
        let pending = tokio::spawn({
            let router = router.clone();
            let request = request.clone();
            let command = external_command(&request, &grant);
            async move { router.dispatch(command, request).await }
        });
        timeout(Duration::from_secs(3), worker.discovery_started.notified())
            .await
            .unwrap();
        state
            .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
            .unwrap();
        worker.release_discovery.notify_one();
        timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap()
    } else {
        let pending = router.dispatch(external_command(&request, &grant), request.clone());
        tokio::pin!(pending);
        // Poll through admission and the real priority executor to its app wait.
        tokio::select! { biased; result = &mut pending => panic!("control did not wait: {}", result.is_ok()), _ = tokio::task::yield_now() => {} }
        state
            .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
            .unwrap();
        drop(guard);
        timeout(Duration::from_secs(3), pending).await.unwrap()
    };
    let requests = worker.requests.load(Ordering::SeqCst);
    let after = state.owned.session_store.get_session(session.id()).unwrap();
    let remaining = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&after, meta.id())
        .unwrap();
    // The task transition happened before revocation; cancellation must still stop.
    assert_eq!(requests, 0, "revoked Meta control reached worker");
    assert_eq!(
        remaining, active,
        "revoked Meta control changed active prompt"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    let attachment = state
        .ensure_metaagent_task_attachment(session.id(), &meta)
        .unwrap();
    state
        .cancel_agent_prompt(session.id(), meta.id(), &attachment)
        .await
        .unwrap();
    assert_eq!(worker.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn kernel_access_terminal_input_rechecks_local_app_wait() {
    let worktree = crate::test_support::TestWorktree::new("access-input-local");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "input-terminal",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = provider_run(session.id(), agent.id(), worktree.path());
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.pty_mut()
        .spawn(crate::pty::PtySpawnRequest {
            process_key: run.id().into(),
            provider_run_id: run.id().into(),
            program: "/bin/cat".into(),
            args: vec![],
            env: Default::default(),
            env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            cols: 80,
            rows: 24,
        })
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(tokio::sync::Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let session_runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(session.id());
    let request = LocalDaemonRequest::SendTerminalInput(crate::local::SendTerminalInputRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        provider_run_id: Some(run.id().into()),
        data_base64: "aGVsbG8K".into(),
    });
    let guard = app.lock().await;
    let pending = tokio::spawn({
        let runtime = session_runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_session_command(command, request).await }
    });
    timeout(Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    let result = timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    let inputs = app.lock().await.terminal().input_records().len();
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-input-control",
        None,
        None,
        &request,
    );
    let control = session_runtime
        .dispatch_session_command(command, request)
        .await;
    let control_inputs = app.lock().await.terminal().input_records().len();
    app.lock().await.pty_mut().remove_process(run.id()).unwrap();
    assert_eq!(inputs, 0, "revoked input was recorded/enqueued");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    control.unwrap();
    assert_eq!(control_inputs, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_terminal_input_rechecks_remote_discovery() {
    let worktree = crate::test_support::TestWorktree::new("access-input-remote");
    let worker = WorkerSpy::new(true);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("input-fixture".into());
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "input-terminal",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.agents_mut()
        .bind_remote_execution(agent.id(), remote_binding(&worker))
        .unwrap();
    let run = provider_run(session.id(), agent.id(), worktree.path());
    app.update_provider_run_projection(run.clone());
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let session_runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(session.id());
    let request = LocalDaemonRequest::SendTerminalInput(crate::local::SendTerminalInputRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        provider_run_id: Some(run.id().into()),
        data_base64: "aGVsbG8K".into(),
    });
    let pending = tokio::spawn({
        let runtime = session_runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_session_command(command, request).await }
    });
    timeout(Duration::from_secs(3), worker.discovery_started.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked terminal input reached worker"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-input-control",
        None,
        None,
        &request,
    );
    session_runtime
        .dispatch_session_command(command, request)
        .await
        .unwrap();
    assert_eq!(worker.requests.load(Ordering::SeqCst), 1);
}
