//! Encrypted display frames. Admission and input authority remain kernel-owned.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chariox_relay::protocol::EncryptedRelayPayload;
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::transport::relay_crypto;

const DISPLAY_WIRE_PROTOCOL: &str = "chariox-display-v1";
const FRAGMENT_BYTES: usize = 64 * 1024;
const MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const ENCRYPTED_PACKET_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DisplayPeer {
    Kernel,
    Viewer,
}

impl DisplayPeer {
    fn opposite(self) -> Self {
        match self {
            Self::Kernel => Self::Viewer,
            Self::Viewer => Self::Kernel,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DisplayMessageKind {
    Text,
    Binary,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DisplayMessage {
    pub(crate) kind: DisplayMessageKind,
    pub(crate) data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayFragment {
    protocol: String,
    stream_id: String,
    sender: DisplayPeer,
    sequence: u64,
    kind: DisplayMessageKind,
    final_fragment: bool,
    data_base64: String,
}

/// One admitted viewer connection, never a Room, permission, or history owner.
/// The caller supplies the admitted peer key and a fresh, unpredictable stream
/// ID. Reconnect must create a new ID rather than replaying display packets.
/// Packets use the existing relay encryption; all routing context is inside
/// the authenticated ciphertext. A decoding failure permanently closes this
/// channel. Queue limits, lease expiry, and upstream input filtering belong to
/// the transport caller and are not granted by successful frame decryption.
pub(crate) struct SecureDisplayChannel {
    private_key: String,
    peer_key: String,
    stream_id: String,
    role: DisplayPeer,
    send_sequence: u64,
    receive_sequence: u64,
    partial_kind: Option<DisplayMessageKind>,
    partial: Vec<u8>,
    failed: bool,
}

impl SecureDisplayChannel {
    pub(crate) fn new(
        private_key: String,
        peer_key: String,
        stream_id: &str,
        role: DisplayPeer,
    ) -> Result<Self, DaemonError> {
        if stream_id.is_empty() || stream_id.len() > 128 {
            return Err(display_error("invalid display stream identity"));
        }
        Ok(Self {
            private_key,
            peer_key,
            stream_id: stream_id.to_owned(),
            role,
            send_sequence: 0,
            receive_sequence: 0,
            partial_kind: None,
            partial: Vec::new(),
            failed: false,
        })
    }

    pub(crate) fn encode(
        &mut self,
        kind: DisplayMessageKind,
        data: &[u8],
    ) -> Result<Vec<EncryptedRelayPayload>, DaemonError> {
        if self.failed || data.len() > MESSAGE_BYTES {
            return Err(display_error("closed display channel or oversized message"));
        }
        if kind == DisplayMessageKind::Text && std::str::from_utf8(data).is_err() {
            return Err(display_error("display text must be UTF-8"));
        }
        let count = data.len().div_ceil(FRAGMENT_BYTES).max(1);
        let next_sequence = self
            .send_sequence
            .checked_add(count as u64)
            .ok_or_else(|| display_error("display sequence exhausted"))?;
        let mut packets = Vec::with_capacity(count);
        for index in 0..count {
            let start = index * FRAGMENT_BYTES;
            let end = (start + FRAGMENT_BYTES).min(data.len());
            let fragment = DisplayFragment {
                protocol: DISPLAY_WIRE_PROTOCOL.to_owned(),
                stream_id: self.stream_id.clone(),
                sender: self.role,
                sequence: self.send_sequence + index as u64,
                kind,
                final_fragment: index + 1 == count,
                data_base64: BASE64.encode(&data[start..end]),
            };
            let plaintext = serde_json::to_vec(&fragment)
                .map_err(|_| display_error("cannot encode display fragment"))?;
            packets.push(relay_crypto::encrypt_payload_for_peer(
                &self.private_key,
                &self.peer_key,
                &plaintext,
            )?);
        }
        self.send_sequence = next_sequence;
        Ok(packets)
    }

    pub(crate) fn decode(
        &mut self,
        packet: &EncryptedRelayPayload,
    ) -> Result<Option<DisplayMessage>, DaemonError> {
        let result = self.decode_fragment(packet);
        if result.is_err() {
            self.failed = true;
            self.partial = Vec::new();
            self.partial_kind = None;
        }
        result
    }

    fn decode_fragment(
        &mut self,
        packet: &EncryptedRelayPayload,
    ) -> Result<Option<DisplayMessage>, DaemonError> {
        if self.failed
            || packet.sender_public_key != self.peer_key
            || packet.ciphertext.len() > ENCRYPTED_PACKET_BYTES
            || packet.nonce.len() > 24
        {
            return Err(display_error(
                "closed channel, unexpected peer, or oversized packet",
            ));
        }
        let plaintext = relay_crypto::decrypt_payload_for_private_key(&self.private_key, packet)?;
        let fragment: DisplayFragment = serde_json::from_slice(&plaintext.plaintext)
            .map_err(|_| display_error("invalid encrypted display fragment"))?;
        if fragment.protocol != DISPLAY_WIRE_PROTOCOL
            || fragment.stream_id != self.stream_id
            || fragment.sender != self.role.opposite()
            || fragment.sequence != self.receive_sequence
        {
            return Err(display_error(
                "display stream, direction, or sequence mismatch",
            ));
        }
        let data = BASE64
            .decode(&fragment.data_base64)
            .map_err(|_| display_error("invalid display bytes"))?;
        if data.len() > FRAGMENT_BYTES
            || self.partial.len() + data.len() > MESSAGE_BYTES
            || (!fragment.final_fragment && data.is_empty())
            || self.partial_kind.is_some_and(|kind| kind != fragment.kind)
        {
            return Err(display_error(
                "invalid or oversized display message fragments",
            ));
        }
        self.receive_sequence = self
            .receive_sequence
            .checked_add(1)
            .ok_or_else(|| display_error("display sequence exhausted"))?;
        self.partial_kind = Some(fragment.kind);
        self.partial.extend_from_slice(&data);
        if !fragment.final_fragment {
            return Ok(None);
        }
        let data = std::mem::take(&mut self.partial);
        self.partial_kind = None;
        if fragment.kind == DisplayMessageKind::Text && std::str::from_utf8(&data).is_err() {
            return Err(display_error("display text must be UTF-8"));
        }
        Ok(Some(DisplayMessage {
            kind: fragment.kind,
            data,
        }))
    }
}

fn display_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "encrypted display stream",
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests;
