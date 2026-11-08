//! The command that runs a held launch on another of the owner's kernels.
use std::sync::Arc;

use super::gateway_placement::GrantedSend;
use super::{error, RoomExecService};
use crate::error::DaemonError;

/// Who asks: a person, or an agent that may place only what it started itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Requester {
    Person,
    Agent(String),
}

impl Requester {
    pub(super) fn ensure_owns(&self, launch_agent: Option<&str>) -> Result<(), DaemonError> {
        match self {
            Self::Person => Ok(()),
            Self::Agent(id) if launch_agent == Some(id.as_str()) => Ok(()),
            Self::Agent(_) => Err(error(
                "exec_not_yours",
                "an agent runs on another kernel only a command it started itself; ask the person or agent that started this one",
            )),
        }
    }
}

pub(crate) struct Placement<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) exec_id: &'a str,
    /// The Room directory on the other kernel.
    pub(crate) directory: &'a str,
    /// What the other kernel calls this one: it labels the launch's process with it.
    pub(crate) origin_kernel: &'a str,
}

pub(crate) async fn run_on_kernel(
    _service: &RoomExecService,
    _placement: Placement<'_>,
    _requester: &Requester,
    _grant: Arc<dyn GrantedSend>,
) -> Result<(), DaemonError> {
    Err(error(
        "exec_place_unavailable",
        "running a held launch on another kernel is not wired yet",
    ))
}
