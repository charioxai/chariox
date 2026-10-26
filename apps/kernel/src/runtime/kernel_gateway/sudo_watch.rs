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
use crate::local::{
    LocalDaemonRequest, RenewKernelSudoRequesterRequest, RevokeKernelAccessGrantRequest,
};
use crate::runtime::kernel_access::REMOTE_REQUESTER_RENEW_EVERY;

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

    /// Keeps the target's lease on the requester fresh while the grant lives, then tells it
    /// the grant ended. A renewal or the end report that never arrives (this kernel killed,
    /// a partition) is covered by the target's own lease, which lapses without renewals.
    async fn report_grant_end(self: Arc<Self>, target: String, origin: Origin) {
        let mut next_renewal = tokio::time::Instant::now() + REMOTE_REQUESTER_RENEW_EVERY;
        let mut unknown_answers = 0_u32;
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
            if tokio::time::Instant::now() >= next_renewal {
                next_renewal += REMOTE_REQUESTER_RENEW_EVERY;
                // The first renewal can overtake a request still being delivered, so only a
                // target that twice answers it does not know the requester ends the watch.
                if self.renew_lease(&target, &origin).await {
                    unknown_answers = 0;
                } else {
                    unknown_answers += 1;
                    if unknown_answers >= 2 {
                        self.unwatch(&target, &origin);
                        return;
                    }
                }
            }
        }
        let request = LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: Some(origin.grant_id.clone()),
        });
        for attempt in 1..=REPORT_ATTEMPTS {
            let command_id = next_command_id("gw-grant-ended");
            match self
                .forward_grant_ended(&target, &origin, &command_id, &request)
                .await
            {
                Ok(_) => break,
                Err(error) if attempt == REPORT_ATTEMPTS => {
                    tracing::warn!(code = error.code(), detail = %error.message(), target = %target, "could not tell the kernel a sudo requester's grant ended; its lease lapses without renewals");
                }
                Err(_) => tokio::time::sleep(REPORT_RETRY).await,
            }
        }
        self.unwatch(&target, &origin);
    }

    /// False only when the target answered that it does not know the requester; a lost
    /// renewal counts as renewed, since the next one may get through.
    async fn renew_lease(&self, target: &str, origin: &Origin) -> bool {
        let request =
            LocalDaemonRequest::RenewKernelSudoRequester(RenewKernelSudoRequesterRequest {});
        let command_id = next_command_id("gw-lease");
        match self
            .forward_grant_ended(target, origin, &command_id, &request)
            .await
        {
            Ok(value) => value
                .get("KernelSudoRequesterRenewed")
                .and_then(|renewed| renewed.get("renewed"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            Err(_) => true,
        }
    }

    fn unwatch(&self, target: &str, origin: &Origin) {
        self.sudo_watch
            .watched
            .lock()
            .expect("sudo watch poisoned")
            .remove(&(target.to_string(), origin.grant_id.clone()));
    }
}

fn next_command_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        REPORT_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
