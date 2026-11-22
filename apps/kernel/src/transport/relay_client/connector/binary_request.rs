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
