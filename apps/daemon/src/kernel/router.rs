use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::agent_actor::{AgentActor, AgentRuntime};
use crate::kernel::capability_executor::execute_capability_request;
use crate::kernel::command::{KernelCommand, KernelCommandPriority};
use crate::kernel::projection::DaemonHealthProjection;
use crate::kernel::session_actor::{FocusedAgentProjection, SessionActor, SessionRuntime};
use crate::local::provider_requests::{
    launch_provider_request_from_local, load_provider_catalog, logout_provider_response,
    provider_auth_status_response, provider_command_catalogs_response,
    start_provider_login_response,
};
use crate::local::{
    GetSessionHistoryRequest, LaunchProviderRunRequest, ListProviderProcessesRequest,
    LocalDaemonRequest, LocalDaemonResponse, RelayStatus, TeardownProviderProcessesRequest,
};
use crate::provider::ProviderRunOperationLanes;
use crate::session_history_page::paginate_session_history;

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
struct InteractiveCommandEnvelope {
    command: KernelCommand,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct CommandRouter {
    app: Arc<Mutex<DaemonApp>>,
    interactive_tx: mpsc::Sender<InteractiveCommandEnvelope>,
    agent_runtime: AgentRuntime,
    session_runtime: SessionRuntime,
}

impl CommandRouter {
    #[cfg(test)]
    pub(crate) fn with_interactive_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
    ) -> Self {
        Self::with_interactive_capacity_and_provider_lanes(
            app,
            interactive_capacity,
            ProviderRunOperationLanes::default(),
        )
    }

    #[cfg(test)]
    fn with_interactive_and_session_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        session_capacity: usize,
    ) -> Self {
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        let provider_runtime_lanes = ProviderRunOperationLanes::default();
        let focus_projection = FocusedAgentProjection::default();
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes,
            focus_projection.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            session_capacity,
            focus_projection,
        );
        tokio::spawn(run_interactive_command_lane(
            Arc::clone(&app),
            interactive_rx,
        ));
        Self {
            app,
            interactive_tx,
            agent_runtime,
            session_runtime,
        }
    }

    pub(crate) fn with_interactive_capacity_and_provider_lanes(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        let focus_projection = FocusedAgentProjection::default();
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            crate::kernel::session_actor::SESSION_COMMAND_QUEUE_LIMIT,
            focus_projection,
        );
        tokio::spawn(run_interactive_command_lane(
            Arc::clone(&app),
            interactive_rx,
        ));
        Self {
            app,
            interactive_tx,
            agent_runtime,
            session_runtime,
        }
    }

    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if matches!(request, LocalDaemonRequest::GetDaemonHealth(_)) {
            return Ok(LocalDaemonResponse::DaemonHealth {
                projection: self.daemon_health_projection(0).await,
            });
        }

        match command.priority {
            KernelCommandPriority::Interactive => self.dispatch_interactive(command, request).await,
            KernelCommandPriority::Normal | KernelCommandPriority::Background => {
                execute_local_request_with_async_boundaries(&self.app, request).await
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn daemon_health_projection(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjection {
        DaemonHealthProjection::new(
            last_event_id,
            self.session_runtime.queue_snapshots().await,
            self.agent_runtime.queue_snapshots().await,
        )
    }

    async fn dispatch_interactive(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if SessionActor::is_session_interactive_command(&request) {
            return self
                .session_runtime
                .dispatch_session_command(command, request)
                .await;
        }

        match request {
            LocalDaemonRequest::SubmitPrompt(request) => {
                return self
                    .agent_runtime
                    .dispatch_prompt_submit(&command, request)
                    .await;
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                return self
                    .agent_runtime
                    .dispatch_prompt_cancel(&command, request)
                    .await;
            }
            request => {
                let (result_tx, result_rx) = oneshot::channel();
                self.interactive_tx
                    .try_send(InteractiveCommandEnvelope {
                        command,
                        request,
                        result_tx,
                    })
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "enqueue interactive kernel command",
                        message: format!("interactive command lane overloaded: {error}"),
                    })?;
                return result_rx
                    .await
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "await interactive kernel command",
                        message: error.to_string(),
                    })?;
            }
        }
    }
}

async fn run_interactive_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    mut rx: mpsc::Receiver<InteractiveCommandEnvelope>,
) {
    while let Some(envelope) = rx.recv().await {
        crate::logging::info_with_fields(
            "daemon.kernel_router",
            "interactive kernel command dispatched",
            serde_json::json!({
                "command_id": envelope.command.command_id,
                "command_type": envelope.command.command_type,
                "correlation_id": envelope.command.correlation_id,
                "session_id": envelope.command.session_id,
                "attachment_id": envelope.command.attachment_id,
                "agent_id": envelope.command.agent_id,
            }),
        );
        let result = execute_interactive_request(&app, envelope.request).await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn execute_interactive_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    if let Some(result) = SessionActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    if let Some(result) = AgentActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    app.handle_local_request(request)
}

pub(crate) async fn execute_local_request_with_async_boundaries(
    app: &Arc<Mutex<DaemonApp>>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::RelayStatus(_) => {
            let (config, relay_state) = {
                let app = app.lock().await;
                (app.config().clone(), app.relay_client_state())
            };
            let connected = relay_state.read().await.connected();
            Ok(LocalDaemonResponse::RelayStatus {
                status: RelayStatus {
                    configured: config.relay_url.is_some() && config.relay_token.is_some(),
                    connected,
                    relay_url: config.relay_url,
                    relay_token_configured: config.relay_token.is_some(),
                    daemon_id: config.daemon_id,
                    machine_id: config.host_machine_id,
                    machine_alias: config.host_machine_alias,
                },
            })
        }
        LocalDaemonRequest::ListRemoteMachines(_) => {
            let config = {
                let app = app.lock().await;
                app.config().clone()
            };
            let machines = crate::transport::relay_discovery::list_live_machines(&config).await?;
            let machines = crate::local::provider_requests::remote_machine_records(
                machines,
                &config.host_machine_id,
            );
            Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
        }
        LocalDaemonRequest::ListRemoteMachineKernels(request) => {
            let config = {
                let app = app.lock().await;
                app.config().clone()
            };
            let machine_ref =
                crate::local::provider_requests::resolve_registered_or_raw_machine_ref(
                    &request.machine_ref,
                );
            let kernels = crate::transport::relay_discovery::list_live_kernels_for_machine(
                &config,
                &machine_ref,
            )
            .await?;
            Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
                machine_ref,
                kernels,
            })
        }
        LocalDaemonRequest::GetSessionHistory(request) => {
            execute_session_history_request(app, request).await
        }
        LocalDaemonRequest::LaunchProviderRun(request) => {
            execute_launch_provider_run_request(app, request).await
        }
        LocalDaemonRequest::GetProviderCatalog(_) => execute_provider_catalog_request(app).await,
        LocalDaemonRequest::GetProviderCommandCatalogs(_) => provider_command_catalogs_response(),
        LocalDaemonRequest::GetProviderAuthStatus(request) => {
            tokio::task::spawn_blocking(move || provider_auth_status_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "get provider auth status",
                    message: error.to_string(),
                })?
        }
        LocalDaemonRequest::StartProviderLogin(request) => {
            tokio::task::spawn_blocking(move || start_provider_login_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "start provider login",
                    message: error.to_string(),
                })?
        }
        LocalDaemonRequest::LogoutProvider(request) => {
            let response = tokio::task::spawn_blocking(move || logout_provider_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "logout provider",
                    message: error.to_string(),
                })??;
            let mut app = app.lock().await;
            app.invalidate_provider_catalog_cache();
            Ok(response)
        }
        LocalDaemonRequest::ListProviderProcesses(request) => {
            execute_list_provider_processes_request(app, request).await
        }
        LocalDaemonRequest::TeardownProviderProcesses(request) => {
            execute_teardown_provider_processes_request(app, request).await
        }
        request if is_capability_request(&request) => execute_capability_request(app, request)
            .await
            .unwrap_or_else(|| {
                Err(DaemonError::LocalTransport {
                    operation: "route capability request",
                    message: "capability request was not handled by executor".to_string(),
                })
            }),
        request => {
            if is_blocking_local_request(&request) {
                let app = Arc::clone(app);
                let handle = tokio::runtime::Handle::current();
                return tokio::task::spawn_blocking(move || {
                    handle.block_on(async move {
                        let mut app = app.lock().await;
                        app.handle_local_request(request)
                    })
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "run blocking kernel request",
                    message: error.to_string(),
                })?;
            }
            let mut app = app.lock().await;
            app.handle_local_request(request)
        }
    }
}

fn is_capability_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)
    )
}

async fn execute_provider_catalog_request(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = {
        let app = app.lock().await;
        if let Some(catalog) = app.cached_provider_catalog() {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }
        app.config().clone()
    };

    let catalog = tokio::task::spawn_blocking(move || load_provider_catalog(config))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "load provider catalog",
            message: error.to_string(),
        })??;
    let mut app = app.lock().await;
    app.cache_provider_catalog(catalog.clone());
    Ok(LocalDaemonResponse::ProviderCatalog { catalog })
}

async fn execute_launch_provider_run_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LaunchProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (started, runtime_init_delay_ms) = {
        let mut app = app.lock().await;
        let launch_request = launch_provider_request_from_local(&app, request);
        (
            app.start_provider_launch(launch_request)?,
            app.config().provider_runtime_init_delay_ms,
        )
    };
    let accepted = started.run.clone();
    let app = Arc::clone(app);
    tokio::spawn(async move {
        if runtime_init_delay_ms > 0 {
            sleep(Duration::from_millis(runtime_init_delay_ms)).await;
        }
        let run = started.run.clone();
        let binding = tokio::task::spawn_blocking(move || {
            DaemonApp::initialize_provider_runtime_binding(&run)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "initialize provider runtime",
            message: error.to_string(),
        });

        match binding {
            Ok(Ok(binding)) => {
                let mut app = app.lock().await;
                if let Err(error) = app.finish_provider_launch(&started, binding) {
                    app.fail_provider_launch(&started, &error);
                }
            }
            Ok(Err(error)) | Err(error) => {
                let mut app = app.lock().await;
                app.fail_provider_launch(&started, &error);
            }
        }
    });
    Ok(LocalDaemonResponse::ProviderRunLaunchAccepted {
        provider_run: accepted,
    })
}

async fn execute_list_provider_processes_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (processes, delay_ms) = {
        let app = app.lock().await;
        (
            app.list_provider_processes(request.provider.as_deref())?,
            app.config().provider_process_list_delay_ms,
        )
    };
    if delay_ms > 0 {
        sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(LocalDaemonResponse::ProviderProcessesListed { processes })
}

async fn execute_teardown_provider_processes_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: TeardownProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    let processes = app.teardown_provider_processes(request.provider.as_deref())?;
    Ok(LocalDaemonResponse::ProviderProcessesTornDown { processes })
}

async fn execute_session_history_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: GetSessionHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (history, session) = {
        let app = app.lock().await;
        let session = app.sessions().get_session(&request.session_id)?;
        (app.history_store(), session)
    };

    tokio::task::spawn_blocking(move || {
        let mut entries = history.load(&session)?;
        if let Some(agent_id) = request.agent_id.as_deref() {
            entries.retain(|entry| {
                entry.agent_id.is_none() || entry.agent_id.as_deref() == Some(agent_id)
            });
        }
        let page = paginate_session_history(
            &entries,
            request.round_count,
            request.max_chars,
            request.before_entry_index,
            request.before_entry_char_offset,
        );
        Ok(LocalDaemonResponse::SessionHistory {
            entries: page.entries,
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history",
        message: error.to_string(),
    })?
}

fn is_blocking_local_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)
            | LocalDaemonRequest::GetProviderAuthStatus(_)
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::LogoutProvider(_)
            | LocalDaemonRequest::GetSessionHistory(_)
            | LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::attachment::ClientCapabilityLevel;
    use crate::kernel::command::KernelCommand;
    use crate::kernel::router::CommandRouter;
    use crate::local::{
        AttachToSessionRequest, EndSessionRequest, FocusAgentRequest, GetDaemonHealthRequest,
        LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse, SpawnAgentRequest,
        SubmitPromptRequest,
    };
    use crate::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use crate::{DaemonApp, DaemonConfig};

    #[tokio::test]
    async fn routes_interactive_commands_through_bounded_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "cli-1".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        });
        let command = KernelCommand::from_local_request("cmd-1", None, None, &request);

        let response = router
            .dispatch(command, request)
            .await
            .expect("command should run");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionAttached { .. }
        ));
    }

    #[tokio::test]
    async fn rejects_session_commands_when_bounded_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_and_session_capacity(Arc::clone(&app), 1, 1);
        let app_guard = app.lock().await;

        let first_request = attach_request(&session_id, "cli-1");
        let first_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-1", "session.attach", first_request)
            .await
            .expect("first command should enter the session lane");

        let mut first_command_is_running = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 1)
            {
                first_command_is_running = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            first_command_is_running,
            "first session command should be running before filling the queue"
        );

        let queued_request = attach_request(&session_id, "queued-cli");
        let queued_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-queued", "session.attach", queued_request)
            .await
            .expect("queued command should fill the session lane");

        let mut session_lane_is_full = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 0)
            {
                session_lane_is_full = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            session_lane_is_full,
            "session command queue should be full before overflow dispatch"
        );

        let third_request = attach_request(&session_id, "cli-overflow");
        let third_command =
            KernelCommand::from_local_request("cmd-overflow", None, None, &third_request);
        let error = router
            .dispatch(third_command, third_request)
            .await
            .expect_err("overflow session command should be rejected while lane is full");
        assert!(error
            .to_string()
            .contains("session command lane overloaded"));

        drop(app_guard);
        let _ = first_result_rx.await.expect("first result should resolve");
        let _ = queued_result_rx
            .await
            .expect("queued result should resolve");
    }

    #[tokio::test]
    async fn focus_uses_session_lane_when_interactive_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let blocked_request = focus_request(&session_id, &agent_id);
        let blocked_command =
            KernelCommand::from_local_request("cmd-blocked-generic", None, None, &blocked_request);
        let (blocked_result_tx, blocked_result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command: blocked_command,
                request: blocked_request,
                result_tx: blocked_result_tx,
            })
            .expect("generic interactive lane should fill");

        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        let focus_router = router.clone();
        let focus_task =
            tokio::spawn(async move { focus_router.dispatch(focus_command, focus_request).await });

        tokio::task::yield_now().await;
        assert!(
            !focus_task.is_finished(),
            "focus should be admitted to the session lane instead of failing on the full generic lane"
        );

        drop(app_guard);
        let _ = blocked_result_rx
            .await
            .expect("blocked generic command should resolve");
        let focus_response = focus_task
            .await
            .expect("focus task should join")
            .expect("focus should succeed");
        assert!(matches!(
            focus_response,
            crate::local::LocalDaemonResponse::AgentFocused { .. }
        ));
    }

    #[tokio::test]
    async fn end_session_uses_session_lane_and_removes_lane_registration() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let attach_request = attach_request(&session_id, "cli-1");
        let attach_command =
            KernelCommand::from_local_request("cmd-attach", None, None, &attach_request);
        router
            .dispatch(attach_command, attach_request)
            .await
            .expect("attach should create a session lane");
        assert!(router.session_runtime.has_lane(&session_id).await);

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let end_command = KernelCommand::from_local_request("cmd-end", None, None, &end_request);
        let response = router
            .dispatch(end_command, end_request)
            .await
            .expect("end session should run through the session lane");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionEnded { .. }
        ));
        assert!(
            !router.session_runtime.has_lane(&session_id).await,
            "ending a session should remove its mailbox registration"
        );
    }

    #[tokio::test]
    async fn daemon_health_projection_reports_session_and_agent_mailboxes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should create a session lane");

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from health projection test".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");

        let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let health_command =
            KernelCommand::from_local_request("cmd-health", None, None, &health_request);
        let health_response = router
            .dispatch(health_command, health_request)
            .await
            .expect("health projection should be returned");
        let projection = match health_response {
            LocalDaemonResponse::DaemonHealth { projection } => projection,
            _ => panic!("unexpected health response"),
        };
        assert!(projection
            .session_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert!(projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id && lane.queue_limit == 128));
    }

    #[tokio::test]
    async fn prompt_submit_uses_agent_lane_when_interactive_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let first_request = focus_request(&session_id, &agent_id);
        let first_command =
            KernelCommand::from_local_request("cmd-focus-1", None, None, &first_request);
        let first_router = router.clone();
        let first_task =
            tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

        for _ in 0..10 {
            if router.interactive_tx.capacity() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let second_request = focus_request(&session_id, &agent_id);
        let second_command =
            KernelCommand::from_local_request("cmd-focus-2", None, None, &second_request);
        let (second_result_tx, second_result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command: second_command,
                request: second_request,
                result_tx: second_result_tx,
            })
            .expect("second command should fill the interactive lane");

        for _ in 0..10 {
            if router.interactive_tx.capacity() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        tokio::task::yield_now().await;
        assert!(
            !prompt_task.is_finished(),
            "prompt should be admitted to the agent lane instead of failing on the full interactive lane"
        );

        drop(app_guard);
        let _ = first_task.await.expect("first focus should join");
        let _ = second_result_rx.await.expect("second focus should resolve");
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
        match prompt_response {
            crate::local::LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent_id);
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn prompt_submit_uses_session_focus_projection_without_app_lock_for_routing() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let focused_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.clone(),
                alias: Some("focused".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("focused agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(focused_agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let focus_request = focus_request(&session_id, focused_agent.id());
        let focus_command =
            KernelCommand::from_local_request("cmd-focus-projection", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should populate the projection");

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello through projected focus".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt-projection", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let mut focused_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == focused_agent.id())
            {
                focused_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            focused_agent_lane_created,
            "prompt submit should resolve focused agent from the session projection before touching the app lock"
        );
        assert!(
            !prompt_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), focused_agent.id());
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    fn attach_request(session_id: &str, client_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: client_id.to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        })
    }

    fn focus_request(session_id: &str, agent_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }
}
