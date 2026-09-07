use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::session::EnvironmentActionState;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult,
};
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
        let mut cancellation_delivered = false;
        loop {
            tokio::select! {
                result = &mut execution => return result,
                _ = poll.tick() => {
                    if cancellation_delivered { continue; }
                    let cancel = match self.room_environment_snapshot(session_id) {
                        Ok(room) => room.actions.iter().find(|action| action.action_id == action_id)
                            .is_none_or(|action| action.cancellation_requested || action.state != EnvironmentActionState::Running),
                        Err(_) => false,
                    };
                    if !cancel { continue; }
                    let cancellation = self.room_browser_controller_command(session_id,
                        RoomBrowserControllerCommand::CancelAction { execution_id: execution_id.to_string() });
                    // The worker may not have registered the execution yet, or
                    // the cancellation acknowledgement may be lost. Retry the
                    // same identity only until the worker accepts it.
                    tokio::select! {
                        result = &mut execution => return result,
                        response = cancellation => {
                            cancellation_delivered = matches!(response,
                                Ok(RoomBrowserControllerResult::CancellationRequested { accepted: true }));
                        },
                    }
                }
            }
        }
    }
}
