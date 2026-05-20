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
    outgoing_tx: &RelayOutgoingSender,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    outgoing_tx.try_send(envelope).map_err(|error| {
        let message = match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                "relay outgoing queue overloaded".to_string()
            }
            tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                "relay connection writer is closed".to_string()
            }
        };
        DaemonError::LocalTransport {
            operation: "send relay envelope",
            message,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_outgoing_envelope_fails_when_relay_queue_is_full() {
        let (outgoing_tx, _outgoing_rx) = mpsc::channel(1);
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::Close {
                reason: "first".to_string(),
            },
        )
        .expect("first envelope should fit");

        let error = send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::Close {
                reason: "second".to_string(),
            },
        )
        .expect_err("second envelope should overflow bounded queue");

        assert!(error
            .to_string()
            .contains("relay outgoing queue overloaded"));
    }
}
