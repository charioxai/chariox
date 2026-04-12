use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::agent_actor::AgentActor;
use crate::kernel::capability_executor::execute_capability_request;
use crate::kernel::command::{KernelCommand, KernelCommandPriority};
use crate::kernel::session_actor::SessionActor;
use crate::local::provider_requests::{
    launch_provider_request_from_local, load_provider_catalog, logout_provider_response,
    provider_auth_status_response, provider_command_catalogs_response,
    start_provider_login_response,
};
use crate::local::{
    GetSessionHistoryRequest, LaunchProviderRunRequest, ListProviderProcessesRequest,
    LocalDaemonRequest, LocalDaemonResponse, RelayStatus, TeardownProviderProcessesRequest,
};
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
}

impl CommandRouter {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self::with_interactive_capacity(app, INTERACTIVE_COMMAND_QUEUE_LIMIT)
    }

    pub(crate) fn with_interactive_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
    ) -> Self {
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        tokio::spawn(run_interactive_command_lane(
            Arc::clone(&app),
            interactive_rx,
        ));
        Self {
            app,
            interactive_tx,
        }
    }

    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match command.priority {
            KernelCommandPriority::Interactive => self.dispatch_interactive(command, request).await,
            KernelCommandPriority::Normal | KernelCommandPriority::Background => {
                execute_local_request_with_async_boundaries(&self.app, request).await
            }
        }
    }

    async fn dispatch_interactive(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
        result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await interactive kernel command",
                message: error.to_string(),
            })?
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
    if let LocalDaemonRequest::SubmitPrompt(request) = request {
        return execute_kernel_prompt_submit(app, request).await;
    }
    let mut app = app.lock().await;
    if let Some(result) = SessionActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    if let Some(result) = AgentActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    app.handle_local_request(request)
}

async fn execute_kernel_prompt_submit(
    app: &Arc<Mutex<DaemonApp>>,
    request: crate::local::SubmitPromptRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let prepared = {
        let mut app = app.lock().await;
        app.submit_prompt_for_kernel(
            &request.session_id,
            &request.attachment_id,
            request.target_agent_id.as_deref(),
            &request.prompt,
            request.attachments,
        )?
    };

    if let Some(dispatch) = prepared.dispatch {
        let app = Arc::clone(app);
        tokio::spawn(async move {
            let executed = tokio::task::spawn_blocking(move || {
                let session_id = dispatch.session_id;
                let provider_run_id = dispatch.provider_run_id;
                let agent_id = dispatch.agent_id;
                let (completion, result) = dispatch.job.execute();
                (session_id, provider_run_id, agent_id, completion, result)
            })
            .await;
            match executed {
                Ok((session_id, provider_run_id, agent_id, completion, result)) => {
                    let mut app = app.lock().await;
                    let _ = app.finish_kernel_prompt_dispatch(
                        session_id,
                        provider_run_id,
                        agent_id,
                        completion,
                        result,
                    );
                }
                Err(error) => {
                    crate::logging::error_with_fields(
                        "daemon.kernel_router",
                        "kernel prompt dispatch task failed",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        });
    }

    Ok(LocalDaemonResponse::PromptSubmitted {
        outcome: prepared.outcome,
        session: prepared.session,
    })
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
    use crate::local::{AttachToSessionRequest, LocalDaemonRequest};
    use crate::session::CreateSessionRequest;
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
    async fn rejects_interactive_commands_when_bounded_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let first_request = attach_request(&session_id, "cli-1");
        let first_command = KernelCommand::from_local_request("cmd-1", None, None, &first_request);
        let first_router = router.clone();
        let first_task =
            tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

        for _ in 0..10 {
            if router.interactive_tx.capacity() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let second_request = attach_request(&session_id, "cli-2");
        let second_command =
            KernelCommand::from_local_request("cmd-2", None, None, &second_request);
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

        let third_request = attach_request(&session_id, "cli-3");
        let third_command = KernelCommand::from_local_request("cmd-3", None, None, &third_request);
        let error = router
            .dispatch(third_command, third_request)
            .await
            .expect_err("third interactive command should be rejected while lane is full");
        assert!(error
            .to_string()
            .contains("interactive command lane overloaded"));

        drop(app_guard);
        let _ = first_task.await.expect("first task should join");
        let _ = second_result_rx
            .await
            .expect("second result should resolve");
    }

    fn attach_request(session_id: &str, client_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: client_id.to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        })
    }
}
