//! A remote job's worker lifecycle, saved in its record. Files are accepted and the worker is
//! released only from the stage that proves the command ended; every other move is refused.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RemoteStage {
    /// No file waits on a worker: a local job, or a remote one before its return plan.
    #[default]
    Idle,
    /// The return plan and the worker's hold are saved; the command is not yet opened.
    Planned,
    /// The command is open on the worker under this Room and execution; its end is not known.
    Running { room: String, exec_id: String },
    /// The command ended with this outcome; its files may still be on the worker.
    Finished { outcome: JobOutcome },
    /// The files are home or given up; only the worker's release is left.
    Settled,
}

/// What happens to a worker's job.
pub(super) enum StageEvent {
    /// The plan and the hold are saved.
    Plan,
    /// The command was opened.
    Open { room: String, exec_id: String },
    /// The command's terminal outcome was read from the worker.
    End(JobOutcome),
    /// Every file of the finished command is home.
    Home,
    /// The command is stopped and the worker is being reaped, or the files are given up.
    GiveUp,
}

impl RemoteStage {
    pub(super) fn next(&self, event: StageEvent) -> Result<Self, DaemonError> {
        match (self, event) {
            (Self::Idle, StageEvent::Plan) => Ok(Self::Planned),
            (Self::Planned, StageEvent::Open { room, exec_id }) => {
                Ok(Self::Running { room, exec_id })
            }
            (Self::Running { .. }, StageEvent::End(outcome)) => Ok(Self::Finished { outcome }),
            (Self::Finished { .. }, StageEvent::Home) => Ok(Self::Settled),
            (Self::Planned | Self::Running { .. } | Self::Finished { .. }, StageEvent::GiveUp) => {
                Ok(Self::Settled)
            }
            _ => Err(ledger::error("The job's worker is not in a state for this")),
        }
    }

    /// The worker may hold files or a running command that its release would destroy.
    pub(super) fn holds_worker(&self) -> bool {
        matches!(
            self,
            Self::Planned | Self::Running { .. } | Self::Finished { .. }
        )
    }

    /// The command's end is not known, so nothing may be collected or released yet.
    pub(super) fn awaits_command(&self) -> bool {
        matches!(self, Self::Planned | Self::Running { .. })
    }
}

impl Record {
    pub(super) fn advance(&mut self, event: StageEvent) -> Result<(), DaemonError> {
        self.remote = self.remote.next(event)?;
        Ok(())
    }
}
