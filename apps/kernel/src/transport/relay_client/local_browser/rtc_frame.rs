//! CXL2 keeps ciphertext opaque and uses the kernel key proved in the handshake.
use super::*;

pub(super) fn compact(text: &str) -> Result<Option<Vec<u8>>, &'static str> {
    let envelope = match serde_json::from_str::<RelayEnvelope>(text) {
        Ok(envelope) => envelope,
        // The handshake and close messages keep their existing text framing.
        Err(_) => return Ok(None),
    };
    let (id, event_id, encrypted) = match envelope {
        RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: Some(encrypted),
            error: None,
        } => (request_id, None, encrypted),
        RelayEnvelope::ClientEvent {
            subscription_id,
            event_id,
            encrypted_event: encrypted,
        } => (subscription_id, Some(event_id), encrypted),
        _ => return Ok(None),
    };
    let (size, id) =
        chariox_relay::binary_routing::route(&id).map_err(|_| "invalid direct routing id")?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&encrypted.nonce)
        .map_err(|_| "invalid direct nonce")?;
    if nonce.len() != 12 {
        return Err("invalid direct nonce");
    }
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&encrypted.ciphertext)
        .map_err(|_| "invalid direct ciphertext")?;
    if ciphertext.len() < 16 {
        return Err("invalid direct ciphertext");
    }
    let mut bytes = Vec::with_capacity(27 + id.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXL2");
    bytes.push(u8::from(event_id.is_some()));
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(&nonce);
    if let Some(sequence) = event_id {
        bytes.extend_from_slice(&sequence.to_be_bytes());
    }
    bytes.extend_from_slice(&id);
    bytes.extend_from_slice(&ciphertext);
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn mp08_compact_direct_961_snapshot() {
        let snapshot: serde_json::Value = serde_json::from_str(include_str!(
            "../../../local/api/tests/protocol_shapes/direct-compact-961.json"
        ))
        .unwrap();
        assert_eq!(
            snapshot["local_protocol"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            snapshot["peer_protocol"],
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        );
        for frame in snapshot["frames"].as_array().unwrap() {
            let encrypted = EncryptedRelayPayload {
                sender_public_key: "pinned-kernel-key".into(),
                nonce: frame["nonce"].as_str().unwrap().into(),
                ciphertext: base64::engine::general_purpose::STANDARD.encode([0_u8; 16]),
            };
            let id = frame["id"].as_str().unwrap().into();
            let envelope = if let Some(event_id) = frame["event_id"].as_u64() {
                RelayEnvelope::ClientEvent {
                    subscription_id: id,
                    event_id,
                    encrypted_event: encrypted,
                }
            } else {
                RelayEnvelope::ClientResponse {
                    request_id: id,
                    error: None,
                    encrypted_response: Some(encrypted),
                }
            };
            let bytes = compact(&serde_json::to_string(&envelope).unwrap())
                .unwrap()
                .unwrap();
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, frame["wire_hex"].as_str().unwrap());
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(digest, frame["sha256"].as_str().unwrap());
        }
    }

    #[test]
    fn compact_direct_rejects_malformed_routing_and_payload() {
        let valid = serde_json::json!({"kind":"client_response", "request_id":"r", "error":null,
            "encrypted_response":{"sender_public_key":"pinned", "nonce":"AAAAAAAAAAAAAAAA", "ciphertext":"AAAAAAAAAAAAAAAAAAAAAA=="}});
        for (field, values) in [
            (
                "nonce",
                vec![
                    "invalid".to_string(),
                    base64::engine::general_purpose::STANDARD.encode([0_u8; 11]),
                ],
            ),
            (
                "ciphertext",
                vec![
                    "invalid".to_string(),
                    base64::engine::general_purpose::STANDARD.encode([0_u8; 15]),
                ],
            ),
        ] {
            for value in values {
                let mut envelope = valid.clone();
                envelope["encrypted_response"][field] = value.into();
                assert!(compact(&envelope.to_string()).is_err());
            }
        }
        for id in [String::new(), "é".repeat(513)] {
            let mut envelope = valid.clone();
            envelope["request_id"] = id.into();
            assert!(compact(&envelope.to_string()).is_err());
        }
        let mut envelope = valid;
        envelope["request_id"] = "é".repeat(512).into();
        assert!(compact(&envelope.to_string()).unwrap().is_some());
        assert!(compact(r#"{"kind":"close","reason":"closed"}"#)
            .unwrap()
            .is_none());
    }
}
