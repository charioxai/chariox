use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::session::EnvironmentActionState;
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use std::future::Future;
use std::time::Duration;

impl KernelRuntimeState {
    pub(super) async fn await_cancellable_browser_action<T>(
        &self,
        session_id: &str,
        action_id: &str,
        execution_id: &str,
        execution: impl Future<Output = Result<T, DaemonError>>,
    ) -> Result<T, DaemonError> {
        tokio::pin!(execution);
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! {
                result = &mut execution => return result,
                _ = poll.tick() => {
                    let cancel = self.room_environment_snapshot(session_id)
                        .map(|room| room.actions.iter().find(|action| action.action_id == action_id)
                            .is_none_or(|action| action.cancellation_requested || action.state != EnvironmentActionState::Running))
                        .unwrap_or(true);
                    if !cancel { continue; }
                    let cancellation = self.room_browser_controller_command(session_id,
                        RoomBrowserControllerCommand::CancelAction { execution_id: execution_id.to_string() });
                    // The worker may not have registered the execution yet, or
                    // the cancellation acknowledgement may be lost. Retry the
                    // same identity while retaining the original execution.
                    tokio::select! {
                        result = &mut execution => return result,
                        _ = cancellation => {},
                    }
                }
            }
        }
    }
}
