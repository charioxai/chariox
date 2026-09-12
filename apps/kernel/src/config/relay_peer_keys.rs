use std::collections::BTreeMap;

use super::persisted_daemon::{load_persisted_daemon_config, persist_daemon_config};
use super::{validate_non_empty, DaemonConfig};
use crate::error::DaemonError;
use crate::transport::relay_crypto;

impl DaemonConfig {
    pub(crate) fn relay_peer_public_key_entries() -> BTreeMap<String, String> {
        load_persisted_daemon_config()
            .relay_peer_public_keys
            .into_iter()
            .filter(|(daemon_id, public_key)| {
                !daemon_id.trim().is_empty() && relay_crypto::decode_public_key(public_key).is_ok()
            })
            .collect()
    }

    pub(crate) fn claim_relay_peer_public_key(
        daemon_id: &str,
        public_key: &str,
    ) -> Result<bool, DaemonError> {
        validate_non_empty("relay_peer_daemon_id", daemon_id)?;
        relay_crypto::decode_public_key(public_key)?;
        let mut persisted = load_persisted_daemon_config();
        match persisted.relay_peer_public_keys.get(daemon_id) {
            Some(existing) => Ok(existing == public_key),
            None => {
                persisted
                    .relay_peer_public_keys
                    .insert(daemon_id.to_string(), public_key.to_string());
                persist_daemon_config(&persisted, "persist relay peer public key")?;
                Ok(true)
            }
        }
    }
}
