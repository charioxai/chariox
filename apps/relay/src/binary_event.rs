//! MP-08/MP-10/MP-11: peer-96 opaque encrypted events. Only routing metadata
//! is decoded here; ciphertext is never interpreted or decrypted by the relay.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::protocol::{EncryptedRelayPayload, RelayEnvelope};

pub const PROTOCOL_VERSION: u32 = 96;
pub const CAPABILITY: &str = "chariox-relay-binary-v96";
pub const VERSION_HEADER: &str = "x-chariox-relay-protocol";
const HEADER_LIMIT: usize = 4096;
const PAYLOAD_LIMIT: usize = 1024 * 1024;

// MP-08/MP-10: preserve the configured path/authority/query exactly.
// Older relays ignore this optional HTTP handshake header.
pub fn connection_request(
    url: &str,
) -> Result<
    tokio_tungstenite::tungstenite::handshake::client::Request,
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert(VERSION_HEADER, "96".parse().unwrap());
    Ok(request)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventHeader {
    pub kind: String,
    pub subscription_id: String,
    pub event_id: u64,
    pub sender_public_key: String,
    pub nonce: String,
}

impl EventHeader {
    fn valid(&self) -> bool {
        matches!(self.kind.as_str(), "daemon_event" | "client_event")
            && !self.subscription_id.is_empty()
            && self.subscription_id.len() <= 1024
            && self.sender_public_key.len() <= 256
            && !self.sender_public_key.is_empty()
            && self.nonce.len() == 16
    }
}

pub fn decode(bytes: &[u8]) -> Result<(EventHeader, &[u8]), std::io::Error> {
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid encrypted relay event",
        )
    };
    if bytes.len() < 8 || &bytes[..4] != b"CXR1" {
        return Err(invalid());
    }
    let len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if !(2..=HEADER_LIMIT).contains(&len) || bytes.len() < 8 + len + 16 {
        return Err(invalid());
    }
    let header: EventHeader = serde_json::from_slice(&bytes[8..8 + len]).map_err(|_| invalid())?;
    let payload = &bytes[8 + len..];
    if !header.valid() || payload.len() > PAYLOAD_LIMIT {
        return Err(invalid());
    }
    Ok((header, payload))
}

pub fn encode(header: &EventHeader, ciphertext: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let metadata = serde_json::to_vec(header).map_err(std::io::Error::other)?;
    if !header.valid()
        || metadata.len() > HEADER_LIMIT
        || !(16..=PAYLOAD_LIMIT).contains(&ciphertext.len())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid encrypted relay event",
        ));
    }
    let mut bytes = Vec::with_capacity(8 + metadata.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXR1");
    bytes.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(ciphertext);
    Ok(bytes)
}

pub fn from_envelope(envelope: &RelayEnvelope) -> Result<Option<Vec<u8>>, std::io::Error> {
    let (kind, subscription_id, event_id, payload) = match envelope {
        RelayEnvelope::DaemonEvent {
            subscription_id,
            event_id,
            encrypted_event,
        } => ("daemon_event", subscription_id, event_id, encrypted_event),
        _ => return Ok(None),
    };
    let ciphertext = STANDARD
        .decode(&payload.ciphertext)
        .map_err(|_| std::io::Error::other("invalid relay ciphertext encoding"))?;
    encode(
        &EventHeader {
            kind: kind.into(),
            subscription_id: subscription_id.clone(),
            event_id: *event_id,
            sender_public_key: payload.sender_public_key.clone(),
            nonce: payload.nonce.clone(),
        },
        &ciphertext,
    )
    .map(Some)
}

pub fn legacy_client_envelope(bytes: &[u8]) -> Result<RelayEnvelope, std::io::Error> {
    let (header, ciphertext) = decode(bytes)?;
    if header.kind != "client_event" {
        return Err(std::io::Error::other("invalid client event route"));
    }
    Ok(RelayEnvelope::ClientEvent {
        subscription_id: header.subscription_id,
        event_id: header.event_id,
        encrypted_event: EncryptedRelayPayload {
            sender_public_key: header.sender_public_key,
            nonce: header.nonce,
            ciphertext: STANDARD.encode(ciphertext),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_peer96_binary_event_is_bounded_and_downgrades_without_reading_plaintext() {
        let header = EventHeader {
            kind: "client_event".into(),
            subscription_id: "route".into(),
            event_id: 7,
            sender_public_key: "opaque-peer-key".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let ciphertext: Vec<_> = (0..65536).map(|n| (n % 256) as u8).collect();
        assert_eq!(PROTOCOL_VERSION, 96);
        let wire = encode(&header, &ciphertext).unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!("{:x}", Sha256::digest(&wire)),
            "40630ddc3753273ffb3d0c6b7f3e86346fcd650579cb032db572cc657fb722d2"
        );
        let (decoded, raw) = decode(&wire).unwrap();
        assert_eq!(decoded.subscription_id, "route");
        assert_eq!(raw, ciphertext);
        let legacy = legacy_client_envelope(&wire).unwrap();
        assert!(serde_json::to_vec(&legacy).unwrap().len() > wire.len() * 13 / 10);
        let mut bad = wire.clone();
        bad[4..8].copy_from_slice(&4097u32.to_be_bytes());
        assert!(decode(&bad).is_err());
        assert!(decode(&wire[..12]).is_err());
        assert!(encode(&header, &[0; 15]).is_err());
        assert!(encode(&header, &vec![0; PAYLOAD_LIMIT + 1]).is_err());
        assert_eq!(
            connection_request("wss://example/relay?x=1")
                .unwrap()
                .uri()
                .to_string(),
            "wss://example/relay?x=1"
        );
    }
    #[test]
    fn mp08_peer96_authority_only_url_preserves_the_websocket_host_and_port() {
        let request = connection_request("ws://127.0.0.1:1234").unwrap();
        assert_eq!(request.uri().host(), Some("127.0.0.1"));
        assert_eq!(request.uri().port_u16(), Some(1234));
        assert_eq!(request.uri().path(), "/");
        assert_eq!(request.uri().query(), None);
        assert_eq!(request.headers()[VERSION_HEADER], "96");
    }
}
