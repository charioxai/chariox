//! MP-08/MP-10: protocol 556 parked Browser state read. A viewer holds one `StateWait` instead of
//! polling `State`; the host says when a read could differ and the reply's content decides.
use super::kernel_browser_favicons::name_favicons;
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::runtime::command::KernelCommand;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// An idle viewer parks this long, so a shown Browser costs one request per wait, not one every two seconds.
const MAX_WAIT_MS: u32 = 10_000;
/// A woken read repeats no sooner than this, so a busy page cannot turn the wait into a poll.
const MIN_GAP: Duration = Duration::from_millis(100);
const NO_SIGNAL_POLL: Duration = Duration::from_millis(250);
/// How often a parked read looks at who holds a Tab by takeover: an in-kernel read, no host request.
const CONTROL_POLL: Duration = Duration::from_millis(100);

/// `<content digest>.<host signal>.<control digest>`: opaque to clients, which echo it back as `after`.
/// The control digest comes from the same snapshot as the state's `taken_over` marks (the host adds it under
/// the actor lock), so a takeover or its return between two reads differs from it at the next wait, before it
/// parks and while it is parked.
fn revision(state: &Value, signal: Option<u64>, control: &str) -> String {
    let digest = Sha256::digest(state.to_string().as_bytes());
    format!("{}.{}.{control}", hex_prefix(&digest), signal.unwrap_or(0))
}
/// A wait that ended without a change: the caller keeps the state it read last.
fn unchanged(revision: String) -> Value {
    serde_json::json!({"revision": revision, "unchanged": true})
}
fn hex_prefix(digest: &[u8]) -> String {
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
struct Known<'a> {
    digest: &'a str,
    signal: u64,
    control: &'a str,
}
fn parse(after: &str) -> Option<Known<'_>> {
    let (digest, rest) = after.split_once('.')?;
    let (signal, control) = rest.split_once('.')?;
    (digest.len() == 16 && control.len() == 16).then_some(())?;
    Some(Known {
        digest,
        signal: signal.parse().ok()?,
        control,
    })
}

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_state_wait(
        &self,
        caller: &KernelCommand,
        after: &str,
        wait_ms: u32,
        known_favicons: &[String],
    ) -> Result<Value, DaemonError> {
        if wait_ms > MAX_WAIT_MS {
            return Err(host_error(format!(
                "MP-08: state wait exceeds {MAX_WAIT_MS} ms"
            )));
        }
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let known = parse(after);
        let deadline = Instant::now() + Duration::from_millis(wait_ms.into());
        let mut signal = known.as_ref().map(|known| known.signal);
        let host = self.owned.kernel_browser_host.clone();
        loop {
            let started = Instant::now();
            let remaining = deadline.saturating_duration_since(started);
            match (signal, &known) {
                // The viewer's state is already behind: read it now instead of parking on a quiet page.
                (Some(_), Some(known))
                    if host.person_control(&user).map_err(host_error)? != known.control => {}
                (Some(after_signal), _) => {
                    let (parked_host, parked_user, wait) =
                        (host.clone(), user.clone(), remaining.as_millis() as u64);
                    // The wait holds no protection barrier: Vault insertion never waits on a parked viewer.
                    let mut parked = tokio::task::spawn_blocking(move || {
                        parked_host.state_signal(&parked_user, Some(after_signal), wait)
                    });
                    // A takeover, its return or an idle reclaim belongs to the person's other windows too: it is no
                    // host signal, so the wait also ends when the held Tabs differ from the viewer's state.
                    loop {
                        tokio::select! {
                            signalled = &mut parked => {
                                signalled
                                    .map_err(|_| host_error("MP-08: state wait interrupted".into()))?
                                    .map_err(host_error)?;
                                break;
                            }
                            _ = tokio::time::sleep(CONTROL_POLL) => {
                                if known.as_ref().is_some_and(|known| {
                                    host.person_control(&user).is_ok_and(|control| control != known.control)
                                }) {
                                    break;
                                }
                            }
                        }
                    }
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
            let control = state
                .as_object_mut()
                .and_then(|object| object.remove("_control"))
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| host_error("MP-08: state read carries no control digest".into()))?;
            if let Some(object) = state.as_object_mut() {
                object.extend(
                    self.user_domain_window_projection(&user)
                        .as_object()
                        .unwrap()
                        .clone(),
                );
            }
            let current = revision(&state, signal, &control);
            let changed = known
                .as_ref()
                .is_none_or(|known| !current.starts_with(known.digest) || known.control != control);
            if !changed && Instant::now() >= deadline {
                // The viewer holds this content: only the revision to wait after travels.
                return Ok(unchanged(current));
            }
            if changed {
                name_favicons(&mut state, Some(known_favicons));
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

    const NONE_HELD: &str = "e3b0c44298fc1c14";

    #[test]
    fn revision_names_the_content_the_host_signal_and_the_control() {
        let state = json!({"generation":1,"tabs":[{"tab_id":"a","title":"x"}]});
        let first = revision(&state, Some(7), NONE_HELD);
        let known = parse(&first).unwrap();
        assert_eq!(known.signal, 7);
        assert_eq!(known.control, NONE_HELD);
        assert!(
            revision(&state, Some(9), NONE_HELD).starts_with(known.digest),
            "the signal alone is not content"
        );
        let other = json!({"generation":1,"tabs":[{"tab_id":"a","title":"y"}]});
        assert!(
            !revision(&other, Some(7), NONE_HELD).starts_with(known.digest),
            "a retitled Tab is a new revision"
        );
        assert_eq!(parse(&revision(&state, None, NONE_HELD)).unwrap().signal, 0);
    }

    #[test]
    fn a_caller_with_no_valid_revision_reads_at_once() {
        for after in [
            "",
            "x",
            "short.1.e3b0c44298fc1c14",
            "0123456789abcdef",
            "0123456789abcdef.12",
            "0123456789abcdef.x.e3b0c44298fc1c14",
            "0123456789abcdef.-1.e3b0c44298fc1c14",
            "0123456789abcdef.1.short",
        ] {
            assert!(parse(after).is_none(), "{after:?}");
        }
        let known = parse("0123456789abcdef.12.e3b0c44298fc1c14").unwrap();
        assert_eq!((known.digest, known.signal), ("0123456789abcdef", 12));
    }
}
