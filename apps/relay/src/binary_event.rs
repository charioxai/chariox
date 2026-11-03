//! MP-08/MP-10/MP-11: relay-98 opaque encrypted events. Only routing metadata
//! is decoded here; ciphertext is never interpreted or decrypted by the relay.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::protocol::{EncryptedRelayPayload, RelayEnvelope};

pub const PROTOCOL_VERSION: u32 = 98;
pub const CAPABILITY: &str = "chariox-relay-binary-v98";
pub const VERSION_HEADER: &str = "x-chariox-relay-protocol";
const PAYLOAD_LIMIT: usize = 1024 * 1024;

// MP-08/MP-10: preserve the configured path/authority/query exactly.
// Older relays ignore this optional HTTP handshake header.
// Preserve the public tungstenite error type used by connection callers.
#[allow(clippy::result_large_err)]
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
        .insert(VERSION_HEADER, "98".parse().unwrap());
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

pub fn decode(bytes: &[u8]) -> Result<(EventHeader, &[u8]), std::io::Error> {
    use crate::binary_routing::{invalid, read_route};
    if bytes.len() < 92 || &bytes[..4] != b"CXR1" || bytes[4] > 1 || bytes[15] != 4 {
        return Err(invalid());
    }
    let size = u16::from_be_bytes(bytes[5..7].try_into().unwrap());
    let event_id = u64::from_be_bytes(bytes[7..15].try_into().unwrap());
    if event_id > 9_007_199_254_740_991 {
        return Err(invalid());
    }
    let (subscription_id, payload) = read_route(size, &bytes[92..])?;
    if !(16..=PAYLOAD_LIMIT).contains(&payload.len()) {
        return Err(invalid());
    }
    Ok((
        EventHeader {
            kind: if bytes[4] == 0 {
                "daemon_event"
            } else {
                "client_event"
            }
            .into(),
            subscription_id,
            event_id,
            sender_public_key: STANDARD.encode(&bytes[15..80]),
            nonce: STANDARD.encode(&bytes[80..92]),
        },
        payload,
    ))
}

pub fn encode(header: &EventHeader, ciphertext: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use crate::binary_routing::{encrypted_fields, invalid, route};
    let kind = match header.kind.as_str() {
        "daemon_event" => 0,
        "client_event" => 1,
        _ => return Err(invalid()),
    };
    if !(16..=PAYLOAD_LIMIT).contains(&ciphertext.len()) || header.event_id > 9_007_199_254_740_991
    {
        return Err(invalid());
    }
    let (size, id) = route(&header.subscription_id)?;
    let (key, nonce) = encrypted_fields(&header.sender_public_key, &header.nonce)?;
    let mut bytes = Vec::with_capacity(92 + id.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXR1");
    bytes.push(kind);
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(&header.event_id.to_be_bytes());
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&id);
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
    fn mp08_relay98_binary_event_is_bounded_and_downgrades_without_reading_plaintext() {
        let header = EventHeader {
            kind: "client_event".into(),
            subscription_id: "route".into(),
            event_id: 7,
            sender_public_key: "BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let ciphertext: Vec<_> = (0..65536).map(|n| (n % 256) as u8).collect();
        assert_eq!(PROTOCOL_VERSION, 98);
        let wire = encode(&header, &ciphertext).unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!("{:x}", Sha256::digest(&wire)),
            "5346ba713dcf998d88aebd03f95899479e850e6a67e3a036bfbd765f5af61db6"
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
        assert_eq!(request.headers()[VERSION_HEADER], "98");
    }
}
