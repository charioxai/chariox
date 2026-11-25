//! A request (`CXR3`) or peer reply (`CXR2`) the relay sent as a binary frame, back as the
//! envelope text the handlers read.
use chariox_relay::frame_transport::FrameReceiver;
use tokio_tungstenite::tungstenite::Message;

const INVALID_FRAME: &str = "{\"kind\":\"close\",\"reason\":\"invalid relay transport frame\"}";

/// The envelope text of a complete binary message and the receipt of its chunk, like
/// `FrameReceiver::receive` for text. Anything the connection did not negotiate, or that does not
/// decode, becomes a close envelope.
pub(super) fn payload(
    frames: &mut FrameReceiver,
    message: Message,
    negotiated: bool,
    binary_peer: bool,
) -> (Option<String>, Option<String>) {
    let Message::Binary(bytes) = message else {
        return (Some(INVALID_FRAME.into()), None);
    };
    if !negotiated {
        return (Some(INVALID_FRAME.into()), None);
    }
    let Ok((complete, receipt)) = frames.receive_binary(&bytes) else {
        return (Some(INVALID_FRAME.into()), None);
    };
    let Some(complete) = complete else {
        return (None, receipt);
    };
    // A `CXR2` message is a peer reply, which only a connection that negotiated binary peer
    // messages can receive.
    let envelope = if complete.starts_with(b"CXR2") {
        if !binary_peer {
            return (Some(INVALID_FRAME.into()), receipt);
        }
        chariox_relay::binary_response::into_envelope(&complete)
    } else {
        chariox_relay::binary_request::into_envelope(&complete)
    };
    match envelope
        .ok()
        .and_then(|envelope| serde_json::to_string(&envelope).ok())
    {
        Some(text) => (Some(text), receipt),
        None => (Some(INVALID_FRAME.into()), receipt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chariox_relay::protocol::{EncryptedRelayPayload, RelayEnvelope};

    fn peer_reply_frame() -> (Vec<u8>, RelayEnvelope) {
        let envelope = RelayEnvelope::DaemonPeerResponse {
            request_id: "daemon-peer-1".into(),
            from_daemon_id: "kernel-2".into(),
            encrypted_response: Some(EncryptedRelayPayload {
                sender_public_key: "opaque-key".into(),
                nonce: "abcdefghijklmnop".into(),
                ciphertext: STANDARD.encode(vec![0xfe; 4096]),
            }),
            error: None,
        };
        let wire = chariox_relay::binary_response::from_envelope(&envelope)
            .unwrap()
            .unwrap();
        (
            chariox_relay::frame_transport::binary_chunk(1, 0, wire.len(), &wire),
            envelope,
        )
    }

    // Row 207: a peer reply is a `CXR2` message, read only on a connection that negotiated
    // binary peer messages.
    #[test]
    fn a_peer_reply_frame_is_read_only_when_binary_peer_messages_were_negotiated() {
        let (chunk, envelope) = peer_reply_frame();
        let mut frames = FrameReceiver::default();
        let (text, receipt) = payload(
            &mut frames,
            Message::Binary(chunk.clone().into()),
            true,
            true,
        );
        assert!(receipt.is_some());
        assert_eq!(
            serde_json::from_str::<RelayEnvelope>(&text.unwrap()).unwrap(),
            envelope
        );
        let mut frames = FrameReceiver::default();
        let (text, _) = payload(&mut frames, Message::Binary(chunk.into()), true, false);
        assert_eq!(text.as_deref(), Some(INVALID_FRAME));
    }
}
