//! MP-08/MP-10: protocol 556 parked Browser state read. A viewer holds one `StateWait` instead of
//! polling `State`; the host says when a read could differ and the reply's content decides.
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::runtime::command::KernelCommand;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const MAX_WAIT_MS: u32 = 2_000;
/// A woken read repeats no sooner than this, so a busy page cannot turn the wait into a poll.
const MIN_GAP: Duration = Duration::from_millis(100);
const NO_SIGNAL_POLL: Duration = Duration::from_millis(250);

/// `<content digest>.<host signal>`: opaque to clients, which echo it back as `after`.
fn revision(state: &Value, signal: Option<u64>) -> String {
    let digest = Sha256::digest(state.to_string().as_bytes());
    format!("{}.{}", hex_prefix(&digest), signal.unwrap_or(0))
}
fn hex_prefix(digest: &[u8]) -> String {
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn parse(after: &str) -> Option<(&str, u64)> {
    let (digest, signal) = after.split_once('.')?;
    (digest.len() == 16).then_some(())?;
    Some((digest, signal.parse().ok()?))
}

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_state_wait(
        &self,
        caller: &KernelCommand,
        after: &str,
        wait_ms: u32,
    ) -> Result<Value, DaemonError> {
        if wait_ms > MAX_WAIT_MS {
            return Err(host_error(format!(
                "MP-08: state wait exceeds {MAX_WAIT_MS} ms"
            )));
        }
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let known = parse(after);
        let deadline = Instant::now() + Duration::from_millis(wait_ms.into());
        let mut signal = known.map(|(_, signal)| signal);
        loop {
            let started = Instant::now();
            let remaining = deadline.saturating_duration_since(started);
            match (signal, known) {
                (Some(after_signal), _) => {
                    let (host, user, wait) = (
                        self.owned.kernel_browser_host.clone(),
                        user.clone(),
                        remaining.as_millis() as u64,
                    );
                    // The wait holds no protection barrier: Vault insertion never waits on a parked viewer.
                    tokio::task::spawn_blocking(move || {
                        host.state_signal(&user, Some(after_signal), wait)
                    })
                    .await
                    .map_err(|_| host_error("MP-08: state wait interrupted".into()))?
                    .map_err(host_error)?;
                }
                // A host without a signal still gets a bounded wait, never a spin.
                (None, Some(_)) => tokio::time::sleep(remaining.min(NO_SIGNAL_POLL)).await,
                (None, None) => {}
            }
            self.refresh_user_domain_grants();
            let admission = self
                .owned
                .kernel_browser_host
                .admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
            let mut state = self
                .kernel_browser_operation_admitted(
                    &user,
                    Some(admission),
                    "host.browser",
                    serde_json::json!({"op":"state","state_wait":true,"observed_by":actor}),
                )
                .await?;
            signal = state
                .as_object_mut()
                .and_then(|object| object.remove("_state_signal"))
                .and_then(|value| value.as_u64());
            if let Some(object) = state.as_object_mut() {
                object.extend(
                    self.user_domain_window_projection(&user)
                        .as_object()
                        .unwrap()
                        .clone(),
                );
            }
            let current = revision(&state, signal);
            let changed = known.is_none_or(|(digest, _)| !current.starts_with(digest));
            if changed || Instant::now() >= deadline {
                state["revision"] = current.into();
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                self.restore_user_app_tabs(&user);
                return Ok(state);
            }
            if let Some(rest) = MIN_GAP.checked_sub(started.elapsed()) {
                tokio::time::sleep(rest).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn revision_names_the_content_and_the_host_signal() {
        let state = json!({"generation":1,"tabs":[{"tab_id":"a","title":"x"}]});
        let first = revision(&state, Some(7));
        let (digest, signal) = parse(&first).unwrap();
        assert_eq!(signal, 7);
        assert!(
            revision(&state, Some(9)).starts_with(digest),
            "the signal alone is not content"
        );
        let other = json!({"generation":1,"tabs":[{"tab_id":"a","title":"y"}]});
        assert!(
            !revision(&other, Some(7)).starts_with(digest),
            "a retitled Tab is a new revision"
        );
        assert_eq!(parse(&revision(&state, None)).unwrap().1, 0);
    }

    #[test]
    fn a_caller_with_no_valid_revision_reads_at_once() {
        for after in [
            "",
            "x",
            "short.1",
            "0123456789abcdef",
            "0123456789abcdef.x",
            "0123456789abcdef.-1",
        ] {
            assert!(parse(after).is_none(), "{after:?}");
        }
        assert_eq!(parse("0123456789abcdef.12"), Some(("0123456789abcdef", 12)));
    }
}
