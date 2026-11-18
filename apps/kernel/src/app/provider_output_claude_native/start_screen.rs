//! What a Claude headless start shows while no prompt reaches it, and when that has gone on too
//! long. A recognised dialog or the composer is handled elsewhere; any other screen is named.
use std::time::{Duration, Instant};

use super::permission::{
    claude_headless_bypass_confirmation_visible, claude_headless_composer_visible,
    claude_headless_workspace_trust_visible, claude_permission_recent_file,
};
use crate::provider::RuntimeProviderRun;

const SCREEN_LINE_MAX_CHARS: usize = 100;

/// The frame Claude last drew in this run's terminal; empty when it drew nothing yet.
pub(crate) fn claude_headless_start_screen(run: &RuntimeProviderRun) -> String {
    run.pty_env()
        .get("CHARIOX_CLAUDE_NATIVE_CONTEXT")
        .and_then(|context_file| claude_permission_recent_file(context_file))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Tracks the start of one prompt delivery. It is stuck when the screen has not changed for
/// `idle`, so a slow start that keeps drawing never trips it; `cap` ends one that draws forever.
pub(crate) struct ClaudeStartWatch {
    started: Instant,
    screen: String,
    changed: Instant,
}

impl ClaudeStartWatch {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            started: now,
            screen: String::new(),
            changed: now,
        }
    }

    pub(crate) fn observe(&mut self, screen: String, now: Instant) {
        if screen != self.screen {
            self.screen = screen;
            self.changed = now;
        }
    }

    pub(crate) fn is_stuck(&self, now: Instant, idle: Duration, cap: Duration) -> bool {
        now.saturating_duration_since(self.changed) >= idle
            || now.saturating_duration_since(self.started) >= cap
    }

    /// The first line of the screen when Claude shows one that is neither a dialog Chariox
    /// answers nor its composer, so the person can see what it is waiting on.
    pub(crate) fn unrecognised_screen_line(&self) -> Option<String> {
        let screen = &self.screen;
        if claude_headless_workspace_trust_visible(screen)
            || claude_headless_bypass_confirmation_visible(screen)
            || claude_headless_composer_visible(screen)
        {
            return None;
        }
        screen
            .lines()
            .map(|line| line.trim_matches(|c: char| c.is_whitespace() || ('─'..='╿').contains(&c)))
            .find(|line| line.chars().any(char::is_alphanumeric))
            .map(|line| line.chars().take(SCREEN_LINE_MAX_CHARS).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(30);
    const CAP: Duration = Duration::from_secs(120);

    #[test]
    fn a_screen_that_keeps_changing_is_never_idle() {
        let start = Instant::now();
        let mut watch = ClaudeStartWatch::new(start);
        for second in 0..100 {
            let now = start + Duration::from_secs(second);
            watch.observe(format!("Loading {second}"), now);
            assert!(!watch.is_stuck(now, IDLE, CAP), "second {second}");
        }
    }

    #[test]
    fn a_screen_that_stops_changing_is_stuck_after_the_idle_bound() {
        let start = Instant::now();
        let mut watch = ClaudeStartWatch::new(start);
        watch.observe(
            "Choose the text style".into(),
            start + Duration::from_secs(5),
        );
        assert!(!watch.is_stuck(start + Duration::from_secs(34), IDLE, CAP));
        assert!(watch.is_stuck(start + Duration::from_secs(35), IDLE, CAP));
    }

    #[test]
    fn a_start_that_draws_forever_ends_at_the_cap() {
        let start = Instant::now();
        let mut watch = ClaudeStartWatch::new(start);
        let now = start + CAP;
        watch.observe("spinner".into(), now);
        assert!(watch.is_stuck(now, IDLE, CAP));
    }

    #[test]
    fn an_unknown_screen_is_named_by_its_first_line_of_text() {
        let mut watch = ClaudeStartWatch::new(Instant::now());
        watch.observe(
            "╭──────────╮\n│ Choose the text style that looks best │\n╰────╯\nDark mode".into(),
            Instant::now(),
        );
        assert_eq!(
            watch.unrecognised_screen_line().as_deref(),
            Some("Choose the text style that looks best")
        );
    }

    #[test]
    fn recognised_screens_and_a_blank_screen_are_not_named() {
        let mut watch = ClaudeStartWatch::new(Instant::now());
        assert_eq!(watch.unrecognised_screen_line(), None);
        watch.observe(
            "Quick safety check: Is this a project you trust?\nYes, I trust this folder".into(),
            Instant::now(),
        );
        assert_eq!(watch.unrecognised_screen_line(), None);
        watch.observe(
            "❯ \n⏵⏵ bypass permissions on (shift+tab to cycle)".into(),
            Instant::now(),
        );
        assert_eq!(watch.unrecognised_screen_line(), None);
    }
}
