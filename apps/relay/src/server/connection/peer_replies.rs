//! Opaque peer replies borrow their incoming ciphertext until the receiver
//! can admit the routed reply to its existing queue.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::protocol::{EncryptedRelayPayload, RelayError};
use crate::registry::{DaemonKey, RelayRouteIndex};

use super::support::{deliver_serialized, EncryptedPeerReply};

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
    let retry = encrypted_response.map(|payload| EncryptedPeerReply {
        request_id: reply.request_id,
        from_daemon_id: reply.from_daemon_id,
        ciphertext_len: payload.ciphertext.len(),
    });
    deliver_serialized(&receiver, &reply, retry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::RelayEnvelope;

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
