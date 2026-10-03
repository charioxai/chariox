use super::{
    EnvironmentActionState, EnvironmentComponent, EnvironmentComponentHealthState,
    EnvironmentEventKind, EnvironmentLifecycle, RoomEnvironment,
};

impl RoomEnvironment {
    /// A kernel restart loses execution ownership, not the physical browser or its Tab identities.
    pub(crate) fn reconcile_after_kernel_restart(&mut self) {
        if matches!(
            self.lifecycle,
            EnvironmentLifecycle::Stopped | EnvironmentLifecycle::Failed
        ) {
            return;
        }
        self.runtime_generation = self.runtime_generation.saturating_add(1);
        self.browser_controller_recovering = false;
        self.element_references.clear();
        self.clear_pointers();
        let interrupted = self
            .action_ledger
            .invalidate_after_kernel_restart(&self.actors);
        for action_id in interrupted {
            self.emit_action_changed(&action_id, EnvironmentActionState::Failed);
        }
        self.emit(EnvironmentEventKind::InputOwnershipChanged);
        self.lifecycle = EnvironmentLifecycle::Degraded;
        self.emit(EnvironmentEventKind::RuntimeInvalidated);
        self.emit(EnvironmentEventKind::LifecycleChanged {
            lifecycle: self.lifecycle,
        });
        self.update_component_health(
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Starting,
            Some("kernel_restarted"),
        );
        self.update_component_health(
            EnvironmentComponent::Browser,
            EnvironmentComponentHealthState::Starting,
            None,
        );
    }
}

// InputTarget has a tagged wire representation, so it cannot be a JSON object key.
pub(super) mod target_map {
    use super::super::InputTarget;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<InputTarget, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<InputTarget, String>, D::Error> {
        let entries = Vec::<(InputTarget, String)>::deserialize(deserializer)?;
        let count = entries.len();
        let map: BTreeMap<_, _> = entries.into_iter().collect();
        if map.len() != count {
            return Err(serde::de::Error::custom("duplicate input target"));
        }
        Ok(map)
    }
}
