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
mod lifecycle;
mod policy;
mod process;
mod receipts;
mod scope;
mod window;
pub(crate) use policy::is_sudo_prompt;
pub(crate) use window::sudo_window_minutes;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod process_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;

pub(super) type SudoStore = Arc<std::sync::Mutex<BTreeMap<String, KernelSudoTurn>>>;

/// Ratified owner constants: default window, selectable durations (maximum
/// eight hours per fresh passkey verification) and the expiry warning.
pub(crate) const SUDO_DEFAULT_MINUTES: u32 = 60;
pub(crate) const SUDO_WINDOW_MINUTES: [u32; 4] = [60, 120, 240, 480];
const SUDO_WARNING: Duration = Duration::from_secs(10 * 60);
/// A due warning/expiry handled later than this by the sweep means its
/// dedicated timer did not fire: the kernel alerts instead of staying silent.
const SUDO_TIMER_TOLERANCE: Duration = Duration::from_secs(5);
