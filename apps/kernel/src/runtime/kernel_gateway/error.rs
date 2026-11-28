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
    Released {
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
    /// The target runs a protocol below the oldest this kernel's gateway serves: no retry helps.
    UpdateNeeded {
        name: String,
        kernel_protocol: u32,
        needed_protocol: u32,
    },
    RequestTimeout {
        name: String,
    },
    SteerUnconfirmed {
        command_id: String,
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

/// Cloud issues the relay token a new connection runs on, so failing to get one is not
/// the relay refusing. A Cloud rate limit says how long to wait; only that reaches the agent.
pub(crate) fn cloud_connection_refused(name: &str, error: &dyn std::fmt::Display) -> GatewayError {
    let detail = match crate::transport::relay_client::cloud_retry_hint(&error.to_string()) {
        Some(wait) => {
            tracing::warn!(%error, "Chariox Cloud is limiting new relay connections");
            format!(
                "Chariox Cloud is limiting new relay connections; retry in {} seconds",
                wait.as_secs()
            )
        }
        None => plain_cause("no relay connection was issued", error),
    };
    GatewayError::Unreachable {
        name: name.into(),
        detail,
    }
}

impl GatewayError {
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Unknown { .. } => "kernel_unknown",
            Self::Ambiguous { .. } => "kernel_ambiguous",
            Self::Offline { .. } | Self::Released { .. } => "kernel_offline",
            Self::Unreachable { .. } | Self::ListUnavailable { .. } => "kernel_unreachable",
            Self::Unpaired { .. } | Self::GatewayRevoked => "kernel_unpaired",
            Self::AccessEnded { .. } => "kernel_access_ended",
            Self::Incompatible { .. } => "kernel_incompatible",
            Self::UpdateNeeded { .. } => "kernel_update_needed",
            Self::RequestTimeout { .. } => "kernel_request_timeout",
            Self::SteerUnconfirmed { .. } => "steer_unconfirmed",
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
            Self::Released { name } => format!(
                "The machine for kernel \"{name}\" was released. Run `chariox kernels` and pick a live kernel."
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
            Self::UpdateNeeded {
                name,
                kernel_protocol,
                needed_protocol,
            } => format!(
                "Kernel \"{name}\" runs protocol {kernel_protocol} and needs {needed_protocol}. Update the Chariox kernel on that machine, then try again."
            ),
            Self::RequestTimeout { name } => format!(
                "Kernel \"{name}\" did not answer in time. Look up GetGatewayCommandStatus with the original command ID on that kernel. A retry with the same --command-id uses the original reservation."
            ),
            Self::SteerUnconfirmed { command_id } => format!(
                "The steer is not confirmed yet. It may still arrive. Retry with the same command ID (--command-id {command_id:?})."
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
            | Self::SteerUnconfirmed { .. }
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

#[cfg(test)]
mod tests {
    use super::*;

    // 12:09-12:12Z 11-06: Cloud's 429 on the client token read as a relay refusal.
    #[test]
    fn a_cloud_refusal_is_not_blamed_on_the_relay() {
        let limited = "local transport `cloud relay request` failed: cloud relay request failed with 429: cloud_api_code=rate_limited: Rate limit exceeded, retry in 10 seconds";
        let message = cloud_connection_refused("lanes-executor", &limited).message();
        assert!(!message.contains("relay did not accept"), "{message}");
        assert!(
            message.contains("Chariox Cloud") && message.contains("retry in 10 seconds"),
            "{message}"
        );
        let other = "cloud relay request failed with 503: accountId=acct-1";
        let error = cloud_connection_refused("b3", &other);
        assert_eq!(error.code(), "kernel_unreachable");
        assert!(
            !error.message().contains("relay did not accept")
                && !error.message().contains("accountId"),
            "{}",
            error.message()
        );
    }

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
