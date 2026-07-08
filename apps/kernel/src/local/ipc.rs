use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    let socket_path = crate::runtime::app_lock::lock_app_instrumented(&app, "local_ipc")
        .await
        .config()
        .local_socket_path
        .clone();
    prepare_socket_path(&socket_path)?;

    let listener =
        UnixListener::bind(&socket_path).map_err(|error| DaemonError::LocalTransport {
            operation: "bind local socket",
            message: error.to_string(),
        })?;
    harden_socket_permissions(&socket_path)?;
    let provider_runtime_lanes = crate::runtime::app_lock::lock_app_instrumented(&app, "local_ipc")
        .await
        .provider_run_operation_lanes();
    let durable_snapshot_scheduler = {
        let app = crate::runtime::app_lock::lock_app_instrumented(&app, "local_ipc").await;
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
                let mut app = crate::runtime::app_lock::lock_app_instrumented(&app, "local_ipc").await;
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
mod tests;
