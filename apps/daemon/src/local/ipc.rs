use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::app::DaemonApp;
use crate::error::DaemonError;

use super::{LocalDaemonRequest, LocalDaemonResponse};

const IPC_IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_IPC_FRAME_BYTES: usize = 1024 * 1024;

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
    let socket_path = app.config().local_socket_path.clone();
    prepare_socket_path(&socket_path)?;

    let listener =
        UnixListener::bind(&socket_path).map_err(|error| DaemonError::LocalTransport {
            operation: "bind local socket",
            message: error.to_string(),
        })?;
    harden_socket_permissions(&socket_path)?;
    let app = Arc::new(Mutex::new(app));

    tokio::pin!(shutdown);

    let result = loop {
        tokio::select! {
            _ = &mut shutdown => break Ok(()),
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.map_err(|error| DaemonError::LocalTransport {
                    operation: "accept local socket connection",
                    message: error.to_string(),
                })?;
                let app = Arc::clone(&app);
                tokio::spawn(async move {
                    let _ = handle_connection(app, stream).await;
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
    app: Arc<Mutex<DaemonApp>>,
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
            let response = {
                let mut app = app.lock().await;
                app.handle_local_request(request)
            };
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
    use std::time::Duration;

    use tokio::sync::oneshot;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AttachToSessionRequest, LaunchProviderRunRequest, PumpTerminalOutputRequest,
        SubmitPromptRequest,
    };
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    use super::{
        read_sync_frame, run_local_ipc_server, LocalDaemonRequest, LocalDaemonResponse,
        LocalIpcClient, StdUnixStream,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn local_ipc_round_trip_exercises_session_and_terminal_flow() {
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
        let session = match client
            .send(&LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ipc", "."),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
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
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "default".to_string(),
                    variant: None,
                },
            ))
            .expect("launch should succeed");
        client
            .send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "ipc smoke\n".to_string(),
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
    async fn malformed_request_does_not_block_followup_request() {
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

    async fn wait_for_socket(socket_path: &Path) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

        while !socket_path.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for socket {}",
                socket_path.display()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_output(client: &LocalIpcClient, session_id: &str, attachment_id: &str) -> String {
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
