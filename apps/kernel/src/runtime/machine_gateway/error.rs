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

impl GatewayError {
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Unknown { .. } => "machine_unknown",
            Self::Ambiguous { .. } => "machine_ambiguous",
            Self::Offline { .. } => "machine_offline",
            Self::Unreachable { .. } => "machine_unreachable",
            Self::Unpaired { .. } => "machine_unpaired",
            Self::Incompatible { .. } => "machine_incompatible",
            Self::RequestTimeout { .. } => "machine_request_timeout",
            Self::ConnectionLost { .. } | Self::SubscriptionEnded { .. } => {
                "machine_connection_lost"
            }
            Self::JoinRefused { .. } => "machine_join_refused",
            Self::Remote { code, .. } => code,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Unknown { name } => format!(
                "No machine is named \"{name}\". Run `chariox machines` to list the machines you can use."
            ),
            Self::Ambiguous { name, candidates } => format!(
                "\"{name}\" matches several machines ({}). Run `chariox machines` and use a kernel id.",
                candidates.join(", ")
            ),
            Self::Offline { name } => format!(
                "Machine \"{name}\" is offline. Start the Chariox kernel there; ssh is only a break-glass."
            ),
            Self::Unreachable { name, detail } => format!(
                "Could not reach machine \"{name}\" ({detail}). Retry; check that the relay is reachable."
            ),
            Self::Unpaired { name } => format!(
                "Machine \"{name}\" does not accept this kernel. Pair it again from a Chariox terminal."
            ),
            Self::Incompatible { name, detail } => format!(
                "Machine \"{name}\" cannot run this request ({detail}). Update the Chariox kernel on that machine."
            ),
            Self::RequestTimeout { name } => format!(
                "Machine \"{name}\" did not answer in time. For a state-changing request retry once with the same --command-id; the machine never runs it twice."
            ),
            Self::ConnectionLost { name } => format!(
                "The connection to machine \"{name}\" ended while the request was in flight. Retry once with the same --command-id."
            ),
            Self::SubscriptionEnded { name, last_event_id } => {
                let resume = last_event_id.map_or_else(
                    || "Subscribe again.".to_string(),
                    |id| format!("Subscribe again with resume_from_event_id {id}."),
                );
                format!("The subscription on machine \"{name}\" ended because the connection was lost. {resume}")
            }
            Self::JoinRefused { detail } => format!("Could not pair with that machine: {detail}"),
            Self::Remote { message, .. } => message.clone(),
        }
    }

    pub(crate) fn retryable(&self) -> bool {
        match self {
            Self::Unreachable { .. }
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
        "machine_unknown"
            | "machine_ambiguous"
            | "machine_offline"
            | "machine_unreachable"
            | "machine_unpaired"
            | "machine_incompatible"
            | "machine_request_timeout"
            | "machine_connection_lost"
            | "machine_outcome_unknown"
            | "machine_not_available"
            | "machine_join_refused"
            | "command_id_required"
            | "ledger_unavailable"
    )
}
