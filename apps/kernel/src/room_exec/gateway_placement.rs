//! The production `ExecPlacement`: the existing Room exec operations, sent to another
//! kernel through the gateway under the grant of the agent that owns the launch.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::placement::{ExecPlacement, PlacementFuture};
use super::{error, ExecOutput, LaunchIntent};
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonRequest, LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest,
    RoomTerminalResult,
};
use crate::runtime::kernel_gateway::{forward_with_retry, GatewayError};

pub(crate) type SendFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Value, GatewayError>> + Send + 'a>>;

/// One request to the other kernel under the owning agent's grant.
pub(crate) trait GrantedSend: Send + Sync {
    fn send<'a>(&'a self, command_id: String, request: &'a LocalDaemonRequest) -> SendFuture<'a>;
}

/// The requester's own gateway grant on this kernel (the owning agent's reach, or the
/// owner's person grant): the same check and the same forwarding as an external agent's
/// request, with no grant of its own.
pub(crate) struct OriginGrant {
    router: crate::runtime::router::CommandRouter,
    origin: crate::runtime::kernel_gateway::Origin,
    kernel: String,
}

impl OriginGrant {
    pub(crate) fn new(
        router: crate::runtime::router::CommandRouter,
        origin: crate::runtime::kernel_gateway::Origin,
        kernel: &str,
    ) -> Self {
        Self {
            router,
            origin,
            kernel: kernel.to_owned(),
        }
    }
}

impl GrantedSend for OriginGrant {
    fn send<'a>(&'a self, command_id: String, request: &'a LocalDaemonRequest) -> SendFuture<'a> {
        Box::pin(async move {
            self.router
                .runtime_state()
                .authorize_gateway_request(&self.origin.grant_id, request)
                .map_err(|refusal| GatewayError::Remote {
                    name: self.kernel.clone(),
                    code: "not_allowed".into(),
                    message: refusal.to_string(),
                    retryable: false,
                })?;
            crate::runtime::kernel_gateway::forward_audited(
                &self.router,
                &self.origin,
                &self.origin,
                &command_id,
                &self.kernel,
                request,
            )
            .await
        })
    }
}

/// A held launch running in one Room of another kernel. Every command id is derived from
/// the launch, so a retry of the same step is the same request to the target's ledger.
pub(crate) struct GatewayExecPlacement {
    grant: Arc<dyn GrantedSend>,
    target_session_id: String,
}

impl GatewayExecPlacement {
    pub(crate) fn new(grant: Arc<dyn GrantedSend>, target_session_id: String) -> Self {
        Self {
            grant,
            target_session_id,
        }
    }

    async fn call(
        &self,
        command_id: String,
        acting_agent: Option<String>,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
            session_id: self.target_session_id.clone(),
            acting_agent,
            host_agent: None,
            operation,
        });
        let (grant, sent) = (&*self.grant, &request);
        let answer = forward_with_retry(command_id, move |id| grant.send(id, sent))
            .await
            .map_err(|failure| error(failure.code(), failure.transport_error().message))?;
        match serde_json::from_value(answer) {
            Ok(LocalDaemonResponse::RoomTerminal { result }) => Ok(result),
            _ => Err(error(
                "exec_target_unreadable",
                "the other kernel answered with something that is not a Room exec result",
            )),
        }
    }
}

/// The Room on the other kernel the launch runs in. Its command id comes from the launch, so
/// a retry after a lost answer is answered by the target's ledger instead of making a second Room.
pub(crate) async fn create_target_room(
    grant: &dyn GrantedSend,
    exec_id: &str,
    directory: Option<&str>,
) -> Result<String, DaemonError> {
    let directory = directory.unwrap_or_default();
    let mut create = crate::session::CreateSessionRequest::new(directory, directory);
    create.no_subscribe = true;
    let request = LocalDaemonRequest::CreateSession(create);
    let sent = &request;
    let answer = forward_with_retry(format!("place:{exec_id}:room"), move |id| {
        grant.send(id, sent)
    })
    .await
    .map_err(|failure| error(failure.code(), failure.transport_error().message))?;
    match serde_json::from_value(answer) {
        Ok(LocalDaemonResponse::SessionCreated { session, .. }) => Ok(session.id().to_owned()),
        _ => Err(error(
            "exec_target_unreadable",
            "the other kernel answered with something that is not a new Room",
        )),
    }
}

/// The workspace and worktree of a Room made without a directory.
pub(crate) fn room_location(
    _directory: Option<&str>,
    _launch_target: Option<crate::local::WaitingRoomLaunchTarget>,
) -> Result<(String, String), DaemonError> {
    Err(error("exec_target_no_directory", "not implemented"))
}

fn unexpected() -> DaemonError {
    error(
        "exec_target_unreadable",
        "the other kernel answered with a result of another exec operation",
    )
}

impl ExecPlacement for GatewayExecPlacement {
    fn open<'a>(
        &'a self,
        intent: &'a LaunchIntent,
        attributed_agent: Option<String>,
    ) -> PlacementFuture<'a, String> {
        Box::pin(async move {
            let command_id = format!("place:{}:open", intent.exec_id);
            let operation = RoomTerminalOperation::ExecOpen {
                command: intent.command.clone(),
            };
            match self.call(command_id, attributed_agent, operation).await? {
                RoomTerminalResult::ExecOpened { exec_id } => Ok(exec_id),
                _ => Err(unexpected()),
            }
        })
    }

    fn write<'a>(
        &'a self,
        exec_id: &'a str,
        offset: u64,
        data_base64: &'a str,
    ) -> PlacementFuture<'a, u64> {
        Box::pin(async move {
            let operation = RoomTerminalOperation::ExecWrite {
                exec_id: exec_id.to_owned(),
                offset,
                data_base64: data_base64.to_owned(),
            };
            let command_id = format!("place:{exec_id}:write:{offset}");
            match self.call(command_id, None, operation).await? {
                RoomTerminalResult::ExecInput { next_offset } => Ok(next_offset),
                _ => Err(unexpected()),
            }
        })
    }

    fn close_stdin<'a>(&'a self, exec_id: &'a str, offset: u64) -> PlacementFuture<'a, u64> {
        Box::pin(async move {
            let operation = RoomTerminalOperation::ExecStdinClose {
                exec_id: exec_id.to_owned(),
                offset,
            };
            let command_id = format!("place:{exec_id}:stdin-close:{offset}");
            match self.call(command_id, None, operation).await? {
                RoomTerminalResult::ExecInput { next_offset } => Ok(next_offset),
                _ => Err(unexpected()),
            }
        })
    }

    fn read<'a>(
        &'a self,
        exec_id: &'a str,
        after_cursor: u64,
        wait: Duration,
    ) -> PlacementFuture<'a, ExecOutput> {
        Box::pin(async move {
            let operation = RoomTerminalOperation::ExecRead {
                exec_id: exec_id.to_owned(),
                after_cursor,
                wait_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
            };
            let command_id = format!("place:{exec_id}:read:{after_cursor}");
            match self.call(command_id, None, operation).await? {
                RoomTerminalResult::ExecOutput { output } => Ok(output),
                _ => Err(unexpected()),
            }
        })
    }

    fn cancel<'a>(&'a self, exec_id: &'a str) -> PlacementFuture<'a, ()> {
        Box::pin(async move {
            let operation = RoomTerminalOperation::ExecCancel {
                exec_id: exec_id.to_owned(),
            };
            let command_id = format!("place:{exec_id}:cancel");
            match self.call(command_id, None, operation).await? {
                RoomTerminalResult::ExecCancelled => Ok(()),
                _ => Err(unexpected()),
            }
        })
    }
}
