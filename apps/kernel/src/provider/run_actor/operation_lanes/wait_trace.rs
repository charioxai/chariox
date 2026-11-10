//! MP-08 / MP-09 / MP-10 / MP-11: names the holder of a hung run lane in debug builds.
//! With `CHARIOX_LANE_WAIT_TRACE=1`, each acquire records the kernel call sites that took
//! the lane, and a wait longer than two seconds logs the waiter's and the holder's.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore};

const REPORT_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
pub(super) struct LaneWaitTrace {
    enabled: bool,
    holders: Mutex<BTreeMap<String, String>>,
    #[cfg(test)]
    reports: Mutex<Vec<String>>,
}

impl LaneWaitTrace {
    pub(super) fn from_env() -> Self {
        Self {
            enabled: std::env::var_os("CHARIOX_LANE_WAIT_TRACE").is_some_and(|value| value == "1"),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn enabled_for_tests() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    pub(super) async fn acquire(
        &self,
        provider_run_id: &str,
        semaphore: Arc<Semaphore>,
    ) -> Result<OwnedSemaphorePermit, AcquireError> {
        if !self.enabled {
            return semaphore.acquire_owned().await;
        }
        let permit = match tokio::time::timeout(
            REPORT_AFTER,
            Arc::clone(&semaphore).acquire_owned(),
        )
        .await
        {
            Ok(permit) => permit,
            Err(_) => {
                self.report(provider_run_id);
                semaphore.acquire_owned().await
            }
        };
        self.record_holder(provider_run_id);
        permit
    }

    pub(super) fn record_holder(&self, provider_run_id: &str) {
        if self.enabled {
            self.holders
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(provider_run_id.to_string(), kernel_call_sites());
        }
    }

    fn report(&self, provider_run_id: &str) {
        // The lane has one permit: its last recorded acquirer still holds it.
        let holder = self
            .holders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(provider_run_id)
            .cloned()
            .unwrap_or_default();
        let waiter = kernel_call_sites();
        crate::logging::warn_with_fields(
            "daemon.provider_run_lane",
            "run lane wait over 2 s",
            serde_json::json!({"provider_run_id": provider_run_id, "waiter": waiter, "holder": holder}),
        );
        #[cfg(test)]
        self.reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!(
                "{provider_run_id} waiter: {waiter} holder: {holder}"
            ));
    }

    #[cfg(test)]
    pub(super) fn reports(&self) -> Vec<String> {
        self.reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Kernel frames by symbol, with their source lines where the build has them.
fn kernel_call_sites() -> String {
    std::backtrace::Backtrace::force_capture()
        .to_string()
        .lines()
        .map(str::trim)
        .filter(|line| {
            let symbol = line.split_once(": ").map_or(*line, |(_, symbol)| symbol);
            symbol.starts_with("chariox_kernel::")
                || symbol.starts_with("<chariox_kernel::")
                || line.contains("apps/kernel/src/")
        })
        .filter(|line| {
            !line.contains("ProviderRunOperationLanes")
                && !line.contains("wait_trace::")
                && !line.contains("operation_lanes.rs")
                && !line.contains("wait_trace.rs")
        })
        .take(12)
        .collect::<Vec<_>>()
        .join(" <- ")
}
