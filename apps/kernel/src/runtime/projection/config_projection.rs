use crate::config::{DaemonConfig, EventGeneratorManagementTarget};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

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

    /// Atomically merge one registry-issued owner-scoped capability into the
    /// current projection. Callers may resolve different owners concurrently;
    /// replacing a caller's stale full snapshot would otherwise erase tokens
    /// resolved by another owner or generator.
    pub(crate) fn merge_event_generator_management_target(
        &self,
        generator_id: &str,
        resolved: EventGeneratorManagementTarget,
    ) {
        let mut config = self
            .config
            .lock()
            .expect("daemon config projection lock should not be poisoned");
        let target = config
            .event_generator_management_targets
            .entry(generator_id.to_string())
            .or_insert_with(|| resolved.clone());
        if target.url.is_empty() {
            target.url = resolved.url;
        }
        if target.token.is_empty() {
            target.token = resolved.token;
        }
        if target.expires_at_ms.is_none() {
            target.expires_at_ms = resolved.expires_at_ms;
        }
        if target.owner_ids.is_none() {
            target.owner_ids = resolved.owner_ids;
        }
        let scoped = target.owner_scoped.get_or_insert_with(BTreeMap::new);
        if let Some(incoming) = resolved.owner_scoped {
            scoped.extend(incoming);
        }
    }
}
