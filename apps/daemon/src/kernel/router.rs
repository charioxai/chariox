use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::command::{KernelCommand, KernelCommandPriority};
use crate::kernel::session_actor::SessionActor;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse, RelayStatus};

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
    let mut app = app.lock().await;
    if let Some(result) = SessionActor::handle_interactive_command(&mut app, request.clone()) {
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

fn is_blocking_local_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)
            | LocalDaemonRequest::GetProviderAuthStatus(_)
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::LogoutProvider(_)
            | LocalDaemonRequest::ListProviderProcesses(_)
            | LocalDaemonRequest::TeardownProviderProcesses(_)
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
