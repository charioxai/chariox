//! Socket-local routing only; authentication remains with the encrypted envelope.
use base64::{engine::general_purpose::STANDARD, Engine};

pub fn invalid() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid compact relay routing",
    )
}

/// Zero length denotes exactly sixteen UUID bytes. Other IDs retain their UTF-8.
pub fn route(id: &str) -> Result<(u16, Vec<u8>), std::io::Error> {
    if id.is_empty() || id.len() > 1024 {
        return Err(invalid());
    }
    if id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
    {
        let hex: String = id.chars().filter(|c| *c != '-').collect();
        let packed = (0..32)
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        return Ok((0, packed));
    }
    Ok((id.len() as u16, id.as_bytes().to_vec()))
}

pub fn read_route(size: u16, bytes: &[u8]) -> Result<(String, &[u8]), std::io::Error> {
    let length = if size == 0 { 16 } else { size as usize };
    if length > 1024 || bytes.len() < length {
        return Err(invalid());
    }
    let id = if size == 0 {
        {
            let hex: String = bytes[..length].iter().map(|b| format!("{b:02x}")).collect();
            format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )
        }
    } else {
        std::str::from_utf8(&bytes[..length])
            .map_err(|_| invalid())?
            .to_string()
    };
    Ok((id, &bytes[length..]))
}

pub fn encrypted_fields(key: &str, nonce: &str) -> Result<(Vec<u8>, Vec<u8>), std::io::Error> {
    let key = STANDARD.decode(key).map_err(|_| invalid())?;
    let nonce = STANDARD.decode(nonce).map_err(|_| invalid())?;
    if key.len() != 65 || key[0] != 4 || nonce.len() != 12 {
        return Err(invalid());
    }
    Ok((key, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_routing_961_snapshot() {
        use crate::{
            binary_event::{self, EventHeader},
            binary_response::{self, ResponseHeader},
        };
        use sha2::{Digest, Sha256};
        let snapshot: serde_json::Value = serde_json::from_str(include_str!(
            "../../kernel/src/local/api/tests/protocol_shapes/relay-compact-961.json"
        ))
        .unwrap();
        for frame in snapshot["frames"].as_array().unwrap() {
            let id = frame["id"].as_str().unwrap().to_string();
            let key = frame["sender_public_key"].as_str().unwrap().to_string();
            let nonce = frame["nonce"].as_str().unwrap().to_string();
            let client = frame["kind"] == 1;
            let wire = if frame["magic"] == "CXR1" {
                let wire = binary_event::encode(
                    &EventHeader {
                        kind: if client {
                            "client_event"
                        } else {
                            "daemon_event"
                        }
                        .into(),
                        subscription_id: id.clone(),
                        event_id: 17,
                        sender_public_key: key,
                        nonce,
                    },
                    &[0; 16],
                )
                .unwrap();
                assert_eq!(binary_event::decode(&wire).unwrap().0.subscription_id, id);
                wire
            } else {
                let header = if client {
                    ResponseHeader::ClientResponse {
                        request_id: id.clone(),
                        sender_public_key: key,
                        nonce,
                    }
                } else {
                    ResponseHeader::DaemonResponse {
                        relay_request_id: id.clone(),
                        sender_public_key: key,
                        nonce,
                    }
                };
                let wire = binary_response::encode(&header, &[0; 16]).unwrap();
                assert!(binary_response::decode(&wire).is_ok());
                wire
            };
            let hex: String = wire.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, frame["wire_hex"]);
            assert_eq!(format!("{:x}", Sha256::digest(&wire)), frame["sha256"]);
        }
    }

    #[test]
    fn canonical_uuid_packing_preserves_every_other_route_exactly() {
        for id in [
            "01234567-89ab-cdef-0123-456789abcdef",
            "01234567-89AB-CDEF-0123-456789ABCDEF",
            "0123456789abcdef0123456789abcdef",
            "{01234567-89ab-cdef-0123-456789abcdef}",
            "\u{feff}route",
            "route",
        ] {
            let (size, bytes) = route(id).unwrap();
            assert_eq!(size == 0, id == "01234567-89ab-cdef-0123-456789abcdef");
            assert_eq!(read_route(size, &bytes).unwrap(), (id.to_string(), &[][..]));
            assert!(read_route(size, &bytes[..bytes.len() - 1]).is_err());
        }
        assert!(route("").is_err());
        assert!(route(&"é".repeat(513)).is_err());
        assert!(route(&"é".repeat(512)).is_ok());
        assert!(read_route(1, &[0xff]).is_err());
    }
}
