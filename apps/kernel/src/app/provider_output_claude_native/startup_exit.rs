//! Content-free native startup exit evidence. PTYs merge stdout and stderr;
//! neither arbitrary terminal rows nor queued prompt/context text may be logged.

use super::permission::{claude_native_marker, claude_permission_recent_file};
use crate::provider::RuntimeProviderRun;

pub(in crate::app) struct ClaudeStartupExit {
    pub(in crate::app) fields: serde_json::Value,
    pub(in crate::app) detail: Option<&'static str>,
}

pub(in crate::app) fn claude_native_startup_exit(
    run: &RuntimeProviderRun,
    terminal_tail: Option<&str>,
) -> ClaudeStartupExit {
    let context = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT");
    let marker = context.and_then(|path| claude_native_marker(path));
    let recent = context
        .and_then(|path| claude_permission_recent_file(path))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    startup_exit_evidence(marker.as_deref(), &recent, terminal_tail)
}

fn startup_exit_evidence(
    marker: Option<&str>,
    recent: &str,
    terminal_tail: Option<&str>,
) -> ClaudeStartupExit {
    let phase = match marker
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
    {
        "startup-workspace-trust" => "workspace_trust",
        "startup-workspace-trust-denied" => "workspace_trust_denied",
        "startup-workspace-trust-selection-failed" => "workspace_trust_selection_failed",
        "startup-wait" => "after_startup_decision",
        "startup-bypass-selection" => "bypass_confirmation",
        "submit-wait" => "awaiting_enter",
        "typed" => "typed",
        "injected" => "awaiting_hook_acknowledgement",
        "accepted" => "acknowledged",
        "" => "none",
        _ => "other",
    };
    // Fixed phrases only: even a secret or a prompt that looks like an error
    // must never become an arbitrary stderr snippet in the kernel log.
    let text = format!("{recent} {}", terminal_tail.unwrap_or_default()).to_ascii_lowercase();
    let phrases = [
        "quick safety check",
        "trust this folder",
        "no, exit",
        "yes, i trust this folder",
        "bypass permissions mode",
        "cannot be used with root/sudo privileges",
        "permission denied",
        "authentication failed",
        "onboarding",
        "error:",
    ];
    let tail = phrases
        .into_iter()
        .filter(|phrase| text.contains(phrase))
        .collect::<Vec<_>>();
    let detail = match phase {
        "workspace_trust" => Some("Claude Code was waiting for workspace trust approval."),
        "workspace_trust_denied" => Some("Claude Code workspace trust was denied."),
        "workspace_trust_selection_failed" => {
            Some("Claude Code could not apply the workspace trust selection.")
        }
        _ => None,
    };
    ClaudeStartupExit {
        fields: serde_json::json!({
            "native_phase": phase,
            "terminal_tail_source": "PTY (stdout and stderr merged)",
            "redacted_terminal_tail": tail,
            "other_terminal_content": "redacted",
        }),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp08_mp11_native_exit_evidence_never_includes_terminal_or_prompt_content() {
        let evidence = startup_exit_evidence(
            Some("startup-workspace-trust:private-interaction-id"),
            "Quick safety check: do you trust this folder? private prompt and hidden context",
            Some("Error: sk-ant-private-credential arbitrary provider exception payload"),
        );
        assert_eq!(evidence.fields["native_phase"], "workspace_trust");
        assert!(evidence.detail.unwrap().contains("workspace trust"));
        let fields = evidence.fields.to_string();
        for private in [
            "private-interaction",
            "private prompt",
            "hidden context",
            "sk-ant",
            "exception payload",
        ] {
            assert!(!fields.contains(private), "{fields}");
        }
        assert!(fields.contains("quick safety check"));
        assert!(
            startup_exit_evidence(Some("accepted:prompt-private"), "", None)
                .detail
                .is_none()
        );
    }
}
