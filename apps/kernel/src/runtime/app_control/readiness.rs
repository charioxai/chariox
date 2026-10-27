//! Callable readiness for an already accepted lifecycle start.
use super::AppControlService;
use crate::runtime::app_worker::AppWorkerLease;
use std::time::Duration;
use tokio::time::Instant;

impl AppControlService {
    pub(crate) async fn wait_for_app_lease(
        &self,
        owner: &str,
        installation: &str,
        deadline: Instant,
    ) -> Option<AppWorkerLease> {
        loop {
            if let Some(lease) = self.active_app_lease(owner, installation) {
                return Some(lease);
            }
            // Durable Starting is narrower than an accepted start: the owner
            // is retained before its claim and through callable publication.
            if !self.lifecycle().has_pending_owner(owner, installation)
                || Instant::now() >= deadline
            {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
