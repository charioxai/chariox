//! MP-10 / F4: Claude's first-run folder-trust question on a native terminal. It is one kernel
//! interaction, shown to every client of the Room (a client that joins later sees it too), whose
//! answer the kernel types into Claude. Answering in the terminal withdraws it.
use super::permission::{
    claude_headless_composer_visible, claude_headless_workspace_trust_visible,
    claude_rendered_terminal_text, claude_workspace_trust_approval_input,
};
use super::workspace_trust::displayed_workspace;
use crate::provider::RuntimeProviderRun;

/// Present while the card is open.
const OPEN_FILE: &str = "trust-card-open";
/// Present once the card was withdrawn; the question is not raised again for this run.
const CLOSED_FILE: &str = "trust-card-closed";
/// Rendered text since the dialog could start: its title and its choices can arrive in different
/// drains, and the selector row is read from it when the card is answered.
const RECENT_FILE: &str = "trust-recent.txt";
const RECENT_CHARS: usize = 4000;
/// Claude Esc closes the dialog like "No, exit".
const DECLINE_INPUT: &[u8] = b"\x1b";

/// What one drain of terminal output did to the trust question.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClaudeTrustDialog {
    /// The question appeared; raise its card for `workspace`.
    Opened { workspace: String },
    /// The person answered in the terminal; withdraw the card.
    AnsweredInTerminal,
}

/// The keystrokes that answer the question. A selector that must move first needs Enter after it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClaudeTrustKeys {
    pub(crate) first: Vec<u8>,
    pub(crate) then_enter: bool,
}

/// The id of the one trust card of `run`.
pub(crate) fn claude_native_trust_interaction_id(run: &RuntimeProviderRun) -> String {
    format!("claude-native-trust-{}", run.id())
}

pub(crate) fn claude_native_trust_dialog_update(
    run: &RuntimeProviderRun,
    rendered: &str,
) -> Option<ClaudeTrustDialog> {
    let context_file = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    trust_dialog_update(context_file, rendered)
}

/// The keys for the person's choice, read from the latest frame of the selector.
pub(crate) fn claude_native_trust_keys(
    run: &RuntimeProviderRun,
    trust: bool,
) -> Option<ClaudeTrustKeys> {
    let context_file = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    if !trust {
        return Some(ClaudeTrustKeys {
            first: DECLINE_INPUT.to_vec(),
            then_enter: false,
        });
    }
    let recent = std::fs::read_to_string(sibling(context_file, RECENT_FILE)).ok()?;
    let first = claude_workspace_trust_approval_input(&recent)?.to_vec();
    let then_enter = first != b"\r";
    Some(ClaudeTrustKeys { first, then_enter })
}

fn sibling(context_file: &str, name: &str) -> std::path::PathBuf {
    std::path::Path::new(context_file).with_file_name(name)
}

/// What Claude draws once the question is gone: its composer, or the welcome header above it
/// ("Claude Code v2.1.327"). Some releases draw a composer footer this kernel does not know.
fn dialog_answered_in_terminal(drained: &str) -> bool {
    let header = drained
        .split("Claude Code v")
        .skip(1)
        .any(|after| after.chars().next().is_some_and(|ch| ch.is_ascii_digit()));
    !claude_headless_workspace_trust_visible(drained)
        && (header || claude_headless_composer_visible(drained))
}

fn trust_dialog_update(context_file: &str, rendered: &str) -> Option<ClaudeTrustDialog> {
    if rendered.is_empty() || sibling(context_file, CLOSED_FILE).exists() {
        return None;
    }
    let drained = claude_rendered_terminal_text(rendered);
    let open_file = sibling(context_file, OPEN_FILE);
    let open = open_file.exists();
    if open && dialog_answered_in_terminal(&drained) {
        let _ = std::fs::remove_file(&open_file);
        std::fs::write(sibling(context_file, CLOSED_FILE), "").ok()?;
        return Some(ClaudeTrustDialog::AnsweredInTerminal);
    }
    let recent_file = sibling(context_file, RECENT_FILE);
    let mut recent = std::fs::read_to_string(&recent_file).unwrap_or_default();
    recent.push_str(&drained);
    let excess = recent.chars().count().saturating_sub(RECENT_CHARS);
    if excess > 0 {
        recent = recent.chars().skip(excess).collect();
    }
    std::fs::write(&recent_file, &recent).ok()?;
    if open
        || !claude_headless_workspace_trust_visible(&recent)
        || claude_workspace_trust_approval_input(&recent).is_none()
    {
        return None;
    }
    std::fs::write(open_file, "").ok()?;
    Some(ClaudeTrustDialog::Opened {
        workspace: displayed_workspace(&recent)?.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIALOG: &str = "\x1b[1mAccessing workspace:\x1b[0m\r\n /work/space\r\n Quick safety check: Is this a project you trust?\r\n Do you trust this folder?\r\n \u{276f} No, exit\r\n   Yes, I trust this folder\r\n";
    const COMPOSER: &str = "\u{276f} Try \"write a test for foo\"\r\n  \u{23f5}\u{23f5} manual mode on (shift+tab to cycle)\r\n";

    fn context(label: &str) -> (crate::test_support::TestWorktree, String) {
        let root = crate::test_support::TestWorktree::new(label);
        let context = root.path().join("hidden-context.txt").display().to_string();
        (root, context)
    }

    #[test]
    fn the_trust_question_opens_one_card_with_its_workspace() {
        let (_root, context) = context("claude-trust-card-opens");
        assert_eq!(
            trust_dialog_update(&context, DIALOG),
            Some(ClaudeTrustDialog::Opened {
                workspace: "/work/space".into()
            })
        );
        assert_eq!(trust_dialog_update(&context, DIALOG), None, "not twice");
    }

    #[test]
    fn a_question_split_across_drains_waits_for_its_choices() {
        let (_root, context) = context("claude-trust-card-split");
        let (head, tail) = DIALOG.split_at(DIALOG.find('\u{276f}').unwrap());
        assert_eq!(trust_dialog_update(&context, head), None);
        assert_eq!(
            trust_dialog_update(&context, tail),
            Some(ClaudeTrustDialog::Opened {
                workspace: "/work/space".into()
            })
        );
    }

    #[test]
    fn ordinary_output_raises_nothing() {
        let (_root, context) = context("claude-trust-card-none");
        assert_eq!(trust_dialog_update(&context, "Reply with PONG"), None);
        assert_eq!(trust_dialog_update(&context, COMPOSER), None);
    }

    #[test]
    fn answering_in_the_terminal_withdraws_the_card_once() {
        let (_root, context) = context("claude-trust-card-local-answer");
        assert!(trust_dialog_update(&context, DIALOG).is_some());
        assert_eq!(
            trust_dialog_update(&context, COMPOSER),
            Some(ClaudeTrustDialog::AnsweredInTerminal)
        );
        assert_eq!(trust_dialog_update(&context, COMPOSER), None);
        assert_eq!(
            trust_dialog_update(&context, DIALOG),
            None,
            "never raised again"
        );
    }

    #[test]
    fn the_welcome_header_also_ends_the_question() {
        let (_root, context) = context("claude-trust-card-header");
        assert!(trust_dialog_update(&context, DIALOG).is_some());
        assert_eq!(
            trust_dialog_update(
                &context,
                "\u{259d}\u{259c} Claude Code v2.1.327\r\n  Haiku 5.5 with low effort\r\n"
            ),
            Some(ClaudeTrustDialog::AnsweredInTerminal)
        );
    }

    #[test]
    fn trusting_moves_the_selector_to_yes_and_confirms() {
        let (_root, context) = context("claude-trust-card-keys");
        assert!(trust_dialog_update(&context, DIALOG).is_some());
        let recent = std::fs::read_to_string(sibling(&context, RECENT_FILE)).unwrap();
        assert_eq!(
            claude_workspace_trust_approval_input(&recent),
            Some(&b"\x1b[B"[..]),
            "No is highlighted, so the selector moves down first"
        );
    }
}
