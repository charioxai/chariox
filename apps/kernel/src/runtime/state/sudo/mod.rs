//! Hour-scale elevation windows bound to one owner-authorized task, held only
//! in kernel memory (MP-08/MP-10/MP-11 A04). The provider bearer never
//! changes: every use resolves the exact running turn of that work again.
use super::*;
use crate::local::*;
use crate::runtime::kernel_access::error;
use crate::session::{
    PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeInteraction,
    RuntimeInteractionChoice, SessionStatus,
};

mod end_wake;
mod entry;
mod external;
pub(crate) mod leased;
mod lifecycle;
mod policy;
mod process;
mod receipts;
mod scope;
mod window;
pub(crate) use policy::{is_sudo_control, is_sudo_prompt};
pub(crate) use window::sudo_window_minutes;
#[cfg(test)]
pub(crate) use window::SUDO_WINDOW_LENGTH_FOR_TEST;
#[cfg(test)]
mod catalog_tests;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod process_tests;
#[cfg(test)]
mod queued_command_tests;
#[cfg(test)]
mod relaunch_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;

pub(super) type SudoStore = Arc<std::sync::Mutex<BTreeMap<String, KernelSudoTurn>>>;

/// MP-08/MP-10/MP-11: read-only projection access for App snapshots. Only
/// kernel runtime modules can obtain the store that mutates window authority.
#[derive(Clone, Default)]
pub(crate) struct SudoWindowProjection(SudoStore);

impl SudoWindowProjection {
    pub(super) fn store(&self) -> SudoStore {
        self.0.clone()
    }

    pub(crate) fn windows_for_session(&self, session_id: &str) -> Vec<KernelSudoTurn> {
        self.0
            .lock()
            .expect("access state poisoned")
            .values()
            .filter(|turn| turn.session_id == session_id && turn.deadline.is_some())
            .cloned()
            .collect()
    }
}

/// Ratified owner constants: default window, selectable durations (maximum
/// eight hours per fresh passkey verification) and the expiry warning.
pub(crate) const SUDO_DEFAULT_MINUTES: u32 = 60;
/// Listed to the agent only while its window is live.
pub(crate) const SUDO_TOOL: &str = "chariox_kernel_request";
pub(crate) const SUDO_WINDOW_MINUTES: [u32; 4] = [60, 120, 240, 480];
const SUDO_WARNING: Duration = Duration::from_secs(10 * 60);
/// A due warning/expiry handled later than this by the sweep means its
/// dedicated timer did not fire: the kernel alerts instead of staying silent.
const SUDO_TIMER_TOLERANCE: Duration = Duration::from_secs(5);
