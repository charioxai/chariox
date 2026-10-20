//! MP-08 / MP-10: revisit timed native phases even when the provider is quiet.
use super::permission::{
    claude_native_marker, read_claude_headless_submit_retry, ClaudeHeadlessSubmitRetry,
};
use super::{CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT, CLAUDE_SUBMIT_DELAY_MS};
use crate::provider::RuntimeProviderRun;

pub(super) const CLAUDE_STARTUP_SETTLE_MS: u64 = 2_500;
pub(super) const CLAUDE_INITIAL_COMPOSER_WAIT_MS: u64 = 4_000;
pub(super) const CLAUDE_HEADLESS_RETRY_MS: u64 = 2_000;

pub(super) fn next_prompt_revisit_delay_ms(
    context_file: &str,
    prompt_id: &str,
    run: &RuntimeProviderRun,
    now_ms: u64,
) -> Option<u64> {
    let marker = claude_native_marker(context_file);
    let headless = run.provider() == "claude-headless";
    let retry = (headless && marker.as_deref() == Some(format!("injected:{prompt_id}").as_str()))
        .then(|| read_claude_headless_submit_retry(context_file));
    let due = next_phase_deadline(
        marker.as_deref(),
        prompt_id,
        headless,
        run.started_at_ms(),
        retry.as_ref(),
    )?;
    // An expired selection/submit phase must advance once, even if this thread
    // was descheduled after writing it. A stale retry or startup observation
    // must not turn the event-driven pump into an immediate polling loop.
    if marker
        .as_deref()
        .is_some_and(|m| m.starts_with("submit-wait:") || m.starts_with("startup-wait:"))
    {
        Some(due.saturating_sub(now_ms))
    } else {
        due.checked_sub(now_ms).filter(|delay| *delay > 0)
    }
}

fn next_phase_deadline(
    marker: Option<&str>,
    prompt_id: &str,
    headless: bool,
    started_at_ms: u64,
    retry: Option<&ClaudeHeadlessSubmitRetry>,
) -> Option<u64> {
    if let Some((id, written)) = marker
        .and_then(|m| m.strip_prefix("submit-wait:"))
        .and_then(|m| m.rsplit_once(':'))
    {
        return (id == prompt_id)
            .then(|| written.parse::<u64>().ok())
            .flatten()
            .map(|at| at.saturating_add(CLAUDE_SUBMIT_DELAY_MS));
    }
    if let Some(started) = marker.and_then(|m| m.strip_prefix("startup-wait:")) {
        return started
            .parse::<u64>()
            .ok()
            .map(|at| at.saturating_add(CLAUDE_STARTUP_SETTLE_MS));
    }
    if headless && marker.is_none_or(str::is_empty) {
        return Some(started_at_ms.saturating_add(CLAUDE_INITIAL_COMPOSER_WAIT_MS));
    }
    if headless && marker == Some(format!("injected:{prompt_id}").as_str()) {
        return retry
            .filter(|r| {
                r.prompt_id == prompt_id
                    && r.count < CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT
                    && r.last_attempt_ms > 0
            })
            .map(|r| r.last_attempt_ms.saturating_add(CLAUDE_HEADLESS_RETRY_MS));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadlines_follow_existing_phase_times_and_prompt_identity() {
        assert_eq!(
            next_phase_deadline(Some("submit-wait:p:1000"), "p", true, 0, None),
            Some(1250)
        );
        assert_eq!(
            next_phase_deadline(Some("submit-wait:other:1000"), "p", true, 0, None),
            None
        );
        assert_eq!(
            next_phase_deadline(Some("startup-wait:1000"), "p", true, 0, None),
            Some(3500)
        );
        assert_eq!(next_phase_deadline(None, "p", true, 1000, None), Some(5000));
        assert_eq!(next_phase_deadline(None, "p", false, 1000, None), None);
        for marker in ["accepted:p", "permission:p", "startup-workspace-trust:p"] {
            assert_eq!(next_phase_deadline(Some(marker), "p", true, 0, None), None);
        }
    }
    #[test]
    fn retries_remain_bounded_and_never_follow_another_prompt() {
        let mut retry = ClaudeHeadlessSubmitRetry {
            prompt_id: "p".into(),
            count: 0,
            last_attempt_ms: 1000,
            visible_prompt: String::new(),
        };
        assert_eq!(
            next_phase_deadline(Some("injected:p"), "p", true, 0, Some(&retry)),
            Some(3000)
        );
        retry.count = CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT;
        assert_eq!(
            next_phase_deadline(Some("injected:p"), "p", true, 0, Some(&retry)),
            None
        );
        retry.count = 0;
        retry.prompt_id = "other".into();
        assert_eq!(
            next_phase_deadline(Some("injected:p"), "p", true, 0, Some(&retry)),
            None
        );
    }
}
