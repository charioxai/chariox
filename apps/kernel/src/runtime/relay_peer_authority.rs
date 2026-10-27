//! Internal authority retained from the encrypted, verified peer transport.
//! Never serialize this alongside caller-supplied invocation contexts.
use crate::error::DaemonError;

#[derive(Clone, Debug)]
pub(crate) struct RelayPeerAuthority {
    pub(crate) kernel_id: String,
    pub(crate) public_key: String,
    pub(crate) sender_bound: bool,
}

impl RelayPeerAuthority {
    pub(crate) fn authorize_worker(&self, worker_id: &str) -> Result<(), DaemonError> {
        let pins = crate::config::DaemonConfig::relay_peer_public_key_entries();
        let key_matches = match pins.get(worker_id) {
            Some(expected) => expected == &self.public_key,
            None => self.sender_bound,
        };
        if self.kernel_id != worker_id || !key_matches {
            return Err(DaemonError::RelayTransport {
                operation: "authorize forwarded peer",
                code: "unauthorized".into(),
                message: "authenticated peer does not match the current remote worker".into(),
                retryable: false,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteProjectionAuthority {
    pub(crate) peer: RelayPeerAuthority,
    pub(crate) expected_binding: Option<crate::agent::RemoteAgentBinding>,
    pub(crate) expected_prompt_id: Option<String>,
}

#[cfg(test)]
pub(crate) fn test_peer_authority(worker_id: &str) -> RelayPeerAuthority {
    let public_key = crate::config::DaemonConfig::relay_peer_public_key_entries()
        .remove(worker_id)
        .unwrap_or_else(|| crate::config::DaemonConfig::for_tests().relay_public_key);
    RelayPeerAuthority {
        kernel_id: worker_id.into(),
        public_key,
        sender_bound: true,
    }
}

#[cfg(test)]
pub(crate) fn test_projection_authority(worker_id: &str) -> RemoteProjectionAuthority {
    RemoteProjectionAuthority {
        peer: test_peer_authority(worker_id),
        expected_binding: None,
        expected_prompt_id: None,
    }
}
