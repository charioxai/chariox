//! MP-08 / MP-10 / MP-11 A07 (protocol 477): a protected owner hand-off.
//!
//! The projection carries only safe target metadata the kernel validated: the
//! bound browser tab, generation, document and node, the document origin and
//! path (never query or fragment), a bounded label, the agent's stated reason
//! and the intended change. Owner input never appears here; it travels in
//! `RespondToHandoff` straight to the bound browser operation.
use serde::{Deserialize, Serialize};

/// Longest agent-supplied explanation shown to the owner.
pub const HANDOFF_EXPLANATION_MAX_CHARS: usize = 600;
/// Longest single intended-change line.
pub const HANDOFF_CHANGE_LINE_MAX_CHARS: usize = 200;
/// Most intended-change lines.
pub const HANDOFF_CHANGE_MAX_LINES: usize = 48;
/// Longest safe target label.
pub const HANDOFF_LABEL_MAX_CHARS: usize = 120;
/// Shortest and longest owner window, seconds.
pub const HANDOFF_MIN_TIMEOUT_SEC: u64 = 30;
pub const HANDOFF_MAX_TIMEOUT_SEC: u64 = 3_600;

/// The one action the owner is asked to perform.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffKind {
    /// Click an observed element, in the scoped view or the live browser.
    Click,
    /// Type a verification code into an observed field.
    Code,
    /// Type a missing secret into an observed password field.
    Secret,
}

impl HandoffKind {
    pub fn accepts_value(self) -> bool {
        matches!(self, Self::Code | Self::Secret)
    }
}

/// Why the agent cannot act itself. A refusal is a reason to hand off, never
/// permission for the model to answer its own interaction.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffReason {
    ModelRefusal,
    AutomationDisallowed,
    HumanVerification,
    OwnerAuthorization,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffChangeOp {
    Keep,
    Add,
    Remove,
}

/// One line of the intended before/after change the owner verifies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffChangeLine {
    pub op: HandoffChangeOp,
    pub text: String,
}

/// The kernel-validated browser target. `origin` and `path` come from the
/// observed frame document; the exact URL is re-read before any action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTarget {
    pub tab_id: String,
    pub generation: u64,
    pub document_id: String,
    pub node_ref: String,
    pub origin: String,
    pub path: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandoff {
    pub kind: HandoffKind,
    pub reason: HandoffReason,
    pub agent_id: String,
    pub task_id: String,
    pub obligation_id: String,
    pub explanation: String,
    pub target: HandoffTarget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change: Vec<HandoffChangeLine>,
    /// Absolute expiry; restart restores the remaining window only.
    pub expires_at_ms: u64,
    /// A secret hand-off may offer saving the typed value to the Vault.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub save_to_vault_offered: bool,
}

/// Bound text for display: no control characters except newlines in the
/// explanation, trimmed to `max` characters.
pub(crate) fn bounded_display_text(value: &str, max: usize, allow_newline: bool) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || (allow_newline && *ch == '\n'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

impl RuntimeHandoff {
    /// The interaction ID for a hand-off: one per obligation.
    pub fn interaction_id(obligation_id: &str) -> String {
        format!("handoff-{obligation_id}")
    }

    pub(crate) fn valid(&self) -> bool {
        let target = &self.target;
        let id_ok =
            |id: &str| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control);
        id_ok(&self.agent_id)
            && id_ok(&self.task_id)
            && id_ok(&self.obligation_id)
            && id_ok(&target.tab_id)
            && id_ok(&target.document_id)
            && id_ok(&target.node_ref)
            && target.generation > 0
            && !target.origin.is_empty()
            && target.origin.len() <= 512
            && target.path.len() <= 1_024
            && target.label.chars().count() <= HANDOFF_LABEL_MAX_CHARS
            && self.explanation.chars().count() <= HANDOFF_EXPLANATION_MAX_CHARS
            && self.change.len() <= HANDOFF_CHANGE_MAX_LINES
            && self
                .change
                .iter()
                .all(|line| line.text.chars().count() <= HANDOFF_CHANGE_LINE_MAX_CHARS)
            && (!self.save_to_vault_offered || self.kind == HandoffKind::Secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handoff() -> RuntimeHandoff {
        RuntimeHandoff {
            kind: HandoffKind::Click,
            reason: HandoffReason::ModelRefusal,
            agent_id: "agent".into(),
            task_id: "task".into(),
            obligation_id: "obligation-1".into(),
            explanation: "Save the firewall rules".into(),
            target: HandoffTarget {
                tab_id: "tab".into(),
                generation: 1,
                document_id: "doc".into(),
                node_ref: "backend:7".into(),
                origin: "https://firewall.example".into(),
                path: "/rules".into(),
                label: "Save".into(),
            },
            change: vec![HandoffChangeLine {
                op: HandoffChangeOp::Add,
                text: "allow tcp 443 from any".into(),
            }],
            expires_at_ms: 1,
            save_to_vault_offered: false,
        }
    }

    #[test]
    fn mp08_mp11_a07_handoff_projection_is_bounded_safe_metadata() {
        assert!(handoff().valid());
        let mut vault_click = handoff();
        vault_click.save_to_vault_offered = true;
        assert!(
            !vault_click.valid(),
            "only a secret hand-off offers Vault save"
        );
        let mut long = handoff();
        long.explanation = "x".repeat(HANDOFF_EXPLANATION_MAX_CHARS + 1);
        assert!(!long.valid());
        assert_eq!(
            bounded_display_text(" a\u{7}b\nc ", 10, false),
            "abc",
            "control characters never reach the owner popup"
        );
        assert_eq!(bounded_display_text("a\nb", 10, true), "a\nb");
        assert_eq!(
            RuntimeHandoff::interaction_id("obligation-1"),
            "handoff-obligation-1"
        );
    }
}
