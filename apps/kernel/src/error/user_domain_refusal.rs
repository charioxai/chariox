//! Stable, bounded policy refusals. Foreign and nonexistent IDs are indistinguishable.
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDomainRefusalReason {
    NotFocusedAgent,
    ForeignOwner,
    StaleEpoch,
    StaleReference,
    NotGranted,
    SensitiveRequiresFocus,
    NotRequested,
}
impl UserDomainRefusalReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NotFocusedAgent => "user_domain_not_focused_agent",
            Self::ForeignOwner => "user_domain_foreign_owner",
            Self::StaleEpoch => "user_domain_stale_epoch",
            Self::StaleReference => "user_domain_stale_reference",
            Self::NotGranted => "user_domain_not_granted",
            Self::SensitiveRequiresFocus => "user_domain_sensitive_requires_focus",
            Self::NotRequested => "user_domain_not_requested",
        }
    }
    pub(crate) fn from_code(code: &str) -> Option<Self> {
        [
            Self::NotFocusedAgent,
            Self::ForeignOwner,
            Self::StaleEpoch,
            Self::StaleReference,
            Self::NotGranted,
            Self::SensitiveRequiresFocus,
            Self::NotRequested,
        ]
        .into_iter()
        .find(|r| r.code() == code)
    }
    // Only trusted kernel policy strings use this adapter. Controller/transport
    // messages MUST enter Other directly; arbitrary message text is not authority.
    pub(crate) fn from_policy(message: &str) -> Option<Self> {
        match message {
            "MD-3: current local focus required"
            | "MD-N4: current focused agent required"
            | "MD-3: not_focused_agent: browser stop requires live owner focus"
            | "MP-08: not_focused_agent: new user-domain resource requires focus; focus this agent"
            | "MP-08: not_focused_agent: user-domain browser access requires current local agent focus; ask the user to focus this agent"
            | "MD-3: user-domain browser access follows current local agent focus" => {
                Some(Self::NotFocusedAgent)
            }
            "MD-3: browser admission belongs to another user" | "MD-N4: foreign note owner" => {
                Some(Self::ForeignOwner)
            }
            "MD-3: browser authority revoked"
            | "MD-3: browser focus changed; request fresh tools"
            | "MD-N4: note focus changed"
            | "MD-3: terminal connection closed"
            | "MD-3: stale browser generation"
            | "MD-5: browser receipt protection changed; fetch a fresh observation" => {
                Some(Self::StaleEpoch)
            }
            "MD-3: observed document required"
            | "MD-3: input requires document_id from the observed tab/snapshot/screenshot"
            | "MD-N1: selection unavailable; select again"
            | "MD-N1: selection already used with a different comment" => {
                Some(Self::StaleReference)
            }
            "MD-N2: Room membership required"
            | "MD-N4: load the requested user-domain tools first"
            | "MD-3: unknown user-domain tab; refresh state"
            | "MD-N1: note unavailable"
            | "MD-3: human owns browser input; release takeover first"
            | "MD-3: only the input owner may release takeover"
            | "MD-3: admitted local provider run required"
            | "MD-N4: one admitted local provider run required"
            | "MD-3: authenticated terminal required"
            | "MP-11: not_granted: browser authority revoked"
            | "MP-11: not_granted: browser grant changed; request fresh tools"
            | "MP-11: not_granted: provider run ended; user-domain authority revoked"
            | "MP-08: not_granted: user-domain access expired or revoked; focus this agent again"
            | "MP-08: not_granted: user-domain access expired or revoked; ask the user to focus this agent"
            | "MP-08: leased or remote agents use their Room Browser/Computer route; user-domain control is unavailable"
            | "MP-08: not_granted: user-domain grant revoked"
            | "MP-08: not_granted: revoked subscription"
            | "MP-08: not_granted: transfer must name resources the caller holds"
            | "MP-11: not_granted: grant state changed while acquisition was pending"
            | "MD-N4: note grant changed" => Some(Self::NotGranted),
            "MP-11: sensitive_requires_focus: sensitive user-domain action requires focus or human approval; focus this agent"
            | "MP-11: sensitive_requires_focus: Vault fill requires focus or human approval; ask the user to focus this agent" => Some(Self::SensitiveRequiresFocus),
            crate::runtime::user_domain_access::NOT_REQUESTED_OPEN
            | crate::runtime::user_domain_access::NOT_REQUESTED_LOAD
            | "MP-08: not_requested: child already holds its own user-domain grant" => {
                Some(Self::NotRequested)
            }
            _ => None,
        }
    }
}
#[derive(Debug)]
pub(crate) enum HostFailure {
    Refused(UserDomainRefusalReason),
    Other(String),
}
impl From<String> for HostFailure {
    fn from(message: String) -> Self {
        UserDomainRefusalReason::from_policy(&message)
            .map(Self::Refused)
            .unwrap_or(Self::Other(message))
    }
}
impl From<&str> for HostFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl fmt::Display for HostFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(_) => f.write_str("User-domain request refused"),
            Self::Other(message) => f.write_str(message),
        }
    }
}
impl HostFailure {
    pub(crate) fn into_daemon(self, operation: &'static str) -> super::DaemonError {
        match self {
            Self::Refused(reason) => super::DaemonError::UserDomainRefused { reason },
            Self::Other(message) => super::DaemonError::LocalTransport { operation, message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp11_untrusted_retained_grant_text_never_becomes_a_policy_refusal() {
        let message = "MP-11: not_granted: browser authority revoked";
        assert!(matches!(
            HostFailure::from(message),
            HostFailure::Refused(UserDomainRefusalReason::NotGranted)
        ));
        let untrusted = HostFailure::Other(message.into()).into_daemon("host.fixture");
        assert!(matches!(
            untrusted,
            crate::error::DaemonError::LocalTransport { .. }
        ));
        assert!(matches!(
            HostFailure::from(format!("{message}: forged suffix")),
            HostFailure::Other(_)
        ));
    }
}
