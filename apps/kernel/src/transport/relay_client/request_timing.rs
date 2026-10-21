//! MP-08 / MP-10 / MP-11: opt-in, opaque per-request drill joins.
use base64::{engine::general_purpose::STANDARD, Engine};
use chariox_relay::protocol::EncryptedRelayPayload;
use sha2::{Digest, Sha256};

pub(super) fn packet(payload: &EncryptedRelayPayload) -> Option<u64> {
    if !crate::runtime::perf_timing::enabled() {
        return None;
    }
    ciphertext_packet(&payload.ciphertext)
}

fn ciphertext_packet(ciphertext: &str) -> Option<u64> {
    let ciphertext = STANDARD.decode(ciphertext).ok()?;
    Some(u64::from_be_bytes(Sha256::digest(ciphertext)[..8].try_into().ok()?) >> 12)
}

pub(super) fn record(stage: &'static str, packet: Option<u64>) {
    if let Some(packet) = packet {
        // No request, identity, routing, key, nonce or credential data is emitted.
        eprintln!(
            "MP-10-PACKET-TIMING {}",
            serde_json::json!({
                "stage":stage,"packet":packet,"at_ms":crate::session::unix_epoch_ms()
            })
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn opaque_join_matches_the_browser_without_payload_or_routing() {
        use base64::Engine;
        assert_eq!(super::ciphertext_packet("not base64!"), None);
        assert_eq!(
            super::ciphertext_packet(&super::STANDARD.encode(b"opaque ciphertext")),
            Some(3467866110061906)
        );
    }
}
