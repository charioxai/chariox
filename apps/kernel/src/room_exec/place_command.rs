//! The command that runs a held launch on another of the owner's kernels.
use std::sync::Arc;

use super::gateway_placement::{self, GatewayExecPlacement, GrantedSend};
use super::{error, RoomExecService};
use crate::error::DaemonError;

/// Who asks: a person, or an agent that may place only what it started itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Requester {
    Person,
    Agent(String),
}

impl Requester {
    pub(crate) fn ensure_owns(&self, launch_agent: Option<&str>) -> Result<(), DaemonError> {
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

/// Makes a Room on the other kernel, then hands the held launch to it. Nothing starts
/// anywhere unless the Room exists; a refusal leaves the launch held.
pub(crate) async fn run_on_kernel(
    service: &RoomExecService,
    placement: Placement<'_>,
    requester: &Requester,
    grant: Arc<dyn GrantedSend>,
) -> Result<(), DaemonError> {
    service.ensure_placeable_by(placement.session_id, placement.exec_id, requester)?;
    let target_session_id =
        gateway_placement::create_target_room(&*grant, placement.exec_id, placement.directory)
            .await?;
    service.place(
        placement.session_id,
        placement.exec_id,
        placement.origin_kernel,
        Arc::new(GatewayExecPlacement::new(grant, target_session_id)),
    )
}
