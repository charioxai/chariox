//! Opaque peer replies borrow their incoming ciphertext until the receiver
//! can admit the routed reply to its existing queue.

use std::borrow::Cow;
use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::frame_transport::CHUNK_BYTES;
use crate::outgoing::RelaySender;
use crate::packed_peer_reply::{decoded_len_lower_bound, PackedReplyPlan};
use crate::protocol::{EncryptedRelayPayload, RelayError};
use crate::registry::{DaemonKey, RelayRouteIndex};

use super::support::{deliver_serialized, finish_delivery, Delivery, EncryptedPeerReply};

pub(super) const WIRE_PREFIX: &str = r#"{"kind":"daemon_incoming_peer_response""#;

#[derive(Deserialize)]
pub(super) struct IncomingPeerReply<'a> {
    #[serde(rename = "kind")]
    _kind: IncomingKind,
    #[serde(borrow)]
    pub(super) relay_request_id: Cow<'a, str>,
    #[serde(borrow)]
    pub(super) encrypted_response: Option<OpaquePayload<'a>>,
    pub(super) error: Option<RelayError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum IncomingKind {
    DaemonIncomingPeerResponse,
}

#[derive(Deserialize, Serialize)]
pub(super) struct OpaquePayload<'a> {
    #[serde(borrow)]
    sender_public_key: Cow<'a, str>,
    #[serde(borrow)]
    nonce: Cow<'a, str>,
    #[serde(borrow)]
    ciphertext: Cow<'a, str>,
}

impl<'a> From<&'a EncryptedRelayPayload> for OpaquePayload<'a> {
    fn from(payload: &'a EncryptedRelayPayload) -> Self {
        Self {
            sender_public_key: Cow::Borrowed(&payload.sender_public_key),
            nonce: Cow::Borrowed(&payload.nonce),
            ciphertext: Cow::Borrowed(&payload.ciphertext),
        }
    }
}

#[derive(Serialize)]
struct RoutedPeerReply<'a> {
    kind: &'static str,
    request_id: &'a str,
    from_daemon_id: &'a str,
    encrypted_response: Option<&'a OpaquePayload<'a>>,
    error: Option<&'a RelayError>,
}

fn packing_plan<'a>(
    reply: &RoutedPeerReply<'_>,
    payload: &'a OpaquePayload<'_>,
    scratch: &'a mut [u8],
) -> Option<PackedReplyPlan<'a>> {
    let empty = OpaquePayload {
        sender_public_key: Cow::Borrowed(&payload.sender_public_key),
        nonce: Cow::Borrowed(&payload.nonce),
        ciphertext: Cow::Borrowed(""),
    };
    let mut out = Cursor::new(scratch);
    // If the metadata cannot fit in an existing frame, use the ordinary text writer.
    serde_json::to_writer(
        &mut out,
        &RoutedPeerReply {
            encrypted_response: Some(&empty),
            ..*reply
        },
    )
    .ok()?;
    let len = out.position() as usize;
    let envelope = std::str::from_utf8(&out.into_inner()[..len]).ok()?;
    PackedReplyPlan::new(envelope, &payload.ciphertext)
}

fn deliver_reply(receiver: &RelaySender, reply: &RoutedPeerReply<'_>) -> Delivery {
    let retry = reply.encrypted_response.map(|payload| EncryptedPeerReply {
        request_id: reply.request_id,
        from_daemon_id: reply.from_daemon_id,
        ciphertext_len: payload.ciphertext.len(),
    });
    if let Some(payload) = reply
        .encrypted_response
        .filter(|p| p.ciphertext.len() > CHUNK_BYTES)
    {
        // Refuse early without scanning or allocating a reply that cannot fit even in packed form.
        if let Err(error) =
            receiver.check_peer_reply_size(decoded_len_lower_bound(payload.ciphertext.len()))
        {
            return finish_delivery(receiver, Err(error), retry);
        }
        let mut scratch = [0; CHUNK_BYTES];
        if let Some(plan) =
            packing_plan(reply, payload, &mut scratch).filter(|plan| plan.charge() > CHUNK_BYTES)
        {
            return finish_delivery(receiver, receiver.try_send_packed_peer_reply(plan), retry);
        }
    }
    deliver_serialized(receiver, reply, retry)
}

pub(super) fn forward(
    routes: &RelayRouteIndex,
    registered_daemon: &DaemonKey,
    relay_request_id: &str,
    encrypted_response: Option<&OpaquePayload<'_>>,
    error: Option<&RelayError>,
) {
    let Some(pending) = routes.take_pending_daemon_if(relay_request_id, |pending| {
        &pending.target_daemon_key == registered_daemon
    }) else {
        return;
    };
    let Some(receiver) = routes.daemon_sender(&pending.requester_daemon_key) else {
        return;
    };
    let reply = RoutedPeerReply {
        kind: "daemon_peer_response",
        request_id: &pending.requester_request_id,
        from_daemon_id: &pending.target_daemon_key.daemon_id,
        encrypted_response,
        error,
    };
    deliver_reply(&receiver, &reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use tokio_tungstenite::tungstenite::Message;

    use crate::outgoing::{Outgoing, PeerReply};
    use crate::protocol::RelayEnvelope;

    fn routed<'a>(payload: &'a OpaquePayload<'a>) -> RoutedPeerReply<'a> {
        RoutedPeerReply {
            kind: "daemon_peer_response",
            request_id: r#"a request with "ciphertext":"" in it"#,
            from_daemon_id: "peer",
            encrypted_response: Some(payload),
            error: None,
        }
    }

    #[test]
    fn canonical_ciphertext_frames_reconstruct_the_exact_routed_bytes() {
        let mib = 1024 * 1024;
        let mut scratch = [0; CHUNK_BYTES];
        for size in [0, 1, 2, 3, mib, 2 * mib, 3 * mib, 4 * mib, 17 * mib / 2] {
            let bytes: Vec<u8> = (0..size).map(|index| index as u8).collect();
            let payload = OpaquePayload {
                sender_public_key: Cow::Borrowed("public-key"),
                nonce: Cow::Borrowed("🦀"),
                ciphertext: Cow::Owned(STANDARD.encode(bytes)),
            };
            let reply = routed(&payload);
            let expected = serde_json::to_string(&reply).unwrap();
            let plan = packing_plan(&reply, &payload, &mut scratch).expect("canonical Base64");
            if size >= mib {
                assert!(plan.charge() < expected.len());
            }
            assert_eq!(plan.into_reply().into_test_text(), expected, "{size} bytes");

            let (sender, mut outgoing) = RelaySender::detached_channel(1);
            assert!(matches!(deliver_reply(&sender, &reply), Delivery::Sent));
            assert_eq!(outgoing.try_recv().unwrap(), Message::Text(expected.into()));
        }
    }

    #[test]
    fn noncanonical_ciphertext_is_forwarded_unchanged_as_text() {
        let prefix = "Zm9v".repeat(CHUNK_BYTES / 4 + 1);
        let mut scratch = [0; CHUNK_BYTES];
        for variant in ["AA", "AA====", "AA=\n", "A\rA", "__--", "AB==", "AAB="] {
            let payload = OpaquePayload {
                sender_public_key: Cow::Borrowed("public-key"),
                nonce: Cow::Borrowed("nonce"),
                ciphertext: Cow::Owned(format!("{prefix}{variant}")),
            };
            let reply = routed(&payload);
            assert!(
                packing_plan(&reply, &payload, &mut scratch).is_none(),
                "{variant:?}"
            );
            let (sender, mut outgoing) = RelaySender::channel(1);
            assert!(matches!(deliver_reply(&sender, &reply), Delivery::Sent));
            let Outgoing::PeerReply(PeerReply::Text(text)) = outgoing.try_recv().unwrap() else {
                panic!("expected a text reply for {variant:?}");
            };
            let mut text = PeerReply::Text(text);
            let mut delivered = String::new();
            let mut offset = 0;
            while let Some((frame, end)) = text.next_chunk(1, offset, CHUNK_BYTES) {
                let crate::frame_transport::TransportFrame::TransportChunk { payload, .. } =
                    serde_json::from_str(&frame).unwrap()
                else {
                    panic!("expected a text transport frame");
                };
                delivered.push_str(&payload);
                text.written(end);
                offset = end;
            }
            assert_eq!(
                delivered,
                serde_json::to_string(&reply).unwrap(),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn peer_reply_borrows_ciphertext_and_encodes_the_same_wire() {
        let inbound = RelayEnvelope::DaemonIncomingPeerResponse {
            relay_request_id: "relay-id".into(),
            encrypted_response: Some(EncryptedRelayPayload {
                sender_public_key: "pub".into(),
                nonce: "nonce".into(),
                ciphertext: "cGVlciByZXBseQ==".into(),
            }),
            error: None,
        };
        let text = serde_json::to_string(&inbound).unwrap();
        assert!(text.starts_with(WIRE_PREFIX));
        let decoded: IncomingPeerReply<'_> = serde_json::from_str(&text).unwrap();
        let payload = decoded.encrypted_response.as_ref().unwrap();
        assert!(matches!(
            payload.ciphertext,
            Cow::Borrowed("cGVlciByZXBseQ==")
        ));
        let routed = RoutedPeerReply {
            kind: "daemon_peer_response",
            request_id: "caller-id",
            from_daemon_id: "peer",
            encrypted_response: Some(payload),
            error: None,
        };
        let RelayEnvelope::DaemonIncomingPeerResponse {
            encrypted_response, ..
        } = inbound
        else {
            unreachable!();
        };
        let expected = RelayEnvelope::DaemonPeerResponse {
            request_id: "caller-id".into(),
            from_daemon_id: "peer".into(),
            encrypted_response,
            error: None,
        };
        assert_eq!(
            serde_json::to_string(&routed).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
    }
}
