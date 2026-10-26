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

    pub(crate) fn local_owner_user_id(&self) -> String {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .unwrap_or_else(|| crate::session::DEFAULT_LOCAL_USER_ID.into())
    }

    /// Pre-enrollment rooms keep their member identity after the owner enrolls.
    pub(crate) fn room_member_user_id_in(&self, room_owner: &str, user_id: &str) -> String {
        room_member_user_id_in(
            &self.config.lock().expect("daemon config projection lock"),
            room_owner,
            user_id,
        )
    }

    pub(crate) fn workflow_invoker_matches(
        &self,
        kernel_id: &str,
        room_owner: &str,
        user_id: &str,
        invoker: &str,
    ) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        config.daemon_id == kernel_id
            && room_member_user_id_in(&config, room_owner, user_id)
                == room_member_user_id_in(&config, room_owner, invoker)
    }

    /// The enrolled owner, or the `local` identity this owner's pre-enrollment rooms keep: the
    /// terminal reaches those under `local` for membership.
    pub(crate) fn is_kernel_owner(&self, kernel_id: &str, user_id: &str) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        config.daemon_id == kernel_id
            && (user_id == crate::session::DEFAULT_LOCAL_USER_ID
                || config
                    .cloud_relay
                    .as_ref()
                    .is_some_and(|profile| profile.user_id == user_id))
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

fn room_member_user_id_in(config: &DaemonConfig, room_owner: &str, user_id: &str) -> String {
    if room_owner == crate::session::DEFAULT_LOCAL_USER_ID
        && !user_id.is_empty()
        && config
            .cloud_relay
            .as_ref()
            .is_some_and(|profile| profile.user_id == user_id)
    {
        crate::session::DEFAULT_LOCAL_USER_ID.into()
    } else {
        user_id.into()
    }
}
