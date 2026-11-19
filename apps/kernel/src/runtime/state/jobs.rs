//! MP-08 / MP-09 / MP-10 / MP-11: durable commands, independent of a provider tool timeout.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence};
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, AgentWake, WakeEnd,
};
use crate::local::{JobOutcome, JobPlacement, JobRanOn, JobResult, RunJobRequest};
use tokio::sync::watch;

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod approval_tests;
#[cfg(test)]
mod cleanup_tests;
mod control;
mod driver;
mod execute;
mod held;
#[cfg(test)]
mod held_tests;
mod local;
#[cfg(test)]
mod marker_tests;
mod never_fit;
#[cfg(test)]
mod never_fit_tests;
#[cfg(test)]
mod off_tests;
mod pending_return;
mod projection;
mod query;
mod record;
mod remote;
mod requests;
#[cfg(test)]
mod return_tests;
mod stage;
#[cfg(test)]
mod stage_tests;
mod summary;
#[cfg(test)]
mod summary_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
use driver::completion_payload;
use projection::effective_needs;
use record::{public_record, save_record, Record};
use requests::validate;
use stage::{RemoteStage, StageEvent};

pub(super) const KIND: &str = "job";
const TAIL_BYTES: usize = 64 * 1024;

pub(crate) use local::LocalJobHandover;

struct Pending {
    state: KernelRuntimeState,
    record: Record,
}

pub(super) struct JobDriver {
    /// The state this driver was built from; it does not yet hold the driver itself.
    state: KernelRuntimeState,
    /// This driver, so recovery can run a job with a state that holds it.
    this: std::sync::OnceLock<std::sync::Weak<JobDriver>>,
    /// The kernel's command router, bound at startup, so recovery can reach the job gateway.
    router: std::sync::OnceLock<std::sync::Weak<crate::runtime::router::CommandRouter>>,
    pending: std::sync::Mutex<BTreeMap<String, Pending>>,
    live: Arc<std::sync::Mutex<BTreeMap<String, watch::Sender<bool>>>>,
    local: Arc<local::LocalJobs>,
    remote_requested: std::sync::Mutex<BTreeSet<String>>,
    owner_stops: std::sync::Mutex<BTreeSet<String>>,
    remote_samples:
        std::sync::Mutex<BTreeMap<String, Vec<crate::runtime::process_resources::SessionProcess>>>,
    remote_origins: std::sync::Mutex<BTreeMap<String, crate::runtime::kernel_gateway::Origin>>,
    /// Grants to workers that still hold files not yet returned; rebuilt from the saved plan after a restart.
    return_grants: std::sync::Mutex<
        BTreeMap<String, Arc<dyn crate::room_exec::gateway_placement::GrantedSend>>,
    >,
}
