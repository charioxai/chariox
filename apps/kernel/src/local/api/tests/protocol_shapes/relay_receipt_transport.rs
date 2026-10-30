// MP-08 / MP-10 / MP-11: allocated Environment transport semantics and wire bounds.
use chariox_relay::frame_transport::{
    TransportFrame, CHUNK_BYTES, MAX_MESSAGE_BYTES, TRANSPORT_QUERY, WINDOW_BYTES,
};
use sha2::{Digest, Sha256};

#[test]
fn environment_relay_receipt_transport_snapshot_is_versioned() {
    let local = crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;
    let peer = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
    assert_eq!(
        (local, peer),
        (
            crate::test_support::expected_local_protocol_version(),
            crate::test_support::expected_relay_peer_protocol_version()
        )
    );
    let snapshot = serde_json::json!({
        "local": local,
        "peer": peer,
        "query": TRANSPORT_QUERY,
        "chunk_bytes": CHUNK_BYTES,
        "window_bytes": WINDOW_BYTES,
        "max_message_bytes": MAX_MESSAGE_BYTES,
        "frames": [
            serde_json::to_value(TransportFrame::TransportChunk {
                transfer_id: 1, offset: 0, total_bytes: 17000,
                payload: "public".to_string(),
            }).unwrap(),
            serde_json::to_value(TransportFrame::TransportAck {
                transfer_id: 1, offset: 6,
            }).unwrap(),
        ],
    });
    // The real writer drill sequential_responses_do_not_wait_for_the_previous_final_receipt
    // verifies connection-wide credit and compatibility with the strict receiver.
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "c6e38c65524e07188e77344bb5460b9c0244376c7bf69d93985c821935b2d8d8",
    );
}
