use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::command::{KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::session::unix_epoch_ms;

use super::{LocalDaemonRequest, LocalDaemonResponse};

const IPC_IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_IPC_FRAME_BYTES: usize = 1024 * 1024;
const DURABLE_SNAPSHOT_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct LocalIpcClient {
    socket_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct IpcResponseEnvelope {
    response: Option<LocalDaemonResponse>,
    error: Option<String>,
}

impl LocalIpcClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn send(&self, request: &LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        send_local_ipc_request(&self.socket_path, request)
    }
}

pub async fn run_local_ipc_server<F>(app: DaemonApp, shutdown: F) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    run_local_ipc_server_with_shared_app(Arc::new(Mutex::new(app)), shutdown).await
}

async fn run_local_ipc_server_with_shared_app<F>(
    app: Arc<Mutex<DaemonApp>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let socket_path = app.lock().await.config().local_socket_path.clone();
    prepare_socket_path(&socket_path)?;

    let listener =
        UnixListener::bind(&socket_path).map_err(|error| DaemonError::LocalTransport {
            operation: "bind local socket",
            message: error.to_string(),
        })?;
    harden_socket_permissions(&socket_path)?;
    let provider_runtime_lanes = app.lock().await.provider_run_operation_lanes();
    let durable_snapshot_scheduler = {
        let app = app.lock().await;
        app.durable_snapshot_scheduler()
    };
    let mut durable_snapshot_task = durable_snapshot_scheduler
        .map(|scheduler| tokio::spawn(scheduler.run(DURABLE_SNAPSHOT_POLL_INTERVAL)));
    let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    ));
    let command_sequence = Arc::new(AtomicU64::new(1));

    tokio::pin!(shutdown);

    let result = loop {
        tokio::select! {
            _ = &mut shutdown => {
                if let Some(task) = durable_snapshot_task.take() {
                    task.abort();
                }
                let mut app = app.lock().await;
                let _ = app.shutdown_cleanup();
                break Ok(());
            },
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.map_err(|error| DaemonError::LocalTransport {
                    operation: "accept local socket connection",
                    message: error.to_string(),
                })?;
                let router = Arc::clone(&router);
                let command_sequence = Arc::clone(&command_sequence);
                tokio::spawn(async move {
                    let _ = handle_connection(router, command_sequence, stream).await;
                });
            }
        }
    };

    let _ = fs::remove_file(&socket_path);
    result
}

pub fn send_local_ipc_request(
    socket_path: &Path,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut stream =
        StdUnixStream::connect(socket_path).map_err(|error| DaemonError::LocalTransport {
            operation: "connect local socket",
            message: error.to_string(),
        })?;
    stream
        .set_read_timeout(Some(IPC_IO_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(IPC_IO_TIMEOUT)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "configure local socket timeouts",
            message: error.to_string(),
        })?;

    let payload = serialize_request(request)?;
    let frame = encode_frame(&payload)?;

    stream
        .write_all(&frame)
        .and_then(|_| stream.shutdown(Shutdown::Write))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write local request",
            message: error.to_string(),
        })?;

    let response_bytes = read_sync_frame(&mut stream)?;
    let envelope = decode_envelope(&response_bytes)?;

    match (envelope.response, envelope.error) {
        (Some(response), None) => Ok(response),
        (_, Some(message)) => Err(DaemonError::LocalTransport {
            operation: "handle local response",
            message,
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "handle local response",
            message: "response envelope was empty".to_string(),
        }),
    }
}

async fn handle_connection(
    router: Arc<CommandRouter>,
    command_sequence: Arc<AtomicU64>,
    mut stream: tokio::net::UnixStream,
) -> Result<(), DaemonError> {
    let request_bytes = match read_async_frame(&mut stream).await {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.ipc.server",
                "failed reading request frame",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            let response = encode_envelope(IpcResponseEnvelope {
                response: None,
                error: Some(error.to_string()),
            })?;
            return write_async_frame(&mut stream, &response).await;
        }
    };

    let envelope = match serde_json::from_slice::<LocalDaemonRequest>(&request_bytes) {
        Ok(request) => {
            let response = dispatch_local_ipc_request(&router, &command_sequence, request).await;
            match response {
                Ok(response) => IpcResponseEnvelope {
                    response: Some(response),
                    error: None,
                },
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.ipc.server",
                        "local request failed",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    IpcResponseEnvelope {
                        response: None,
                        error: Some(error.to_string()),
                    }
                }
            }
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.ipc.server",
                "invalid local request payload",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            IpcResponseEnvelope {
                response: None,
                error: Some(format!("invalid local request: {error}")),
            }
        }
    };

    let response_bytes = encode_envelope(envelope)?;
    match write_async_frame(&mut stream, &response_bytes).await {
        Ok(()) => Ok(()),
        Err(error) => {
            if matches!(
                &error,
                DaemonError::LocalTransport {
                    operation,
                    message,
                } if *operation == "decode local frame" && message.contains("payload exceeded")
            ) {
                crate::logging::warn_with_fields(
                    "daemon.ipc.server",
                    "local response exceeded ipc frame limit",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                let fallback = encode_envelope(IpcResponseEnvelope {
                    response: None,
                    error: Some(
                        "local response exceeded ipc frame limit; request a smaller payload"
                            .to_string(),
                    ),
                })?;
                return write_async_frame(&mut stream, &fallback).await;
            }
            Err(error)
        }
    }
}

async fn dispatch_local_ipc_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let sequence = command_sequence.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("ipc-{}-{sequence}", unix_epoch_ms());
    let caller = router
        .local_command_caller(KernelCommandSource::LocalIpc)
        .await;
    let command = KernelCommand::from_local_request_with_caller(
        command_id,
        KernelCommandSource::LocalIpc,
        caller,
        None,
        None,
        &request,
    );
    router.dispatch(command, request).await
}

fn prepare_socket_path(socket_path: &Path) -> Result<(), DaemonError> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "prepare local socket directory",
            message: error.to_string(),
        })?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "harden local socket directory permissions",
                message: error.to_string(),
            }
        })?;
    }

    if socket_path.exists() {
        match StdUnixStream::connect(socket_path) {
            Ok(_) => {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare local socket path",
                    message: format!("socket `{}` is already in use", socket_path.display()),
                });
            }
            Err(_) => {
                fs::remove_file(socket_path).map_err(|error| DaemonError::LocalTransport {
                    operation: "remove stale local socket",
                    message: error.to_string(),
                })?;
            }
        }
    }

    Ok(())
}

fn harden_socket_permissions(socket_path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "harden local socket permissions",
            message: error.to_string(),
        }
    })?;

    Ok(())
}

fn serialize_request(request: &LocalDaemonRequest) -> Result<Vec<u8>, DaemonError> {
    serde_json::to_vec(request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize local request",
        message: error.to_string(),
    })
}

fn encode_envelope(envelope: IpcResponseEnvelope) -> Result<Vec<u8>, DaemonError> {
    serde_json::to_vec(&envelope).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize local response",
        message: error.to_string(),
    })
}

fn decode_envelope(bytes: &[u8]) -> Result<IpcResponseEnvelope, DaemonError> {
    serde_json::from_slice(bytes).map_err(|error| DaemonError::LocalTransport {
        operation: "decode local response",
        message: error.to_string(),
    })
}

fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, DaemonError> {
    ensure_frame_size(payload.len())?;

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn read_sync_frame(stream: &mut StdUnixStream) -> Result<Vec<u8>, DaemonError> {
    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read local response header",
            message: error.to_string(),
        })?;
    let payload_len = u32::from_be_bytes(header) as usize;
    ensure_frame_size(payload_len)?;

    let mut payload = vec![0_u8; payload_len];
    stream
        .read_exact(&mut payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read local response body",
            message: error.to_string(),
        })?;
    Ok(payload)
}

async fn read_async_frame(stream: &mut tokio::net::UnixStream) -> Result<Vec<u8>, DaemonError> {
    let mut header = [0_u8; 4];
    timeout(IPC_IO_TIMEOUT, stream.read_exact(&mut header))
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "read local request header",
            message: "timed out".to_string(),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read local request header",
            message: error.to_string(),
        })?;

    let payload_len = u32::from_be_bytes(header) as usize;
    ensure_frame_size(payload_len)?;

    let mut payload = vec![0_u8; payload_len];
    timeout(IPC_IO_TIMEOUT, stream.read_exact(&mut payload))
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "read local request body",
            message: "timed out".to_string(),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read local request body",
            message: error.to_string(),
        })?;

    Ok(payload)
}

async fn write_async_frame(
    stream: &mut tokio::net::UnixStream,
    payload: &[u8],
) -> Result<(), DaemonError> {
    let frame = encode_frame(payload)?;
    timeout(IPC_IO_TIMEOUT, stream.write_all(&frame))
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "write local response frame",
            message: "timed out".to_string(),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write local response frame",
            message: error.to_string(),
        })?;
    timeout(IPC_IO_TIMEOUT, stream.shutdown())
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "close local response",
            message: "timed out".to_string(),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "close local response",
            message: error.to_string(),
        })
}

fn ensure_frame_size(payload_len: usize) -> Result<(), DaemonError> {
    if payload_len > MAX_IPC_FRAME_BYTES {
        return Err(DaemonError::LocalTransport {
            operation: "decode local frame",
            message: format!("payload exceeded {} bytes", MAX_IPC_FRAME_BYTES),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::Shutdown;
    use std::path::Path;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::Duration;

    use tokio::sync::{oneshot, Mutex as TokioMutex};

    use crate::attachment::ClientCapabilityLevel;
    use crate::config::PersistedCloudRelayProfile;
    use crate::local::api::{
        AddWorkflowEdgeRequest, AddWorkflowNodeRequest, CancelWorkflowRunRequest,
        CreateWorkflowEndpointRequest, CreateWorkflowRequest, GetWorkflowRunRequest,
        InvokeWorkflowEndpointRequest, ListWorkflowRunsRequest,
    };
    use crate::local::{
        AttachToSessionRequest, CompletePromptRequest, LaunchProviderRunRequest,
        PumpTerminalOutputRequest, RunShellCapabilityRequest, SpawnAgentRequest,
        SubmitPromptRequest,
    };
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    use super::{
        read_sync_frame, run_local_ipc_server, LocalDaemonRequest, LocalDaemonResponse,
        LocalIpcClient, StdUnixStream,
    };

    static LOCAL_IPC_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn local_ipc_test_guard() -> MutexGuard<'static, ()> {
        LOCAL_IPC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_ipc_round_trip_exercises_session_and_terminal_flow() {
        let _guard = local_ipc_test_guard();
        let config = DaemonConfig::for_tests();
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let app = Arc::new(TokioMutex::new(
            DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
        ));
        let server_app = Arc::clone(&app);
        let server = tokio::spawn(async move {
            super::run_local_ipc_server_with_shared_app(server_app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let client = LocalIpcClient::new(socket_path.clone());
        let session = match client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc", "."),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            other => panic!("unexpected response: {other:?}"),
        };
        let attachment = match client
            .send(&LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "ipc-client".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            other => panic!("unexpected response: {other:?}"),
        };

        client
            .send(&LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: None,
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "default".to_string(),
                    variant: None,
                    structured_endpoint: None,
                    provider_session_id: None,
                    native_tui: false,
                },
            ))
            .expect("launch should succeed");
        client
            .send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: None,
                prompt: "ipc smoke\n".to_string(),
                attachments: Vec::new(),
            }))
            .expect("prompt submit should succeed");

        let output = wait_for_output(&client, session.id(), attachment.id()).await;
        assert!(output.contains("ipc smoke"));

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_ipc_uses_linked_cloud_user_for_session_creation() {
        let _guard = local_ipc_test_guard();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "miguel@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-cloud".to_string(),
            account_slug: "miguel".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "ws://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: Some("local-cli".to_string()),
            machine_id: Some("machine-1".to_string()),
            machine_alias: Some("macbook".to_string()),
            machine_credential: None,
            cloud_session_token: Some("session-token".to_string()),
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
            run_local_ipc_server(app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let client = LocalIpcClient::new(socket_path.clone());
        let response = client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc-cloud", "."),
            ))
            .expect("session create should succeed");
        let session = match response {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(session.owner_user_id(), "user-cloud");
        assert!(session.has_member("user-cloud"));

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_ipc_prompt_submit_acks_while_shell_capability_is_slow() {
        let _guard = local_ipc_test_guard();
        let config = DaemonConfig::for_tests();
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
            run_local_ipc_server(app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let client = LocalIpcClient::new(socket_path.clone());
        let cwd = std::env::current_dir()
            .expect("current directory should be available")
            .to_string_lossy()
            .to_string();
        let session = match client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(cwd.as_str(), cwd.as_str()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected response: {other:?}"),
        };
        let agent_id = session
            .agents()
            .first()
            .expect("default agent should exist")
            .id()
            .to_string();
        let attachment = match client
            .send(&LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "ipc-responsive-client".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            other => panic!("unexpected response: {other:?}"),
        };

        client
            .send(&LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id.clone()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "default".to_string(),
                    variant: None,
                    structured_endpoint: None,
                    provider_session_id: None,
                    native_tui: false,
                },
            ))
            .expect("launch should succeed");

        let slow_client = LocalIpcClient::new(socket_path.clone());
        let slow_session_id = session.id().to_string();
        let slow_attachment_id = attachment.id().to_string();
        let slow_task = tokio::task::spawn_blocking(move || {
            slow_client.send(&LocalDaemonRequest::RunShellCommand(
                RunShellCapabilityRequest {
                    session_id: slow_session_id,
                    attachment_id: slow_attachment_id,
                    command: "sh".to_string(),
                    args: vec!["-c".to_string(), "sleep 0.5".to_string()],
                    working_directory: None,
                    timeout_ms: Some(1_000),
                },
            ))
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let submit_client = LocalIpcClient::new(socket_path.clone());
        let submit_session_id = session.id().to_string();
        let submit_attachment_id = attachment.id().to_string();
        let submit_agent_id = agent_id.clone();
        let submit_task = tokio::task::spawn_blocking(move || {
            submit_client.send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: submit_session_id,
                attachment_id: submit_attachment_id,
                target_agent_id: Some(submit_agent_id),
                prompt: "ipc prompt should ack while shell command is still running".to_string(),
                attachments: Vec::new(),
            }))
        });
        let submit_response = tokio::time::timeout(Duration::from_millis(250), submit_task)
            .await
            .expect("prompt submit should not wait for slow shell")
            .expect("prompt submit task should join")
            .expect("prompt submit should succeed");
        assert!(matches!(
            submit_response,
            LocalDaemonResponse::PromptSubmitted { .. }
        ));

        let shell_response = slow_task
            .await
            .expect("slow shell task should join")
            .expect("slow shell request should succeed");
        assert!(matches!(
            shell_response,
            LocalDaemonResponse::ShellCommandCompleted { .. }
        ));

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_request_does_not_block_followup_request() {
        let _guard = local_ipc_test_guard();
        let config = DaemonConfig::for_tests();
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let app = Arc::new(TokioMutex::new(
            DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
        ));
        let server_app = Arc::clone(&app);
        let server = tokio::spawn(async move {
            super::run_local_ipc_server_with_shared_app(server_app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let mut bad_client = StdUnixStream::connect(&socket_path).expect("socket should accept");
        bad_client
            .set_write_timeout(Some(Duration::from_secs(1)))
            .expect("write timeout should configure");
        bad_client
            .write_all(&2_u32.to_be_bytes())
            .expect("bad frame header should write");
        bad_client.write_all(b"{").expect("bad body should write");
        bad_client
            .shutdown(Shutdown::Write)
            .expect("bad client should close write side");
        let _ = read_sync_frame(&mut bad_client).expect("server should answer malformed request");

        let client = LocalIpcClient::new(socket_path.clone());
        let response = client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc-followup", "."),
            ))
            .expect("followup request should still succeed");
        match response {
            LocalDaemonResponse::SessionCreated { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_ipc_round_trip_exercises_workflow_run_lifecycle() {
        let _guard = local_ipc_test_guard();
        let config = DaemonConfig::for_tests();
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let app = Arc::new(TokioMutex::new(
            DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
        ));
        let server_app = Arc::clone(&app);
        let server = tokio::spawn(async move {
            super::run_local_ipc_server_with_shared_app(server_app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let client = LocalIpcClient::new(socket_path.clone());
        let session = match client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc-workflow", "."),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected response: {other:?}"),
        };

        let agent = match client
            .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
            }))
            .expect("workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected response: {other:?}"),
        };

        let workflow = match client
            .send(&LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("review".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            other => panic!("unexpected response: {other:?}"),
        };

        let node = match client
            .send(&LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: agent.id().to_string(),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow node add should succeed")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected response: {other:?}"),
        };

        let endpoint = match client
            .send(&LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow endpoint create should succeed")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            other => panic!("unexpected response: {other:?}"),
        };

        match client
            .send(&LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "default".to_string(),
                    variant: None,
                    structured_endpoint: None,
                    provider_session_id: None,
                    native_tui: false,
                },
            ))
            .expect("provider run launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { .. }
            | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let workflow_run = match client
            .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("socket drill".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(workflow_run.workflow_id(), workflow.id());
        assert_eq!(format!("{:?}", workflow_run.status()), "Running");

        let listed = match client
            .send(&LocalDaemonRequest::ListWorkflowRuns(
                ListWorkflowRunsRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: Some(workflow.id().to_string()),
                },
            ))
            .expect("workflow runs list should succeed")
        {
            LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id(), workflow_run.id());

        let resolved = match client
            .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("workflow run get should succeed")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(resolved.id(), workflow_run.id());
        assert_eq!(format!("{:?}", resolved.status()), "Running");

        fan_out_ipc_workflow_output(&app, session.id(), "workflow-backed prompt").await;
        match client
            .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("workflow-backed prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let completed = match client
            .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("completed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(format!("{:?}", completed.status()), "Completed");

        let second_run = match client
            .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("socket drill again".to_string()),
                },
            ))
            .expect("second workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };

        let cancelled = match client
            .send(&LocalDaemonRequest::CancelWorkflowRun(
                CancelWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: second_run.id().to_string(),
                },
            ))
            .expect("workflow run cancel should succeed")
        {
            LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(format!("{:?}", cancelled.status()), "Stopped");

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_ipc_round_trip_routes_downstream_workflow_nodes() {
        let _guard = local_ipc_test_guard();
        let config = DaemonConfig::for_tests();
        let socket_path = config.local_socket_path.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let app = Arc::new(TokioMutex::new(
            DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
        ));
        let server_app = Arc::clone(&app);
        let server = tokio::spawn(async move {
            super::run_local_ipc_server_with_shared_app(server_app, async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path).await;

        let client = LocalIpcClient::new(socket_path.clone());
        let session = match client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc-workflow-chain", "."),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected response: {other:?}"),
        };

        let first_agent = match client
            .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("planner".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
            }))
            .expect("first workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected response: {other:?}"),
        };

        let second_agent = match client
            .send(&LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
            }))
            .expect("second workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected response: {other:?}"),
        };

        let workflow = match client
            .send(&LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("review".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            other => panic!("unexpected response: {other:?}"),
        };

        let first_node = match client
            .send(&LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: first_agent.id().to_string(),
                    expected_workflow_revision: None,
                },
            ))
            .expect("first workflow node add should succeed")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected response: {other:?}"),
        };

        let duplicate_node = client
            .send(&LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: first_agent.id().to_string(),
                    expected_workflow_revision: None,
                },
            ))
            .expect_err("duplicate workflow node add should be rejected");
        assert!(matches!(
            duplicate_node,
            DaemonError::LocalTransport { operation: "handle local response", ref message }
                if message.contains("already has a node for agent")
        ));

        let second_node = match client
            .send(&LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: second_agent.id().to_string(),
                    expected_workflow_revision: None,
                },
            ))
            .expect("second workflow node add should succeed")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected response: {other:?}"),
        };

        match client
            .send(&LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: first_node.id().to_string(),
                    to_node_id: second_node.id().to_string(),
                    handoff_schema_ref: None,
                    validation_policy: None,
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow edge add should succeed")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let endpoint = match client
            .send(&LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: first_node.id().to_string(),
                    alias: Some("entry".to_string()),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow endpoint create should succeed")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            other => panic!("unexpected response: {other:?}"),
        };

        let workflow_run = match client
            .send(&LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("socket chain drill".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(format!("{:?}", workflow_run.status()), "Running");

        fan_out_ipc_workflow_output(&app, session.id(), "entry workflow prompt").await;
        match client
            .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("entry workflow prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let routed = match client
            .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("routed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(format!("{:?}", routed.status()), "Running");
        assert_eq!(routed.node_runs().len(), 2);
        assert_eq!(
            routed.active_node_run_id(),
            Some(routed.node_runs()[1].id())
        );
        assert_eq!(routed.node_runs()[1].node_id(), second_node.id());

        fan_out_ipc_workflow_output(&app, session.id(), "downstream workflow prompt").await;
        match client
            .send(&LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("downstream workflow prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let completed = match client
            .send(&LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("completed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(format!("{:?}", completed.status()), "Completed");
        assert_eq!(completed.node_runs().len(), 2);

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("server task should join")
            .expect("server should stop cleanly");
    }

    async fn wait_for_socket(socket_path: &Path) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

        loop {
            if socket_path.exists() && StdUnixStream::connect(socket_path).is_ok() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for socket {}",
                socket_path.display()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn fan_out_ipc_workflow_output(
        app: &Arc<TokioMutex<DaemonApp>>,
        session_id: &str,
        label: &str,
    ) {
        let payload = serde_json::json!({
            "summary": format!("{label} completed"),
            "output": {
                "message": format!("{label} output"),
            },
        });
        let output = format!(
            "```json\n{}\n```\n",
            serde_json::to_string(&payload).expect("workflow test output should serialize")
        );
        let mut app = app.lock().await;
        let provider_run_id = app
            .sessions()
            .get_session(session_id)
            .expect("session should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string();
        app.fan_out_output(
            session_id,
            &provider_run_id,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            output.as_bytes(),
        );
    }

    async fn wait_for_output(
        client: &LocalIpcClient,
        session_id: &str,
        attachment_id: &str,
    ) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

        loop {
            let response = client
                .send(&LocalDaemonRequest::PumpTerminalOutput(
                    PumpTerminalOutputRequest {
                        session_id: session_id.to_string(),
                        attachment_id: attachment_id.to_string(),
                    },
                ))
                .expect("output poll should succeed");
            if let LocalDaemonResponse::TerminalOutput { records } = response {
                if !records.is_empty() {
                    let combined = records
                        .into_iter()
                        .flat_map(|record| record.bytes)
                        .collect::<Vec<u8>>();
                    return String::from_utf8_lossy(&combined).into_owned();
                }
            }

            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for output"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
