//! Peers across kernels, sending side: a room agent here messages, steers, cancels or spawns an
//! ordinary agent of a room on another kernel of its owner.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

/// Two kernels in one test process: a request to a registered kernel id reaches its router as
/// the relay would deliver it, after the same-owner admission.
#[cfg(test)]
pub(crate) mod peer_agent_link_for_tests {
    use super::*;
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, LazyLock, Mutex};

    type Answer = Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send>>;
    pub(crate) type Link = Arc<dyn Fn(String, String, RelayPeerRequest) -> Answer + Send + Sync>;

    static LINKS: LazyLock<Mutex<HashMap<String, (String, Link)>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// `kernel` answers under its id and under `name`.
    pub(crate) fn register(kernel: &str, name: &str, link: Link) {
        LINKS
            .lock()
            .unwrap()
            .insert(kernel.to_string(), (name.to_string(), link));
    }

    pub(crate) fn unregister(kernel: &str) {
        LINKS.lock().unwrap().remove(kernel);
    }

    pub(super) fn resolve(kernel_ref: &str) -> Option<String> {
        LINKS
            .lock()
            .unwrap()
            .iter()
            .find(|(id, (name, _))| *id == kernel_ref || name == kernel_ref)
            .map(|(id, _)| id.clone())
    }

    pub(super) fn link(kernel: &str) -> Option<Link> {
        LINKS
            .lock()
            .unwrap()
            .get(kernel)
            .map(|(_, link)| link.clone())
    }
}
