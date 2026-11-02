//! CXL1 keeps ciphertext opaque and uses the kernel key proved in the handshake.
use super::*;

pub(super) fn compact(text: &str) -> Result<Option<Vec<u8>>, &'static str> {
    let envelope = match serde_json::from_str::<RelayEnvelope>(text) {
        Ok(envelope) => envelope,
        // The handshake and close messages keep their existing text framing.
        Err(_) => return Ok(None),
    };
    let (header, encrypted) = match envelope {
        RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: Some(encrypted),
            error: None,
        } => (
            serde_json::json!({"r": request_id, "n": encrypted.nonce}),
            encrypted,
        ),
        RelayEnvelope::ClientEvent {
            subscription_id,
            event_id,
            encrypted_event: encrypted,
        } => (
            serde_json::json!({"s": subscription_id, "e": event_id, "n": encrypted.nonce}),
            encrypted,
        ),
        _ => return Ok(None),
    };
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&encrypted.ciphertext)
        .map_err(|_| "invalid direct ciphertext")?;
    if ciphertext.len() < 16 {
        return Err("invalid direct ciphertext");
    }
    let header = serde_json::to_vec(&header).map_err(|_| "invalid direct header")?;
    if header.len() > 4096 {
        return Err("direct header too large");
    }
    let mut bytes = Vec::with_capacity(8 + header.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXL1");
    bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&ciphertext);
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn mp08_compact_direct_951_snapshot() {
        let snapshot: serde_json::Value = serde_json::from_str(include_str!(
            "../../../local/api/tests/protocol_shapes/direct-compact-951.json"
        ))
        .unwrap();
        let envelope = RelayEnvelope::ClientResponse {
            request_id: "r".into(),
            error: None,
            encrypted_response: Some(EncryptedRelayPayload {
                sender_public_key: "pinned-kernel-key".into(),
                nonce: "AAAAAAAAAAAAAAAA".into(),
                ciphertext: base64::engine::general_purpose::STANDARD.encode([0_u8; 16]),
            }),
        };
        let bytes = compact(&serde_json::to_string(&envelope).unwrap())
            .unwrap()
            .unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, snapshot["wire_hex"].as_str().unwrap());
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, snapshot["sha256"].as_str().unwrap());
    }
}
