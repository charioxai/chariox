//! MP-08/MP-10: opt-in opaque packet timing. No identities or plaintext are logged.
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::RelayEnvelope;

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("CHARIOX_BROWSER_DISPLAY_TIMING").as_deref() == Ok("1"))
}

// A 52-bit digest of ciphertext joins the same opaque packet at each endpoint.
// Authentication/registration envelopes have no signature and are never logged.
fn signature(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(Sha256::digest(bytes)[..8].try_into().unwrap()) >> 12
}

pub fn envelope_signature(envelope: &RelayEnvelope) -> Option<u64> {
    if !enabled() {
        return None;
    }
    let payload = match envelope {
        RelayEnvelope::ClientRequest {
            encrypted_request, ..
        }
        | RelayEnvelope::DaemonRequest {
            encrypted_request, ..
        } => encrypted_request,
        RelayEnvelope::DaemonEvent {
            encrypted_event, ..
        }
        | RelayEnvelope::ClientEvent {
            encrypted_event, ..
        } => encrypted_event,
        _ => return None,
    };
    STANDARD
        .decode(&payload.ciphertext)
        .ok()
        .map(|bytes| signature(&bytes))
}

pub fn message_signature(message: &Message) -> Option<u64> {
    if !enabled() {
        return None;
    }
    match message {
        Message::Binary(bytes) => crate::binary_event::decode(bytes)
            .ok()
            .map(|(_, bytes)| signature(bytes)),
        Message::Text(text) => serde_json::from_str(text)
            .ok()
            .and_then(|envelope| envelope_signature(&envelope)),
        _ => None,
    }
}

pub fn record(stage: &'static str, packet: Option<u64>) {
    if let Some(packet) = packet {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;
        eprintln!(
            "MP-10-PACKET-TIMING {}",
            serde_json::json!({"stage":stage,"packet":packet,"at_ms":at})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::EncryptedRelayPayload;

    #[test]
    fn opaque_packet_join_ignores_routing_and_rejects_authentication() {
        let event = RelayEnvelope::DaemonEvent {
            subscription_id: "diagnostic".into(),
            event_id: 1,
            encrypted_event: EncryptedRelayPayload {
                sender_public_key: "public".into(),
                nonce: "0000000000000000".into(),
                ciphertext: STANDARD.encode([1, 2, 3, 4]),
            },
        };
        let text = Message::Text(serde_json::to_string(&event).unwrap().into());
        let binary = Message::Binary(
            crate::binary_event::from_envelope(&event)
                .unwrap()
                .unwrap()
                .into(),
        );
        assert_eq!(message_signature(&text), message_signature(&binary));
        if enabled() {
            assert!(message_signature(&text).is_some());
        }
        assert!(message_signature(&Message::Text(
            r#"{"kind":"client_hello","auth_token":"synthetic-do-not-record"}"#.into()
        ))
        .is_none());
        assert!(message_signature(&Message::Ping(vec![1, 2].into())).is_none());
        assert!(message_signature(&Message::Binary(vec![1, 2].into())).is_none());
    }
}
