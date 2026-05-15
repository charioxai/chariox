use std::collections::BTreeMap;

mod outcomes;
mod run_lifecycle;
mod runtime_io;
mod store;
pub(crate) use outcomes::{
    ProviderRunEndedOutcome, ProviderRunLivenessReconciliation, ProviderRunParkedOutcome,
    ProviderRunResumedOutcome, ProviderRunStartedOutcome, ProviderSessionRunsTerminatedOutcome,
};
pub(crate) use runtime_io::ProviderRuntimeBinding;
pub use store::ProviderProcessServiceStore;

use super::{
    LaunchProviderRequest, ProviderNativeInteractionBridge, ProviderRegistry,
    ProviderRunActorMailbox, ProviderRunState, RuntimeProviderRun,
};

pub struct ProviderProcessService {
    registry: ProviderRegistry,
    run_actor_mailbox: ProviderRunActorMailbox,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            run_actor_mailbox: ProviderRunActorMailbox::default(),
            runs: BTreeMap::new(),
            next_run_number: 0,
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: std::sync::Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.run_actor_mailbox.set_native_interaction_bridge(bridge);
    }
}

impl Default for ProviderProcessService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
