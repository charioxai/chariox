//! Definitive gateway outcomes: a code, one plain sentence and the next step.
use crate::transport::kernel_protocol::KernelTransportError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GatewayError {
    Unknown {
        name: String,
    },
    Ambiguous {
        name: String,
        candidates: Vec<String>,
    },
    Offline {
        name: String,
    },
    Unreachable {
        name: String,
        detail: String,
    },
    Unpaired {
        name: String,
    },
    /// The relay's kernel list could not be fetched.
    ListUnavailable {
        detail: String,
    },
    /// The owner revoked this kernel's gateway in the Cloud clients list.
    GatewayRevoked,
    /// The grant the request acts under ended while the connection was being set up.
    AccessEnded {
        name: String,
    },
    Incompatible {
        name: String,
        detail: String,
    },
    RequestTimeout {
        name: String,
    },
    ConnectionLost {
        name: String,
    },
    /// A pairing link could not be used; `detail` says why and what to do.
    JoinRefused {
        detail: String,
    },
    /// The events of a subscription stopped because the connection ended.
    SubscriptionEnded {
        name: String,
        last_event_id: Option<u64>,
    },
    /// The target kernel answered with its own typed refusal; it is passed through.
    Remote {
        name: String,
        code: String,
        message: String,
        retryable: bool,
    },
}

/// A transport failure in plain words for the agent. The raw text can carry the relay URL with
/// account and realm ids and OS error codes: it goes to the kernel's log, not to the caller.
pub(crate) fn plain_cause(what: &str, error: &dyn std::fmt::Display) -> String {
    tracing::warn!(%error, "{what}");
    what.to_string()
}

impl GatewayError {
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Unknown { .. } => "kernel_unknown",
            Self::Ambiguous { .. } => "kernel_ambiguous",
            Self::Offline { .. } => "kernel_offline",
            Self::Unreachable { .. } | Self::ListUnavailable { .. } => "kernel_unreachable",
            Self::Unpaired { .. } | Self::GatewayRevoked => "kernel_unpaired",
            Self::AccessEnded { .. } => "kernel_access_ended",
            Self::Incompatible { .. } => "kernel_incompatible",
            Self::RequestTimeout { .. } => "kernel_request_timeout",
            Self::ConnectionLost { .. } | Self::SubscriptionEnded { .. } => {
                "kernel_connection_lost"
            }
            Self::JoinRefused { .. } => "kernel_join_refused",
            Self::Remote { code, .. } => code,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Unknown { name } => format!(
                "No kernel is named \"{name}\". Run `chariox kernels` to list the kernels you can use."
            ),
            Self::Ambiguous { name, candidates } => format!(
                "\"{name}\" matches several kernels: {}. Name one of them as `<machine>/<label>` (see `chariox kernels`) or by its kernel id.",
                candidates.join(", ")
            ),
            Self::Offline { name } => format!(
                "Kernel \"{name}\" is offline. Start the Chariox kernel there; ssh is only a break-glass."
            ),
            Self::Unreachable { name, detail } => format!(
                "Could not reach kernel \"{name}\" ({detail}). Retry; check that the relay is reachable."
            ),
            Self::ListUnavailable { detail } => format!(
                "Could not get the list of your kernels ({detail}). Retry; check that this machine can reach the Chariox relay."
            ),
            Self::Unpaired { name } => format!(
                "Kernel \"{name}\" does not accept this kernel. Pair it again from a Chariox terminal."
            ),
            Self::AccessEnded { name } => format!(
                "Your access ended before the request to kernel \"{name}\" was sent, so nothing was sent. Ask the user to approve a new access request."
            ),
            Self::GatewayRevoked => "This kernel's agent gateway was revoked in the Cloud clients list and the agent access grants ended. Ask the user to approve a new agent access request.".to_string(),
            Self::Incompatible { name, detail } => format!(
                "Kernel \"{name}\" cannot run this request ({detail}). Update the Chariox kernel on that machine."
            ),
            Self::RequestTimeout { name } => format!(
                "Kernel \"{name}\" did not answer in time. For a state-changing request retry once with the same --command-id; the kernel never runs it twice."
            ),
            Self::ConnectionLost { name } => format!(
                "The connection to kernel \"{name}\" ended while the request was in flight. Retry once with the same --command-id."
            ),
            Self::SubscriptionEnded { name, last_event_id } => {
                let resume = last_event_id.map_or_else(
                    || "Subscribe again.".to_string(),
                    |id| format!("Subscribe again with resume_from_event_id {id}."),
                );
                format!("The subscription on kernel \"{name}\" ended because the connection was lost or the kernel stopped answering. {resume} If the kernel restarted, expect a replay_gap and read its session state again.")
            }
            Self::JoinRefused { detail } => format!("Could not pair with that machine: {detail}"),
            Self::Remote { message, .. } => message.clone(),
        }
    }

    pub(crate) fn retryable(&self) -> bool {
        match self {
            Self::Unreachable { .. }
            | Self::ListUnavailable { .. }
            | Self::RequestTimeout { .. }
            | Self::ConnectionLost { .. }
            | Self::SubscriptionEnded { .. } => true,
            Self::Remote { retryable, .. } => *retryable,
            _ => false,
        }
    }

    pub(crate) fn transport_error(&self) -> KernelTransportError {
        KernelTransportError {
            code: self.code().to_string(),
            message: self.message(),
            retryable: self.retryable(),
        }
    }
}

/// Codes whose text the gateway or a target's gateway ledger writes itself, so
/// the grant holder may read them in full.
pub(crate) fn is_public_code(code: &str) -> bool {
    matches!(
        code,
        "kernel_unknown"
            | "kernel_ambiguous"
            | "kernel_offline"
            | "kernel_unreachable"
            | "kernel_unpaired"
            | "kernel_incompatible"
            | "kernel_request_timeout"
            | "kernel_connection_lost"
            | "kernel_access_ended"
            | "kernel_outcome_unknown"
            | "kernel_not_available"
            | "kernel_join_refused"
            | "command_id_required"
            | "ledger_unavailable"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transport_failure_reaches_the_agent_without_its_raw_text() {
        let raw = "relay wss://relay.example/?accountId=acct-1&realmId=realm-2: os error 111";
        let detail = plain_cause("the relay connection failed", &raw);
        let error = GatewayError::Unreachable {
            name: "b3".into(),
            detail,
        };
        assert!(!error.message().contains("accountId") && !error.message().contains("os error"));
        assert!(error.message().contains("the relay connection failed"));
        let list = GatewayError::ListUnavailable {
            detail: plain_cause("the relay did not answer", &raw),
        };
        assert_eq!(list.code(), "kernel_unreachable");
        assert!(!list.message().contains("realmId"));
        assert!(list.message().contains("list of your kernels"));
    }
}
