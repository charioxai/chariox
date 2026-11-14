//! Non-PTY operations enter only through the existing Room terminal gate.
use std::path::Path;
use std::time::Duration;

use super::*;
use crate::local::{RoomTerminalOperation, RoomTerminalResult};

impl KernelRuntimeState {
    pub(crate) fn authorize_queued_exec(
        &self,
        intent: &crate::room_exec::LaunchIntent,
    ) -> Result<(), DaemonError> {
        self.authorize_current_external_command()?;
        self.authorize_gateway_requester()?;
        self.ensure_room_running(&intent.session_id)?;
        let session = self.owned.session_store.get_session(&intent.session_id)?;
        if session.owner_user_id() != intent.owner_user_id {
            return Err(crate::room_exec::error(
                "exec_owner_changed",
                "Room ownership changed before the command started",
            ));
        }
        Ok(())
    }

    pub(super) async fn run_room_exec_operation(
        &self,
        session_id: &str,
        cwd: &Path,
        actor: crate::room_terminal::TerminalActor,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let service = &self.owned.room_execs;
        Ok(match operation {
            RoomTerminalOperation::ExecList => RoomTerminalResult::ExecListed {
                execs: service.list(session_id),
            },
            operation @ (RoomTerminalOperation::ExecOpen { .. } | RoomTerminalOperation::ExecOpenJob { .. }) => {
                let (command, needs) = match operation {
                    RoomTerminalOperation::ExecOpen { command } => (command, None),
                    RoomTerminalOperation::ExecOpenJob { command, needs } => (command, Some(needs)),
                    _ => unreachable!(),
                };
                self.ensure_room_running(session_id)?;
                self.authorize_current_external_command()?;
                let session = self.owned.session_store.get_session(session_id)?;
                let agent = match actor { crate::room_terminal::TerminalActor::User => None, crate::room_terminal::TerminalActor::Agent(id) => Some(id) };
                let exec_id = if let Some(needs) = needs {
                    service.open_job_as(session_id, session.owner_user_id(), cwd, &command, &needs, Some(self.clone()), agent)?
                } else {
                    service.open_as(session_id, session.owner_user_id(), cwd, &command, Some(self.clone()), agent)?
                };
                if let Err(ended) = self.ensure_room_running(session_id) {
                    service.end_session(session_id);
                    return Err(ended);
                }
                RoomTerminalResult::ExecOpened { exec_id }
            }
            RoomTerminalOperation::ExecStartNow { exec_id } => {
                self.authorize_current_external_command()?;
                service.start_now(session_id, &exec_id)?;
                RoomTerminalResult::ExecOpened { exec_id }
            }
            RoomTerminalOperation::ExecPlace { .. } => {
                return Err(crate::room_exec::error(
                    "exec_place_not_here",
                    "a held launch is placed by the kernel that holds it, not through a worker or another kernel's gateway",
                ))
            }
            RoomTerminalOperation::ExecWrite {
                exec_id,
                offset,
                data_base64,
            } => RoomTerminalResult::ExecInput {
                next_offset: service.write(session_id, &exec_id, offset, &data_base64)?,
            },
            RoomTerminalOperation::ExecStdinClose { exec_id, offset } => {
                RoomTerminalResult::ExecInput {
                    next_offset: service.close_stdin(session_id, &exec_id, offset)?,
                }
            }
            RoomTerminalOperation::ExecRead {
                exec_id,
                after_cursor,
                wait_ms,
            } => RoomTerminalResult::ExecOutput {
                output: service
                    .read(
                        session_id,
                        &exec_id,
                        after_cursor,
                        Duration::from_millis(wait_ms),
                    )
                    .await?,
            },
            RoomTerminalOperation::ExecCancel { exec_id } => {
                service.cancel(session_id, &exec_id)?;
                RoomTerminalResult::ExecCancelled
            }
            RoomTerminalOperation::ExecClose { exec_id } => {
                service.close(session_id, &exec_id)?;
                RoomTerminalResult::ExecClosed
            }
            _ => unreachable!("only binary exec operations enter here"),
        })
    }
}
