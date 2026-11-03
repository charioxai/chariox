//! Tests only: record App storage deletion without the platform storage helper.
//! Kernel tests and CI do not run the Linux root storage helper, which refuses
//! root callers. A test kernel uses this fixture instead so data-deleting App
//! lifecycles run on every platform. Production never uses this `cfg(test)` module.
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub(crate) struct FixtureAppStorage(Arc<Mutex<Vec<(String, String)>>>);

impl FixtureAppStorage {
    pub(crate) fn delete(&self, owner: &str, installation: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((owner.to_owned(), installation.to_owned()));
    }

    /// Each deleted (owner, installation) storage, in order.
    pub(crate) fn deleted(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
