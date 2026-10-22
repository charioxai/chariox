//! Tells a target kernel when the grant behind a forwarded sudo request ends.
//!
//! The window and the pending popup live on the target, bound to the requesting grant.
//! That grant lives here, so this kernel reports its end: the target then ends the
//! request through the same path a local grant's end takes.
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{KernelGateway, Origin};
use crate::local::{LocalDaemonRequest, RevokeKernelAccessGrantRequest};

const GRANT_POLL: Duration = Duration::from_millis(500);
const REPORT_ATTEMPTS: u32 = 4;
const REPORT_RETRY: Duration = Duration::from_secs(3);
static REPORT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(super) struct SudoWatch {
    watched: Mutex<HashSet<(String, String)>>,
}

impl KernelGateway {
    /// Starts reporting the end of `origin`'s grant to `target`, once per pair.
    pub(crate) fn watch_sudo_grant(self: &Arc<Self>, target: &str, origin: &Origin) {
        let pair = (target.to_string(), origin.grant_id.clone());
        if !self
            .sudo_watch
            .watched
            .lock()
            .expect("sudo watch poisoned")
            .insert(pair.clone())
        {
            return;
        }
        let origin = Origin {
            grant_id: origin.grant_id.clone(),
            harness: origin.harness.clone(),
        };
        tokio::spawn(Arc::clone(self).report_grant_end(pair.0, origin));
    }

    async fn report_grant_end(self: Arc<Self>, target: String, origin: Origin) {
        loop {
            tokio::time::sleep(GRANT_POLL).await;
            self.runtime_state.sweep_kernel_access();
            if self
                .runtime_state
                .access_grant_summary(&origin.grant_id)
                .is_none()
            {
                break;
            }
        }
        let request = LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: Some(origin.grant_id.clone()),
        });
        for attempt in 1..=REPORT_ATTEMPTS {
            let command_id = format!(
                "gw-grant-ended-{}",
                REPORT_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            match self
                .forward_grant_ended(&target, &origin, &command_id, &request)
                .await
            {
                Ok(_) => break,
                Err(error) if attempt == REPORT_ATTEMPTS => {
                    tracing::warn!(code = error.code(), detail = %error.message(), target = %target, "could not tell the kernel a sudo requester's grant ended; its window ends at its own deadline");
                }
                Err(_) => tokio::time::sleep(REPORT_RETRY).await,
            }
        }
        self.sudo_watch
            .watched
            .lock()
            .expect("sudo watch poisoned")
            .remove(&(target, origin.grant_id));
    }
}
