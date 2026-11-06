//! Stop for one utility request: the request's own provider turn or child ends, nothing else of the
//! borrowed agent's provider run.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct UtilityStop(Arc<AtomicBool>);

impl UtilityStop {
    pub(crate) fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
