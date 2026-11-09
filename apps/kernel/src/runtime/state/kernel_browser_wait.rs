//! MP-08/MP-09/MP-10/MP-11: an agent's `wait`. The host compares the page in slices and the kernel joins them,
//! so a wait of 30 seconds never holds the host for more than one slice and the agent never polls.
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::runtime::kernel_browser_host::KernelBrowserAdmission;
use serde_json::Value;
use std::time::{Duration, Instant};

/// What one host call may take; viewers' and other agents' calls queue behind at most this long.
const SLICE_MS: u64 = 1_500;
/// A host that answers a slice early without a match cannot turn the wait into a spin.
const MIN_GAP: Duration = Duration::from_millis(50);

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_wait(
        &self,
        user: &str,
        admission: KernelBrowserAdmission,
        params: Value,
    ) -> Result<Value, DaemonError> {
        let started = Instant::now();
        let total = Duration::from_millis(
            params["timeout_ms"]
                .as_u64()
                .unwrap_or(0)
                .min(super::kernel_browser_tool_ops::WAIT_LIMIT_MS.into()),
        );
        loop {
            let slice = total
                .saturating_sub(started.elapsed())
                .min(Duration::from_millis(SLICE_MS));
            let mut slice_params = params.clone();
            slice_params["timeout_ms"] = (slice.as_millis() as u64).into();
            let mut state = self
                .kernel_browser_operation_admitted(
                    user,
                    Some(admission.clone()),
                    "host.browser",
                    slice_params,
                )
                .await?;
            if state["matched"] == true || started.elapsed() >= total {
                state["waited_ms"] = (started.elapsed().as_millis() as u64).into();
                return Ok(state);
            }
            tokio::time::sleep(MIN_GAP).await;
        }
    }
}
