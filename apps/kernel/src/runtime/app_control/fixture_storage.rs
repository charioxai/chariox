//! Tests only: App storage without the platform's. Linux App storage belongs
//! to the root storage helper, which kernel tests and CI do not run (and whose
//! client refuses root), so uninstalling with `delete_data` fails there. A
//! test kernel given this fixture deletes from it instead and records each
//! deletion, so App lifecycles that delete data (a deployment's copy) run on
//! every platform. Production has no such path: this module is `cfg(test)`.
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
