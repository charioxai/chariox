use std::sync::{Arc, Mutex, Weak};

use crate::error::DaemonError;
use crate::session::RuntimeInteraction;

pub(crate) trait ProviderNativeInteractionBridge: Send + Sync {
    /// Called when the provider creates the request, before any blocking handoff.
    fn capture_turn_origin(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _provider_run_id: &str,
    ) -> Option<crate::session::NativeInteractionOrigin> {
        None
    }

    fn request_blocking(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProviderNativeInteractionResolution {
    pub(crate) status: String,
    pub(crate) choice_id: Option<String>,
    pub(crate) reply: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderNativeInteractionBridgeStore {
    inner: Arc<Mutex<Option<Weak<dyn ProviderNativeInteractionBridge>>>>,
}

impl ProviderNativeInteractionBridgeStore {
    pub(super) fn read(&self) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
        self.inner
            .lock()
            .expect("provider native interaction bridge mutex poisoned")
            .as_ref()
            .and_then(Weak::upgrade)
    }

    pub(crate) fn set(&self, bridge: Arc<dyn ProviderNativeInteractionBridge>) {
        *self
            .inner
            .lock()
            .expect("provider native interaction bridge mutex poisoned") =
            Some(Arc::downgrade(&bridge));
    }
}
