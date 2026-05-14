use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::config::DaemonConfig;

#[derive(Clone)]
pub(crate) struct DaemonConfigProjectionStore {
    config: Arc<StdMutex<DaemonConfig>>,
}

impl DaemonConfigProjectionStore {
    pub(crate) fn new(config: DaemonConfig) -> Self {
        Self {
            config: Arc::new(StdMutex::new(config)),
        }
    }

    pub(crate) fn snapshot(&self) -> DaemonConfig {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .clone()
    }

    pub(crate) fn update(&self, config: DaemonConfig) {
        *self
            .config
            .lock()
            .expect("daemon config projection lock should not be poisoned") = config;
    }
}
