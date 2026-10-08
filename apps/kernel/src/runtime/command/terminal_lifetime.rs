//! MD-3: transport-owned cancellation, never serialized or accepted from callers.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalLifetime(Arc<AtomicBool>);
impl PartialEq for TerminalLifetime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl TerminalLifetime {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn is_live(&self) -> bool {
        !self.0.load(Ordering::Acquire)
    }
}
