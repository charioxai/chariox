//! One-turn elevation, held only in kernel memory. The provider bearer never
//! changes: every use resolves it to the exact active run and prompt again.
use super::*;
use crate::local::*;
use crate::runtime::kernel_access::error;
use crate::session::{
    PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeInteraction,
    RuntimeInteractionChoice, SessionStatus,
};

mod entry;
mod external;
mod lifecycle;
mod policy;
mod receipts;
mod process;
pub(crate) use policy::is_sudo_prompt;
pub(crate) use receipts::sudo_approval_receipt;
#[cfg(test)]
mod tests;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod process_tests;

pub(super) type SudoStore = Arc<std::sync::Mutex<BTreeMap<String, KernelSudoTurn>>>;
