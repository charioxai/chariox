//! Opaque encrypted requests as bytes. A client request and the daemon request the relay forwards
//! carry their ciphertext after a small routing header (`CXR3`), the way answers (`CXR2`) and
//! events (`CXR1`) do, so a file chunk is not inflated by base64 on the way to the kernel.
//! Only routing metadata is decoded here; the ciphertext is never interpreted.
use crate::protocol::{ClientTarget, EncryptedRelayPayload, RelayCallerIdentity, RelayEnvelope};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

/// A client offers binary requests in its handshake next to the binary events, and the relay
/// answers with the same header when it takes them. A daemon that offers it is sent the requests
/// it receives as `CXR3` frames; a client that offers it may send its own that way.
pub const HEADER: &str = "x-chariox-relay-requests";
pub const OFFER: &str = "binary-v1";
const HEADER_LIMIT: usize = 4096;

/// The handshake request with the offer added (see `binary_event::connection_request`).
pub fn offer(
    mut request: tokio_tungstenite::tungstenite::handshake::client::Request,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    request
        .headers_mut()
        .insert(HEADER, OFFER.parse().expect("a valid header value"));
    request
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestHeader {
    ClientRequest {
        request_id: String,
        target: ClientTarget,
        sender_public_key: String,
        nonce: String,
    },
    DaemonRequest {
        relay_request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller_identity: Option<RelayCallerIdentity>,
        sender_public_key: String,
        nonce: String,
    },
}

impl RequestHeader {
    fn fields(&self) -> (&str, &str, &str) {
        match self {
            Self::ClientRequest {
                request_id,
                sender_public_key,
                nonce,
                ..
            } => (request_id, sender_public_key, nonce),
            Self::DaemonRequest {
                relay_request_id,
                sender_public_key,
                nonce,
                ..
            } => (relay_request_id, sender_public_key, nonce),
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
        "invalid encrypted relay request",
    )
}

pub fn decode(bytes: &[u8]) -> Result<(RequestHeader, &[u8]), std::io::Error> {
    if bytes.len() < 8
        || !bytes.starts_with(b"CXR3")
        || bytes.len() > crate::frame_transport::MAX_MESSAGE_BYTES
    {
        return Err(invalid());
    }
    let size = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if !(2..=HEADER_LIMIT).contains(&size) || bytes.len() < 8 + size + 16 {
        return Err(invalid());
    }
    let header: RequestHeader =
        serde_json::from_slice(&bytes[8..8 + size]).map_err(|_| invalid())?;
    if !header.valid() {
        return Err(invalid());
    }
    Ok((header, &bytes[8 + size..]))
}

pub fn encode(header: &RequestHeader, ciphertext: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let metadata = serde_json::to_vec(header).map_err(|_| invalid())?;
    if !header.valid()
        || metadata.len() > HEADER_LIMIT
        || ciphertext.len() < 16
        || metadata.len() + 8 + ciphertext.len() > crate::frame_transport::MAX_MESSAGE_BYTES
    {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(8 + metadata.len() + ciphertext.len());
    bytes.extend_from_slice(b"CXR3");
    bytes.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(ciphertext);
    Ok(bytes)
}

/// The binary frame of a request envelope, or `None` for any other envelope.
pub fn from_envelope(envelope: &RelayEnvelope) -> Result<Option<Vec<u8>>, std::io::Error> {
    let (header, payload) = match envelope {
        RelayEnvelope::ClientRequest {
            request_id,
            target,
            encrypted_request: p,
        } => (
            RequestHeader::ClientRequest {
                request_id: request_id.clone(),
                target: target.clone(),
                sender_public_key: p.sender_public_key.clone(),
                nonce: p.nonce.clone(),
            },
            p,
        ),
        RelayEnvelope::DaemonRequest {
            relay_request_id,
            caller_identity,
            encrypted_request: p,
        } => (
            RequestHeader::DaemonRequest {
                relay_request_id: relay_request_id.clone(),
                caller_identity: caller_identity.clone(),
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
    let encrypted_request = EncryptedRelayPayload {
        sender_public_key: key.into(),
        nonce: nonce.into(),
        ciphertext: STANDARD.encode(ciphertext),
    };
    Ok(match header {
        RequestHeader::ClientRequest {
            request_id, target, ..
        } => RelayEnvelope::ClientRequest {
            request_id,
            target,
            encrypted_request,
        },
        RequestHeader::DaemonRequest {
            relay_request_id,
            caller_identity,
            ..
        } => RelayEnvelope::DaemonRequest {
            relay_request_id,
            caller_identity,
            encrypted_request,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::RelaySubjectKind;

    fn payload(len: usize) -> EncryptedRelayPayload {
        EncryptedRelayPayload {
            sender_public_key: "opaque-key".into(),
            nonce: "abcdefghijklmnop".into(),
            ciphertext: STANDARD.encode(vec![0xfe; len]),
        }
    }

    #[test]
    fn a_request_travels_as_its_ciphertext_bytes_and_comes_back_unchanged() {
        for envelope in [
            RelayEnvelope::ClientRequest {
                request_id: "client-request".into(),
                target: ClientTarget {
                    daemon_id: Some("kernel-1".into()),
                    daemon_alias: None,
                },
                encrypted_request: payload(65 * 1024),
            },
            RelayEnvelope::DaemonRequest {
                relay_request_id: "relay-request".into(),
                caller_identity: Some(RelayCallerIdentity {
                    realm_id: "realm".into(),
                    subject: "terminal".into(),
                    subject_kind: RelaySubjectKind::Client,
                    expires_at_ms: 7,
                    token_id: Some("token".into()),
                    user_id: Some("owner".into()),
                    public_key_thumbprint: None,
                }),
                encrypted_request: payload(65 * 1024),
            },
        ] {
            let wire = from_envelope(&envelope).unwrap().unwrap();
            assert!(
                wire.len() < 65 * 1024 + 1024,
                "the ciphertext is carried as bytes, not as base64: {} bytes",
                wire.len()
            );
            assert_eq!(into_envelope(&wire).unwrap(), envelope);
        }
    }

    #[test]
    fn bounds_and_route_schema_fail_without_echoing_ciphertext() {
        let good = RequestHeader::ClientRequest {
            request_id: "request".into(),
            target: ClientTarget {
                daemon_id: None,
                daemon_alias: None,
            },
            sender_public_key: "opaque-key".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let mut wire = encode(&good, &[0xfe; 16]).unwrap();
        assert!(decode(&wire[..wire.len() - 1]).is_err());
        wire[3] = b'2';
        assert!(decode(&wire).is_err());
        for id in ["".to_string(), "x".repeat(1025)] {
            assert!(encode(
                &RequestHeader::DaemonRequest {
                    relay_request_id: id,
                    caller_identity: None,
                    sender_public_key: "key".into(),
                    nonce: "abcdefghijklmnop".into()
                },
                &[1; 16]
            )
            .is_err());
        }
        assert!(from_envelope(&RelayEnvelope::Close {
            reason: "x".into(),
            kernel_reload: false
        })
        .unwrap()
        .is_none());
        let metadata = br#"{"kind":"client_request","request_id":"request","target":{},"relay_request_id":"forged","sender_public_key":"key","nonce":"abcdefghijklmnop"}"#;
        let forged = [
            b"CXR3".as_slice(),
            &(metadata.len() as u32).to_be_bytes(),
            metadata,
            &[1; 16],
        ]
        .concat();
        assert!(decode(&forged).is_err());
    }
}
