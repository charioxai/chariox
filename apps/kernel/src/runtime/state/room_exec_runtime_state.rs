//! Non-PTY operations enter only through the existing Room terminal gate.
use std::path::Path;
use std::time::Duration;

use super::*;
use crate::local::{RoomTerminalOperation, RoomTerminalResult};

impl KernelRuntimeState {
    pub(super) async fn run_room_exec_operation(
        &self,
        session_id: &str,
        cwd: &Path,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let service = &self.owned.room_execs;
        Ok(match operation {
            RoomTerminalOperation::ExecOpen { command } => {
                self.ensure_room_running(session_id)?;
                let exec_id = service.open(session_id, cwd, &command)?;
                if let Err(ended) = self.ensure_room_running(session_id) {
                    service.end_session(session_id);
                    return Err(ended);
                }
                RoomTerminalResult::ExecOpened { exec_id }
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
