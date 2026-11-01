//! Opaque successful RPC responses. Routing still uses the existing pending request ledger.
use crate::protocol::{EncryptedRelayPayload, RelayEnvelope};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

const HEADER_LIMIT: usize = 4096;
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseHeader {
    DaemonResponse {
        relay_request_id: String,
        sender_public_key: String,
        nonce: String,
    },
    ClientResponse {
        request_id: String,
        sender_public_key: String,
        nonce: String,
    },
}
impl ResponseHeader {
    fn fields(&self) -> (&str, &str, &str) {
        match self {
            Self::DaemonResponse {
                relay_request_id,
                sender_public_key,
                nonce,
            } => (relay_request_id, sender_public_key, nonce),
            Self::ClientResponse {
                request_id,
                sender_public_key,
                nonce,
            } => (request_id, sender_public_key, nonce),
        }
    }
    fn valid(&self) -> bool {
        let (id, key, nonce) = self.fields();
        !id.is_empty()
            && id.len() <= 1024
            && !key.is_empty()
            && key.len() <= 256
            && nonce.len() == 16
    }
}
fn invalid() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid encrypted relay response",
    )
}
pub fn decode(bytes: &[u8]) -> Result<(ResponseHeader, &[u8]), std::io::Error> {
    if bytes.len() < 8
        || !bytes.starts_with(b"CXR2")
        || bytes.len() > crate::frame_transport::MAX_MESSAGE_BYTES
    {
        return Err(invalid());
    }
    let size = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if !(2..=HEADER_LIMIT).contains(&size) || bytes.len() < 8 + size + 16 {
        return Err(invalid());
    }
    let header: ResponseHeader =
        serde_json::from_slice(&bytes[8..8 + size]).map_err(|_| invalid())?;
    if !header.valid() {
        return Err(invalid());
    }
    Ok((header, &bytes[8 + size..]))
}
pub fn encode(header: &ResponseHeader, ciphertext: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let metadata = serde_json::to_vec(header).map_err(|_| invalid())?;
    if !header.valid()
        || metadata.len() > HEADER_LIMIT
        || ciphertext.len() < 16
        || metadata.len() + 8 + ciphertext.len() > crate::frame_transport::MAX_MESSAGE_BYTES
    {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(8 + metadata.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXR2");
    bytes.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(ciphertext);
    Ok(bytes)
}
pub fn from_envelope(envelope: &RelayEnvelope) -> Result<Option<Vec<u8>>, std::io::Error> {
    let (header, payload) = match envelope {
        RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: Some(p),
            error: None,
        } => (
            ResponseHeader::DaemonResponse {
                relay_request_id: relay_request_id.clone(),
                sender_public_key: p.sender_public_key.clone(),
                nonce: p.nonce.clone(),
            },
            p,
        ),
        RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: Some(p),
            error: None,
        } => (
            ResponseHeader::ClientResponse {
                request_id: request_id.clone(),
                sender_public_key: p.sender_public_key.clone(),
                nonce: p.nonce.clone(),
            },
            p,
        ),
        _ => return Ok(None),
    };
    let ciphertext = STANDARD
        .decode(&payload.ciphertext)
        .map_err(|_| invalid())?;
    encode(&header, &ciphertext).map(Some)
}
pub fn into_envelope(bytes: &[u8]) -> Result<RelayEnvelope, std::io::Error> {
    let (header, ciphertext) = decode(bytes)?;
    let (_, key, nonce) = header.fields();
    let payload = Some(EncryptedRelayPayload {
        sender_public_key: key.into(),
        nonce: nonce.into(),
        ciphertext: STANDARD.encode(ciphertext),
    });
    Ok(match header {
        ResponseHeader::DaemonResponse {
            relay_request_id, ..
        } => RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: payload,
            error: None,
        },
        ResponseHeader::ClientResponse { request_id, .. } => RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: payload,
            error: None,
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp09_mp10_mp11_response_722_snapshot_hash() {
        use sha2::{Digest, Sha256};
        let header = ResponseHeader::ClientResponse {
            request_id: "request".into(),
            sender_public_key: "opaque-key".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let wire = encode(&header, &[0xfe; 16]).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&wire)),
            "1e9f9c294223a05c70682865f1740e9812b62316ca82c048a7fd9be2c92f4889"
        );
    }
    #[test]
    fn response_ciphertext_roundtrip_preserves_both_request_names() {
        for envelope in [
            RelayEnvelope::DaemonResponse {
                relay_request_id: "relay-mirror".into(),
                encrypted_response: Some(EncryptedRelayPayload {
                    sender_public_key: "opaque-key".into(),
                    nonce: "abcdefghijklmnop".into(),
                    ciphertext: STANDARD.encode(vec![0xfe; 100 * 1024]),
                }),
                error: None,
            },
            RelayEnvelope::ClientResponse {
                request_id: "client-mirror".into(),
                encrypted_response: Some(EncryptedRelayPayload {
                    sender_public_key: "opaque-key".into(),
                    nonce: "abcdefghijklmnop".into(),
                    ciphertext: STANDARD.encode(vec![0xfe; 100 * 1024]),
                }),
                error: None,
            },
        ] {
            let wire = from_envelope(&envelope).unwrap().unwrap();
            assert!(wire.len() < 101 * 1024);
            assert_eq!(into_envelope(&wire).unwrap(), envelope);
        }
    }
    #[test]
    fn response_bounds_and_route_schema_fail_without_echoing_ciphertext() {
        let good = ResponseHeader::ClientResponse {
            request_id: "request".into(),
            sender_public_key: "opaque-key".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let mut wire = encode(&good, &[0xfe; 16]).unwrap();
        assert!(decode(&wire[..wire.len() - 1]).is_err());
        wire[3] = b'1';
        assert!(decode(&wire).is_err());
        for id in ["".to_string(), "x".repeat(1025)] {
            assert!(encode(
                &ResponseHeader::ClientResponse {
                    request_id: id,
                    sender_public_key: "key".into(),
                    nonce: "abcdefghijklmnop".into()
                },
                &[1; 16]
            )
            .is_err());
        }
        assert!(from_envelope(&RelayEnvelope::ClientResponse {
            request_id: "request".into(),
            encrypted_response: None,
            error: None
        })
        .unwrap()
        .is_none());
        let metadata=br#"{"kind":"client_response","request_id":"request","relay_request_id":"forged","sender_public_key":"key","nonce":"abcdefghijklmnop"}"#;
        let forged = [
            b"CXR2".as_slice(),
            &(metadata.len() as u32).to_be_bytes(),
            metadata,
            &[1; 16],
        ]
        .concat();
        assert!(decode(&forged).is_err());
    }
}
