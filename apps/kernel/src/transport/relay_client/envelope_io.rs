//! Relay envelope serialization, encryption, and outgoing send helpers.

use super::*;

pub(super) async fn encrypt_json_response(
    router: &Arc<CommandRouter>,
    client_public_key: &str,
    value: serde_json::Value,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let plaintext = serde_json::to_vec(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay response",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)
}

pub(super) fn encrypt_peer_payload<T: serde::Serialize>(
    sender_private_key: &str,
    peer_public_key: &str,
    value: &T,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let plaintext = serde_json::to_vec(value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer payload",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(sender_private_key, peer_public_key, &plaintext)
}

pub(super) fn send_outgoing_envelope(
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    outgoing_tx
        .send(envelope)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "send relay envelope",
            message: error.to_string(),
        })
}
