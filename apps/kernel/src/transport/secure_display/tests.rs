use super::*;
use crate::transport::relay_crypto;

mod relay;

fn channels(stream: &str) -> (SecureDisplayChannel, SecureDisplayChannel) {
    let kernel_key = relay_crypto::generate_private_key_base64();
    let viewer_key = relay_crypto::generate_private_key_base64();
    let kernel_public = relay_crypto::public_key_from_private_key_base64(&kernel_key).unwrap();
    let viewer_public = relay_crypto::public_key_from_private_key_base64(&viewer_key).unwrap();
    (
        SecureDisplayChannel::new(kernel_key, viewer_public, stream, DisplayPeer::Kernel).unwrap(),
        SecureDisplayChannel::new(viewer_key, kernel_public, stream, DisplayPeer::Viewer).unwrap(),
    )
}

#[test]
fn large_binary_display_frame_crosses_encrypted_fragments_without_text_loss() {
    let (mut kernel, mut viewer) = channels("viewer-1");
    let frame: Vec<u8> = (0..200_000).map(|index| (index % 256) as u8).collect();
    let packets = kernel.encode(DisplayMessageKind::Binary, &frame).unwrap();
    assert!(
        packets.len() > 1,
        "large frames must not monopolize one relay packet"
    );
    let mut received = Vec::new();
    for packet in &packets {
        assert!(serde_json::to_vec(packet).unwrap().len() <= 128 * 1024);
        if let Some(message) = viewer.decode(packet).unwrap() {
            received.push(message);
        }
    }
    assert_eq!(
        received,
        vec![DisplayMessage {
            kind: DisplayMessageKind::Binary,
            data: frame
        }]
    );
    let reply = viewer
        .encode(DisplayMessageKind::Text, b"CLIENT_FRAME_ACK 7")
        .unwrap();
    assert_eq!(
        kernel.decode(&reply[0]).unwrap().unwrap().data,
        b"CLIENT_FRAME_ACK 7"
    );
}

#[test]
fn display_replay_and_reordering_close_the_channel_without_recovery_on_later_packets() {
    let (mut kernel, mut viewer) = channels("replay");
    let first = kernel.encode(DisplayMessageKind::Text, b"first").unwrap();
    let next = kernel.encode(DisplayMessageKind::Text, b"second").unwrap();
    assert!(viewer.decode(&first[0]).unwrap().is_some());
    assert!(viewer.decode(&first[0]).is_err());
    assert!(
        viewer.decode(&next[0]).is_err(),
        "a failed channel must not silently recover"
    );

    let (mut kernel, mut viewer) = channels("ordering");
    let fragments = kernel
        .encode(DisplayMessageKind::Binary, &vec![9; 100_000])
        .unwrap();
    assert!(viewer.decode(&fragments[1]).is_err());
    assert!(viewer.decode(&fragments[0]).is_err());
}

#[test]
fn display_ciphertext_tampering_and_another_viewer_fail_closed() {
    let (mut kernel, mut viewer) = channels("tamper");
    let mut packets = kernel
        .encode(DisplayMessageKind::Text, b"private screen contents")
        .unwrap();
    assert!(!serde_json::to_string(&packets)
        .unwrap()
        .contains("private screen contents"));
    let replacement = if packets[0].ciphertext.starts_with('A') {
        "B"
    } else {
        "A"
    };
    packets[0].ciphertext.replace_range(..1, replacement);
    assert!(viewer.decode(&packets[0]).is_err());

    let (mut kernel, _viewer) = channels("wrong-viewer");
    let (_, mut stranger) = channels("wrong-viewer");
    let packets = kernel
        .encode(DisplayMessageKind::Text, b"private screen contents")
        .unwrap();
    assert!(stranger.decode(&packets[0]).is_err());
}

fn peer_fixture() -> (String, String, String) {
    let kernel_key = relay_crypto::generate_private_key_base64();
    let viewer_key = relay_crypto::generate_private_key_base64();
    let kernel_public = relay_crypto::public_key_from_private_key_base64(&kernel_key).unwrap();
    (kernel_key, viewer_key, kernel_public)
}

fn wire_fragment(sequence: u64, final_fragment: bool, data: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "protocol": "chariox-display-v1", "stream_id": "bound-stream", "sender": "kernel",
        "sequence": sequence, "kind": "binary", "final_fragment": final_fragment,
        "data_base64": BASE64.encode(data),
    })
}

fn send_wire(
    kernel_key: &str,
    viewer_key: &str,
    fragment: &serde_json::Value,
) -> EncryptedRelayPayload {
    relay_crypto::encrypt_payload_for_peer(
        kernel_key,
        &relay_crypto::public_key_from_private_key_base64(viewer_key).unwrap(),
        &serde_json::to_vec(fragment).unwrap(),
    )
    .unwrap()
}

#[test]
fn authenticated_display_context_is_bound_to_stream_direction_version_and_sequence() {
    for (field, value) in [
        ("stream_id", serde_json::json!("different-room-viewer")),
        ("sender", serde_json::json!("viewer")),
        ("protocol", serde_json::json!("unknown-display-protocol")),
        ("sequence", serde_json::json!(1)),
        ("unexpected_field", serde_json::json!(true)),
    ] {
        let (kernel_key, viewer_key, kernel_public) = peer_fixture();
        let mut viewer = SecureDisplayChannel::new(
            viewer_key.clone(),
            kernel_public,
            "bound-stream",
            DisplayPeer::Viewer,
        )
        .unwrap();
        let mut fragment = wire_fragment(0, true, b"secret");
        fragment[field] = value;
        let packet = send_wire(&kernel_key, &viewer_key, &fragment);
        assert!(viewer.decode(&packet).is_err(), "accepted invalid {field}");
    }
}

#[test]
fn display_assembly_and_packet_limits_reject_oversize_without_accepting_partial_frames() {
    let (mut kernel, mut viewer) = channels("limits");
    assert!(kernel
        .encode(DisplayMessageKind::Binary, &vec![0; 4 * 1024 * 1024 + 1])
        .is_err());
    assert!(kernel.encode(DisplayMessageKind::Text, &[0xff]).is_err());
    let mut packet = kernel
        .encode(DisplayMessageKind::Binary, b"small")
        .unwrap()
        .remove(0);
    packet.ciphertext = "A".repeat(128 * 1024 + 1);
    assert!(viewer.decode(&packet).is_err());

    let (kernel_key, viewer_key, kernel_public) = peer_fixture();
    let mut viewer = SecureDisplayChannel::new(
        viewer_key.clone(),
        kernel_public,
        "bound-stream",
        DisplayPeer::Viewer,
    )
    .unwrap();
    for sequence in 0..64 {
        let packet = send_wire(
            &kernel_key,
            &viewer_key,
            &wire_fragment(sequence, false, &vec![8; 65536]),
        );
        assert!(viewer.decode(&packet).unwrap().is_none());
    }
    let overflow = send_wire(&kernel_key, &viewer_key, &wire_fragment(64, true, &[8]));
    assert!(
        viewer.decode(&overflow).is_err(),
        "assembled message exceeded four MiB"
    );
}
