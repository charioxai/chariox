use super::*;

// MP-08 / MP-10: all additions remain in the allocated, unreleased 508/116 bump.
#[test]
fn panes_history_508_range_steering_and_initial_content_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    for (source, expected_hash) in [
        (
            include_str!("../../../../session_history_page.rs"),
            "732bdf072fe9483e319fe931aa6df21f8b6ee417f09c8787436ae6f8655739b8",
        ),
        (
            include_str!("../../types/history.rs"),
            "8307a9b640347089007b61e7989e374e48776d1d7fb3f148a12d21a993d133b8",
        ),
        (
            include_str!("../../../../terminal/stream/records.rs"),
            "652e9b84fe8102871a3c19ad6e0280003c7be6909acd5cca2c8a10b0ac262bb7",
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            expected_hash,
            "MP-08 / MP-10: wire definitions changed; update the allocated protocol and snapshot"
        );
    }
    let expected = serde_json::json!({
        "page": {"entry_index": 10, "sequence_end": 13, "sequence_indices": [10,12,13], "sequence_byte_lengths": [2, 3, 2], "fragment_start": 0, "fragment_end": 3, "total_chars": 3, "entry": {"session_id":"s", "provider_run_id":"r", "agent_id":"a", "source_attachment_id":null, "kind":"provider_output", "merge_key":"m", "text":"é中é", "timestamp_ms":1}},
        "request": {"GetSessionHistoryOutline": {"session_id": "s", "agent_ids": ["a"], "latest_prompt_count": 4, "include_prefetched_content": true}},
        "agent": {"agent_id": "a", "turns": [], "prefetched_blobs": []},
        "blob": {"blob_id": "history:10:10", "kind": "user_prompt", "title": "prompt", "summary": "steer", "sequence_start": 10, "sequence_end": 10, "entry_count": 1, "total_chars": 5, "timestamp_ms": 1, "steering": true},
        "output": {"record_id": 1, "sequence": 10, "sequence_end": 11, "sequence_byte_lengths": [2, 3], "timestamp_ms": 1, "session_id": "s", "provider_run_id": "r", "kind": "provider_output", "recipient_attachment_ids": ["c"], "pending_recipient_attachment_ids": ["c"], "bytes": [195,169,228,184,173]}
    });
    let page: crate::session_history_page::SessionHistoryPageEntry =
        serde_json::from_value(expected["page"].clone()).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(expected["request"].clone()).unwrap();
    let agent: crate::local::SessionHistoryOutlineAgent =
        serde_json::from_value(expected["agent"].clone()).unwrap();
    let blob: crate::local::SessionHistoryOutlineBlob =
        serde_json::from_value(expected["blob"].clone()).unwrap();
    let output: crate::terminal::TerminalOutputRecord =
        serde_json::from_value(expected["output"].clone()).unwrap();
    let snapshot = serde_json::json!({ "page": page, "request": request, "agent": agent, "blob": blob, "output": output });
    assert_eq!(snapshot, expected);
    let hash = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{hash:x}"),
        "0cc528d63467dc46965fdf51075ad217ddc494199eb1388c888f602673443879"
    );

    // Missing optional metadata preserves the legacy shapes.
    let mut legacy = expected["output"].clone();
    legacy.as_object_mut().unwrap().remove("sequence_end");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("sequence_byte_lengths");
    let record: crate::terminal::TerminalOutputRecord =
        serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), legacy);
}
