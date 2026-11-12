//! MP-08 / MP-09 / MP-10 / MP-11: lossless, bounded peer snapshot encoding.

use std::io::Write;

use base64::Engine;
use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot;
use crate::transport::room_browser_controller::RoomBrowserControllerResult;

const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_COMPRESSED_BYTES: usize = MAX_SNAPSHOT_BYTES + 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    decoded_bytes: usize,
    zlib_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SnapshotResultWire {
    Snapshot { snapshot: Option<SnapshotWire> },
}

// Keep compression at the peer envelope boundary. In-process results must stay
// readable to the Room's observation scrubber before they become transport bytes.
pub(super) fn serialize<S: Serializer>(
    result: &RoomBrowserControllerResult,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::Error;
    let RoomBrowserControllerResult::Snapshot { snapshot } = result else {
        return result.serialize(serializer);
    };
    let snapshot = snapshot
        .as_ref()
        .map(encode)
        .transpose()
        .map_err(S::Error::custom)?;
    SnapshotResultWire::Snapshot { snapshot }.serialize(serializer)
}

fn encode(snapshot: &BrowserControllerStructuredSnapshot) -> Result<SnapshotWire, String> {
    let bytes = serde_json::to_vec(snapshot).map_err(|_| "browser snapshot encoding failed")?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err("browser snapshot exceeds its decoded byte limit".into());
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&bytes)
        .map_err(|_| "browser snapshot compression failed")?;
    let compressed = encoder
        .finish()
        .map_err(|_| "browser snapshot compression failed")?;
    if compressed.len() > MAX_COMPRESSED_BYTES {
        return Err("browser snapshot compressed data exceeds its byte limit".into());
    }
    Ok(SnapshotWire {
        decoded_bytes: bytes.len(),
        zlib_base64: base64::engine::general_purpose::STANDARD.encode(compressed),
    })
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<RoomBrowserControllerResult, D::Error> {
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("snapshot") {
        return serde_json::from_value(value).map_err(D::Error::custom);
    }
    let SnapshotResultWire::Snapshot { snapshot } =
        serde_json::from_value(value).map_err(D::Error::custom)?;
    let snapshot = snapshot.map(decode).transpose().map_err(D::Error::custom)?;
    Ok(RoomBrowserControllerResult::Snapshot { snapshot })
}

fn decode(wire: SnapshotWire) -> Result<BrowserControllerStructuredSnapshot, String> {
    if wire.decoded_bytes == 0 || wire.decoded_bytes > MAX_SNAPSHOT_BYTES {
        return Err("browser snapshot decoded byte count is outside its limit".into());
    }
    if wire.zlib_base64.len() > MAX_COMPRESSED_BYTES.div_ceil(3) * 4 {
        return Err("browser snapshot compressed data exceeds its byte limit".into());
    }
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(&wire.zlib_base64)
        .map_err(|_| "browser snapshot has invalid base64")?;
    if compressed.len() > MAX_COMPRESSED_BYTES {
        return Err("browser snapshot compressed data exceeds its byte limit".into());
    }
    let mut decoder = Decompress::new(true);
    let mut bytes = vec![0; wire.decoded_bytes + 1];
    let status = decoder
        .decompress(&compressed, &mut bytes, FlushDecompress::Finish)
        .map_err(|_| "browser snapshot has invalid zlib data")?;
    if status != Status::StreamEnd
        || decoder.total_in() != compressed.len() as u64
        || decoder.total_out() != wire.decoded_bytes as u64
    {
        return Err("browser snapshot decoded byte count does not match its data".into());
    }
    bytes.truncate(wire.decoded_bytes);
    serde_json::from_slice(&bytes).map_err(|_| "browser snapshot has invalid JSON".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::browser_controller_snapshot::{
        BrowserControllerAccessibilityNode, BrowserControllerDomDocument, BrowserControllerDomNode,
        BrowserControllerNodeBounds,
    };
    use crate::transport::relay_peer::RelayPeerResponse;
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;

    fn snapshot() -> BrowserControllerStructuredSnapshot {
        BrowserControllerStructuredSnapshot {
            browser_generation: 1,
            target_id: "target".into(),
            document_id: "document".into(),
            snapshot_revision: 2,
            accessibility_nodes: (1..=5000)
                .map(|id| BrowserControllerAccessibilityNode {
                    node_ref: format!("backend:{id}"),
                    parent_ref: None,
                    child_refs: Vec::new(),
                    role: "link".into(),
                    name: format!("Computer architecture reference {id}"),
                    description: String::new(),
                    value: String::new(),
                    ignored: false,
                    disabled: false,
                    focused: false,
                    states: Vec::new(),
                })
                .collect(),
            accessibility_truncated: true,
            dom_documents: vec![BrowserControllerDomDocument {
                document_index: 0,
                url: "https://en.wikipedia.org/wiki/Computer".into(),
                owner_node_ref: None,
            }],
            shadow_roots: Vec::new(),
            dom_nodes: (1..=5000)
                .map(|id| BrowserControllerDomNode {
                    node_ref: format!("backend:{id}"),
                    parent_ref: None,
                    document_index: 0,
                    node_type: 1,
                    node_name: "A".into(),
                    text: String::new(),
                    attributes: [("href".into(), format!("/wiki/Computer_{id}"))].into(),
                    bounds: Some(BrowserControllerNodeBounds {
                        x: 1.0,
                        y: id as f64,
                        width: 100.0,
                        height: 20.0,
                    }),
                })
                .collect(),
        }
    }

    #[test]
    fn peer_snapshot_keeps_every_node_in_a_small_lossless_reply() {
        let snapshot = snapshot();
        snapshot.validate("target", "document").unwrap();
        let response = RelayPeerResponse::RoomBrowserController {
            session_id: "room".into(),
            slice_id: "slice".into(),
            result: RoomBrowserControllerResult::Snapshot {
                snapshot: Some(snapshot),
            },
        };
        let wire = serde_json::to_vec(&response).unwrap();
        assert!(
            wire.len() < 256 * 1024,
            "snapshot reply uses {} bytes",
            wire.len()
        );
        let decoded: RelayPeerResponse = serde_json::from_slice(&wire).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn peer_snapshot_wire_protocol_snapshot_and_hash() {
        use sha2::{Digest, Sha256};
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/peer-browser-snapshot-protocol.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["local"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            fixture["peer"],
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&fixture).unwrap())
            ),
            "ad6a1fa6ddfa8c48e94f725c59672e5a35a9e8e33d5973b2dc5b2ebb68e78e6d"
        );
        let response: RelayPeerResponse =
            serde_json::from_value(fixture["response"].clone()).unwrap();
        let decoded: RelayPeerResponse =
            serde_json::from_slice(&serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(decoded, response);
        let none = RoomBrowserControllerResult::Snapshot { snapshot: None };
        assert_eq!(
            serde_json::to_value(&none).unwrap(),
            serde_json::json!({"kind":"snapshot", "snapshot":null})
        );
    }

    fn encoded(bytes: &[u8], decoded_bytes: usize) -> SnapshotWire {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        SnapshotWire {
            decoded_bytes,
            zlib_base64: base64::engine::general_purpose::STANDARD
                .encode(encoder.finish().unwrap()),
        }
    }

    #[test]
    fn peer_snapshot_rejects_expansion_past_the_declared_size() {
        assert!(decode(encoded(&vec![b'x'; 1024 * 1024], 16)).is_err());
        assert!(decode(encoded(b"{}", MAX_SNAPSHOT_BYTES + 1)).is_err());
        assert!(decode(encoded(b"{}", 0)).is_err());
        assert!(decode(encoded(b"{}", 3)).is_err());
    }

    #[test]
    fn peer_snapshot_rejects_corrupt_and_trailing_data() {
        let bytes = serde_json::to_vec(&snapshot()).unwrap();
        let wire = encoded(&bytes, bytes.len());
        let mut compressed = base64::engine::general_purpose::STANDARD
            .decode(wire.zlib_base64)
            .unwrap();
        for removed in 1..=4 {
            assert!(
                decode(SnapshotWire {
                    decoded_bytes: bytes.len(),
                    zlib_base64: base64::engine::general_purpose::STANDARD
                        .encode(&compressed[..compressed.len() - removed]),
                })
                .is_err(),
                "truncated zlib checksum must not count as a complete snapshot"
            );
        }
        compressed.push(0);
        assert!(decode(SnapshotWire {
            decoded_bytes: bytes.len(),
            zlib_base64: base64::engine::general_purpose::STANDARD.encode(&compressed),
        })
        .is_err());
        compressed.truncate(compressed.len() / 2);
        assert!(decode(SnapshotWire {
            decoded_bytes: bytes.len(),
            zlib_base64: base64::engine::general_purpose::STANDARD.encode(compressed),
        })
        .is_err());
        assert!(decode(SnapshotWire {
            decoded_bytes: 2,
            zlib_base64: "invalid!".into()
        })
        .is_err());
    }
}
