use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;

use super::*;
use crate::local::{
    LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult,
};
use crate::room_terminal::{OpenRoomTerminal, TerminalActor};
use crate::runtime::command::{KernelCaller, KernelCallerKind};

const OPERATION: &str = "terminal.room";

impl KernelRuntimeState {
    /// Terminals belong to the Room's owner (users and the owner's agents).
    /// Other people in the Room get no access.
    pub(crate) async fn room_terminal_for_client(
        &self,
        caller: &KernelCaller,
        request: RoomTerminalRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = self.authorize_room_terminal(caller, &request)?;
        let service = &self.owned.room_terminals;
        let session_id = request.session_id.as_str();
        let result = match request.operation {
            RoomTerminalOperation::Open {
                cols,
                rows,
                title,
                command,
            } => RoomTerminalResult::Opened {
                terminal: service.open(OpenRoomTerminal {
                    session_id: request.session_id.clone(),
                    working_directory: self.room_project_directory(session_id)?,
                    cols,
                    rows,
                    title,
                    opened_by: actor,
                    command,
                })?,
            },
            RoomTerminalOperation::List => RoomTerminalResult::Listed {
                terminals: service.list(session_id),
            },
            RoomTerminalOperation::Snapshot { terminal_id } => RoomTerminalResult::Snapshot {
                snapshot: service.snapshot(session_id, &terminal_id)?,
            },
            RoomTerminalOperation::Input {
                terminal_id,
                data_base64,
            } => {
                let input = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|_| refused("terminal input is not valid base64".to_string()))?;
                RoomTerminalResult::Input {
                    input: service
                        .input(session_id, &terminal_id, &actor, &input)
                        .await?,
                }
            }
            RoomTerminalOperation::Resize {
                terminal_id,
                cols,
                rows,
            } => RoomTerminalResult::Resized {
                terminal: service.resize(session_id, &terminal_id, &actor, cols, rows)?,
            },
            RoomTerminalOperation::ReadOutput {
                terminal_id,
                after_offset,
                wait_ms,
            } => RoomTerminalResult::Output {
                output: service
                    .read_output(
                        session_id,
                        &terminal_id,
                        after_offset,
                        Duration::from_millis(wait_ms),
                    )
                    .await?,
            },
            RoomTerminalOperation::Wait {
                terminal_id,
                watches,
                after_offset,
                timeout_ms,
            } => RoomTerminalResult::Waited {
                waited: service
                    .wait(
                        session_id,
                        &terminal_id,
                        &actor.to_string(),
                        watches,
                        after_offset,
                        Duration::from_millis(timeout_ms),
                    )
                    .await?,
            },
            RoomTerminalOperation::Close { terminal_id } => {
                service.close(session_id, &terminal_id, &actor)?;
                RoomTerminalResult::Closed
            }
        };
        Ok(LocalDaemonResponse::RoomTerminal { result })
    }

    pub(crate) fn end_room_terminals(&self, session_id: &str) {
        self.owned.room_terminals.end_session(session_id);
    }

    fn authorize_room_terminal(
        &self,
        caller: &KernelCaller,
        request: &RoomTerminalRequest,
    ) -> Result<TerminalActor, DaemonError> {
        if !matches!(
            caller.caller_kind,
            KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
        ) {
            return Err(refused(
                "only the Room owner's clients can use its terminals".to_string(),
            ));
        }
        let user_id = caller
            .user_id
            .as_deref()
            .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if session.owner_user_id() != user_id {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: session.owner_user_id().to_string(),
                resource: request.session_id.clone(),
                operation: OPERATION,
            });
        }
        match &request.acting_agent {
            None => Ok(TerminalActor::User),
            Some(agent_id) => match self.owned.agent_store.get_agent(agent_id) {
                Ok(agent) if agent.session_id() == request.session_id => {
                    Ok(TerminalActor::Agent(agent_id.clone()))
                }
                _ => Err(refused(format!(
                    "this Room has no agent `{agent_id}` to type as"
                ))),
            },
        }
    }

    /// Where the Room's agents run their commands on this machine.
    fn room_project_directory(&self, session_id: &str) -> Result<PathBuf, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(PathBuf::from(session.worktree_id()))
    }
}

fn refused(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message,
    }
}
