//! A short command on an acquired machine uses the ordinary Room exec pipes.
use super::remote::Remote;
use crate::error::DaemonError;
use crate::local::{EndSessionRequest, LocalDaemonRequest};
use crate::room_exec::gateway_placement::{create_target_room, GrantedSend};
use crate::room_exec::ExecStream;
use crate::runtime::kernel_gateway::forward_with_retry;
use crate::runtime::state::machine_acquisition::acquisition_error;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);
const OUTPUT_LIMIT: usize = 32 * 1024;

pub(in crate::runtime::state) async fn run(
    grant: Arc<dyn GrantedSend>,
    command_id: &str,
    argv: Vec<String>,
    directory: Option<&str>,
) -> Result<Value, DaemonError> {
    let room = create_target_room(&*grant, command_id, directory).await?;
    let mut cleanup = RoomCleanup {
        grant: Some(grant.clone()),
        room: room.clone(),
        command_id: format!("{command_id}:end-room"),
    };
    let remote = Remote::new(grant, room, command_id.to_owned());
    let result = run_command(&remote, argv).await;
    let closed = cleanup.close().await;
    result.and_then(|value| closed.map(|()| value))
}

async fn run_command(remote: &Remote, argv: Vec<String>) -> Result<Value, DaemonError> {
    let id = remote.open(argv, None, "held-exec").await?;
    let executed = tokio::time::timeout(COMMAND_TIMEOUT, async {
        remote.close_stdin(&id, 0).await?;
        let mut cursor = 0;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut truncated = false;
        loop {
            let page = remote.read(&id, cursor).await?;
            cursor = page.next_cursor;
            for chunk in page.chunks {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(chunk.data_base64)
                    .map_err(|_| {
                        acquisition_error(
                            "machine_exec_output_invalid",
                            "The machine returned unreadable command output",
                        )
                    })?;
                let stream = match chunk.stream {
                    ExecStream::Stdout => &mut stdout,
                    ExecStream::Stderr => &mut stderr,
                };
                let count = bytes.len().min(OUTPUT_LIMIT.saturating_sub(stream.len()));
                stream.extend_from_slice(&bytes[..count]);
                truncated |= count < bytes.len();
            }
            if let Some(outcome) = page.outcome {
                return Ok(json!({
                    "outcome":outcome,
                    "stdout":String::from_utf8_lossy(&stdout),
                    "stderr":String::from_utf8_lossy(&stderr),
                    "truncated":truncated,
                }));
            }
        }
    })
    .await;
    match executed {
        Ok(result) => result,
        Err(_) => {
            let _ = remote.cancel(&id).await;
            Err(acquisition_error(
                "machine_exec_timeout",
                "The command exceeded 25 seconds. Use a job for longer work",
            ))
        }
    }
}

struct RoomCleanup {
    grant: Option<Arc<dyn GrantedSend>>,
    room: String,
    command_id: String,
}

impl RoomCleanup {
    async fn close(&mut self) -> Result<(), DaemonError> {
        let Some(grant) = self.grant.as_ref() else {
            return Ok(());
        };
        end_room(&**grant, &self.room, &self.command_id).await?;
        self.grant = None;
        Ok(())
    }
}

impl Drop for RoomCleanup {
    fn drop(&mut self) {
        if let Some(grant) = self.grant.take() {
            let room = self.room.clone();
            let command_id = self.command_id.clone();
            tokio::spawn(async move {
                if let Err(error) = end_room(&*grant, &room, &command_id).await {
                    tracing::warn!(room, %error, "could not close the acquired machine command Room");
                }
            });
        }
    }
}

async fn end_room(
    grant: &dyn GrantedSend,
    room: &str,
    command_id: &str,
) -> Result<(), DaemonError> {
    let request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: room.to_owned(),
    });
    forward_with_retry(command_id.to_owned(), |id| grant.send(id, &request))
        .await
        .map_err(|error| {
            acquisition_error(
                "machine_exec_cleanup_unavailable",
                &format!(
                    "The acquired machine did not close the command Room: {}",
                    error.transport_error().message
                ),
            )
        })?;
    Ok(())
}
