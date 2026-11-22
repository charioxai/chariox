//! Stable, bounded policy refusals. Foreign and nonexistent IDs are indistinguishable.
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDomainRefusalReason {
    /// The caller is not this owner (or the thing does not exist: the two look alike).
    ForeignOwner,
    /// The document generation the caller holds is out of date.
    StaleEpoch,
    /// A page reference the caller holds is out of date.
    StaleReference,
    /// The operation is not allowed for this caller: a person-only step, or a path outside the Room.
    NotGranted,
    /// The person's own input owner refuses agent input to a sensitive native target.
    SensitiveRequiresFocus,
}
impl UserDomainRefusalReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ForeignOwner => "user_domain_foreign_owner",
            Self::StaleEpoch => "user_domain_stale_epoch",
            Self::StaleReference => "user_domain_stale_reference",
            Self::NotGranted => "user_domain_not_granted",
            Self::SensitiveRequiresFocus => "user_domain_sensitive_requires_focus",
        }
    }
    pub(crate) fn from_code(code: &str) -> Option<Self> {
        [
            Self::ForeignOwner,
            Self::StaleEpoch,
            Self::StaleReference,
            Self::NotGranted,
            Self::SensitiveRequiresFocus,
        ]
        .into_iter()
            .find(|r| r.code() == code)
    }
    // Only trusted kernel policy strings use this adapter. Controller/transport
    // messages MUST enter Other directly; arbitrary message text is not authority.
    pub(crate) fn from_policy(message: &str) -> Option<Self> {
        match message {
            "MD-3: browser admission belongs to another user"
            | "MD-N4: foreign note owner"
            => Some(Self::ForeignOwner),
            "MD-3: stale browser generation"
            | "MD-5: browser receipt protection changed; fetch a fresh observation" => {
                Some(Self::StaleEpoch)
            }
            "MD-N2: Room membership required"
            | "MD-N1: note unavailable"
            | "MD-3: only the input owner may release takeover"
            | "MD-3: admitted local provider run required"
            | "MD-N4: one admitted local provider run required"
            | "MD-3: authenticated terminal required"
            | "MD-3: unknown user-domain tab; refresh state"
            | "MP-08: the agent is not in this live Room"
            | "MP-08: leased or remote agents use their Room Browser/Computer route; user-domain control is unavailable"
            | "MD-N4: note grant changed" => Some(Self::NotGranted),
            "MD-3: observed document required"
            | "MD-N1: selection unavailable; select again"
            | "MD-N1: selection already used with a different comment" => {
                Some(Self::StaleReference)
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
    fn untrusted_message_text_never_becomes_a_policy_refusal() {
        let message = "MD-3: browser admission belongs to another user";
        assert!(matches!(
            HostFailure::from(message),
            HostFailure::Refused(UserDomainRefusalReason::ForeignOwner)
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
