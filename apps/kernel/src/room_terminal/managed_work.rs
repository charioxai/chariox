//! A managed kernel must not stop under a running terminal command, and the
//! stop fence closes terminal commands like it closes agent prompts. This
//! module is the terminals' side of that contract; it knows no terminal.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use crate::error::DaemonError;
use crate::runtime::state::{ManagedKernelAdmissionGuard, ManagedKernelQuiescenceGate};

struct Link {
    gate: Arc<ManagedKernelQuiescenceGate>,
    /// Records the kernel's activity transition once running work changed.
    work_changed: Weak<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
pub(super) struct ManagedWork {
    link: OnceLock<Link>,
    working: AtomicBool,
}

impl ManagedWork {
    pub(super) fn link(
        &self,
        gate: Arc<ManagedKernelQuiescenceGate>,
        work_changed: Weak<dyn Fn() + Send + Sync>,
    ) {
        let _ = self.link.set(Link { gate, work_changed });
    }

    /// Hold the guard while a command is committed: a stop cannot be reserved
    /// in between, and none is admitted once it is. Ordinary kernels have no
    /// fence.
    pub(super) fn admission(&self) -> Result<Option<ManagedKernelAdmissionGuard<'_>>, DaemonError> {
        self.link
            .get()
            .map(|link| link.gate.admission_guard())
            .transpose()
    }

    /// Tell the kernel when the answer to "is a command running" changed.
    /// Called without any terminal lock held.
    pub(super) fn report(&self, is_working: impl FnOnce() -> bool) {
        let Some(link) = self.link.get() else {
            return;
        };
        let is_working = is_working();
        if self.working.swap(is_working, Ordering::AcqRel) != is_working {
            if let Some(work_changed) = link.work_changed.upgrade() {
                work_changed();
            }
        }
    }
}
