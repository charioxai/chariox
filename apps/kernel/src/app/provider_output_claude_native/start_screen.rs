//! What holds a Claude headless start back, and what to tell the person when it runs out of time.
use super::permission::{
    claude_headless_bypass_selection_pending, claude_headless_composer_visible,
    claude_headless_workspace_trust_interaction_marker, claude_native_marker,
    claude_permission_recent_file,
};
use crate::provider::RuntimeProviderRun;

const SCREEN_LINE_MAX_CHARS: usize = 100;

/// True once the prompt was written to Claude's terminal: the start phase is over and the
/// acknowledgement clock may run.
pub(crate) fn claude_headless_prompt_written(run: &RuntimeProviderRun, prompt_id: &str) -> bool {
    run.pty_env()
        .get("CHARIOX_CLAUDE_NATIVE_CONTEXT")
        .and_then(|context_file| claude_native_marker(context_file))
        .is_some_and(|marker| prompt_written_marker(&marker, prompt_id))
}

fn prompt_written_marker(marker: &str, prompt_id: &str) -> bool {
    ["typed:", "submit-wait:", "injected:"]
        .iter()
        .filter_map(|prefix| marker.strip_prefix(prefix))
        .any(|rest| rest.split(':').next() == Some(prompt_id))
}

/// The marker of the card the start waits on, while it names one: the workspace-trust card or a
/// permission card. The caller checks the card is still open; a card lost with a hot reload or a
/// restart holds nothing back.
pub(crate) fn claude_headless_owner_decision_marker(run: &RuntimeProviderRun) -> Option<String> {
    let context_file = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    claude_native_marker(context_file).filter(|marker| {
        claude_headless_workspace_trust_interaction_marker(marker)
            || marker.starts_with("permission:")
    })
}

/// The first line of the screen Claude shows when it is not its composer, so the person can see
/// what the start is stuck on.
pub(crate) fn claude_headless_stuck_screen_line(run: &RuntimeProviderRun) -> Option<String> {
    let context_file = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    let screen = claude_permission_recent_file(context_file)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    stuck_screen_line(&screen).or_else(|| {
        // The kernel clears the frame each time it answers the bypass confirmation, so the
        // screen is known by what the kernel is waiting on.
        claude_headless_bypass_selection_pending(context_file)
            .then(|| "Bypass Permissions mode confirmation".to_string())
    })
}

fn stuck_screen_line(screen: &str) -> Option<String> {
    if claude_headless_composer_visible(screen) {
        return None;
    }
    screen
        .lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || ('─'..='╿').contains(&c)))
        .find(|line| line.chars().any(char::is_alphanumeric))
        .map(|line| line.chars().take(SCREEN_LINE_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_counts_as_written_from_its_own_typing_onward() {
        for marker in ["typed:p1", "submit-wait:p1:1000", "injected:p1"] {
            assert!(prompt_written_marker(marker, "p1"), "{marker}");
            assert!(!prompt_written_marker(marker, "p2"), "{marker}");
        }
        for marker in [
            "startup-workspace-trust:p1",
            "startup-wait:1000",
            "permission:p1",
            "accepted:p1",
        ] {
            assert!(!prompt_written_marker(marker, "p1"), "{marker}");
        }
    }

    #[test]
    fn a_stuck_screen_is_named_by_its_first_line_of_text() {
        assert_eq!(
            stuck_screen_line(
                "╭──────────╮\n│ Choose the text style that looks best │\n╰────╯\nDark mode"
            )
            .as_deref(),
            Some("Choose the text style that looks best")
        );
        assert_eq!(
            stuck_screen_line("WARNING: Bypass Permissions mode\n1. No, exit\n2. Yes, I accept")
                .as_deref(),
            Some("WARNING: Bypass Permissions mode")
        );
    }

    #[test]
    fn the_composer_and_a_blank_screen_are_not_named() {
        assert_eq!(stuck_screen_line(""), None);
        assert_eq!(
            stuck_screen_line("❯ \n⏵⏵ bypass permissions on (shift+tab to cycle)"),
            None
        );
    }
}
