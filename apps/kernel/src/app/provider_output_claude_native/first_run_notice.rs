//! MP-10: Claude's first-run dialog on a native terminal belongs to the person
//! at that terminal. The kernel cannot answer it, but every other client must
//! be told why the agent has not started, once.
use super::permission::{claude_headless_workspace_trust_visible, claude_rendered_terminal_text};
use crate::provider::RuntimeProviderRun;

const FIRST_RUN_NOTICE_FILE: &str = "first-run-notice";
/// Rendered text since the last drain that could still be part of the dialog: its title and its
/// question can arrive in different drains.
const FIRST_RUN_RECENT_FILE: &str = "first-run-recent.txt";
const FIRST_RUN_RECENT_CHARS: usize = 4000;

pub(crate) const CLAUDE_TRUST_FOLDER_NOTICE: &str =
    "Claude is asking whether to trust this folder in its terminal. Answer it there to start the agent.";

/// The notice for the dialog drawn in `rendered`, the first time it appears.
pub(crate) fn claude_native_first_run_notice(
    run: &RuntimeProviderRun,
    rendered: &str,
) -> Option<&'static str> {
    let context_file = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    first_run_notice_once(context_file, rendered)
}

fn first_run_notice_once(context_file: &str, rendered: &str) -> Option<&'static str> {
    let context = std::path::Path::new(context_file);
    let sent = context.with_file_name(FIRST_RUN_NOTICE_FILE);
    if sent.exists() {
        return None;
    }
    let recent_file = context.with_file_name(FIRST_RUN_RECENT_FILE);
    let mut recent = std::fs::read_to_string(&recent_file).unwrap_or_default();
    recent.push_str(&claude_rendered_terminal_text(rendered));
    let excess = recent
        .chars()
        .count()
        .saturating_sub(FIRST_RUN_RECENT_CHARS);
    if excess > 0 {
        recent = recent.chars().skip(excess).collect();
    }
    if !claude_headless_workspace_trust_visible(&recent) {
        std::fs::write(&recent_file, recent).ok()?;
        return None;
    }
    let _ = std::fs::remove_file(&recent_file);
    std::fs::write(sent, "").ok()?;
    Some(CLAUDE_TRUST_FOLDER_NOTICE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trust_dialog_is_announced_once() {
        let root = crate::test_support::TestWorktree::new("claude-first-run-notice");
        let context = root.path().join("hidden-context.txt");
        let context_file = context.to_str().unwrap();
        let dialog = "\x1b[1mQuick safety check:\x1b[0m Is this a project you trust?\r\n Do you trust this folder?\r\n";
        assert_eq!(
            first_run_notice_once(context_file, dialog),
            Some(CLAUDE_TRUST_FOLDER_NOTICE)
        );
        assert_eq!(first_run_notice_once(context_file, dialog), None);
    }

    #[test]
    fn a_dialog_split_across_two_drains_is_announced() {
        let root = crate::test_support::TestWorktree::new("claude-first-run-split");
        let context = root.path().join("hidden-context.txt");
        let context_file = context.to_str().unwrap();
        assert_eq!(
            first_run_notice_once(
                context_file,
                "\x1b[1mQuick safety check:\x1b[0m Is this a project you trust?\r\n"
            ),
            None
        );
        assert_eq!(
            first_run_notice_once(context_file, " Do you trust this folder?\r\n"),
            Some(CLAUDE_TRUST_FOLDER_NOTICE)
        );
    }

    #[test]
    fn ordinary_output_is_not_announced() {
        let root = crate::test_support::TestWorktree::new("claude-first-run-none");
        let context = root.path().join("hidden-context.txt");
        assert_eq!(
            first_run_notice_once(context.to_str().unwrap(), "Reply with PONG"),
            None
        );
    }
}
