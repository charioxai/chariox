//! MP-08 / MP-09 / MP-10 / MP-11: toward a relay that reassembles them, the
//! kernel sends messages above one chunk as chunks-v2 frames, on its own relay
//! connection and on the gateway's client links. A priority frame then waits
//! for at most one chunk instead of a whole message, and the chunks in flight
//! follow the drain rate the relay's receipts show.
use std::collections::VecDeque;

use chariox_relay::drain_window::{DrainWindow, SendStamp};
use chariox_relay::frame_transport::{
    binary_chunk, binary_chunk_payload, text_chunk, TransportFrame, CHUNK_BYTES,
};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lane {
    Priority,
    Event,
}

/// The relay's receipt for one of this connection's upload chunks.
pub(crate) fn receipt(payload: &str) -> Option<(u32, u32)> {
    if !payload.starts_with("{\"kind\":\"transport_ack\"") {
        return None;
    }
    match serde_json::from_str(payload).ok()? {
        TransportFrame::TransportAck {
            transfer_id,
            offset,
        } => Some((transfer_id, offset)),
        _ => None,
    }
}

/// One message being sent in chunks.
struct Upload {
    /// Assigned when the first chunk is written; the relay requires increasing ids.
    id: u32,
    message: Message,
    offset: usize,
}

impl Upload {
    /// A text frame stays within `limit`; so does a binary one toward a relay that takes chunks
    /// shorter than a full one (`adaptive`), otherwise it is always one full chunk.
    fn next_chunk(&self, id: u32, limit: usize, adaptive: bool) -> Option<(Message, usize)> {
        match &self.message {
            Message::Text(text) => text_chunk(id, text, self.offset, limit)
                .map(|(frame, end)| (Message::Text(frame.into()), end)),
            Message::Binary(bytes) if self.offset < bytes.len() => {
                let end = (self.offset + binary_chunk_payload(limit, adaptive)).min(bytes.len());
                let chunk = binary_chunk(id, self.offset, bytes.len(), &bytes[self.offset..end]);
                Some((Message::Binary(chunk.into()), end))
            }
            _ => None,
        }
    }
}

/// The message each lane is sending in chunks, and the chunks the relay has
/// not acknowledged yet, in wire order.
pub(crate) struct Uploads {
    priority: Option<Upload>,
    event: Option<Upload>,
    pending: VecDeque<(u32, u32, usize, SendStamp)>,
    bytes: usize,
    window: DrainWindow,
    last_id: u32,
    adaptive_binary: bool,
}

impl Uploads {
    /// `adaptive_binary`: the relay accepted binary requests, and with them binary chunks sized
    /// to the link like text ones.
    pub(crate) fn new(adaptive_binary: bool) -> Self {
        Self {
            priority: None,
            event: None,
            pending: VecDeque::new(),
            bytes: 0,
            window: DrainWindow::new(Instant::now()),
            last_id: 0,
            adaptive_binary,
        }
    }

    /// Small frames go out whole; a larger one is returned to the caller only
    /// when it was not taken for chunking.
    pub(crate) fn start(&mut self, lane: Lane, message: Message) -> Option<Message> {
        if message.len() <= CHUNK_BYTES || !matches!(message, Message::Text(_) | Message::Binary(_))
        {
            return Some(message);
        }
        let slot = self.slot(lane);
        debug_assert!(slot.is_none(), "a lane sends one message at a time");
        *slot = Some(Upload {
            id: 0,
            message,
            offset: 0,
        });
        None
    }

    /// True while the lane is still sending a message; it takes no other until then.
    pub(crate) fn busy(&self, lane: Lane) -> bool {
        match lane {
            Lane::Priority => self.priority.is_some(),
            Lane::Event => self.event.is_some(),
        }
    }

    /// True when the lane has a chunk the window has room for; one chunk always
    /// fits when nothing is unacknowledged.
    pub(crate) fn ready(&self, lane: Lane) -> bool {
        let upload = match lane {
            Lane::Priority => &self.priority,
            Lane::Event => &self.event,
        };
        let frame = match upload {
            Some(upload) if matches!(&upload.message, Message::Text(_)) || self.adaptive_binary => {
                self.window.chunk_bytes()
            }
            Some(_) => CHUNK_BYTES,
            None => return false,
        };
        self.bytes == 0 || self.bytes + frame <= self.window.bytes()
    }

    /// The lane's next chunk; the lane is free again after its last one.
    pub(crate) fn next_chunk(&mut self, lane: Lane) -> Option<Message> {
        let next_id = self.last_id.checked_add(1)?;
        let adaptive = self.adaptive_binary;
        let limit = self.window.chunk_bytes();
        let upload = self.slot(lane).as_mut()?;
        let id = if upload.id == 0 { next_id } else { upload.id };
        let (chunk, end) = upload.next_chunk(id, limit, adaptive)?;
        upload.id = id;
        upload.offset = end;
        if end == upload.message.len() {
            *self.slot(lane) = None;
        }
        self.last_id = self.last_id.max(id);
        let stamp = self.window.sent(Instant::now(), self.pending.is_empty());
        self.bytes += chunk.len();
        self.pending.push_back((id, end as u32, chunk.len(), stamp));
        Some(chunk)
    }

    /// The relay acknowledges chunks in the order they were written; anything
    /// else means the two sides disagree about the stream.
    pub(crate) fn receipt(&mut self, id: u32, offset: u32) -> bool {
        match self.pending.front() {
            Some(&(expected_id, expected_offset, bytes, stamp))
                if (expected_id, expected_offset) == (id, offset) =>
            {
                self.pending.pop_front();
                self.bytes -= bytes;
                self.window.received(Instant::now(), bytes, stamp, true);
                true
            }
            _ => false,
        }
    }

    fn slot(&mut self, lane: Lane) -> &mut Option<Upload> {
        match lane {
            Lane::Priority => &mut self.priority,
            Lane::Event => &mut self.event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::frame_transport::FrameReceiver;

    // The relay's own reassembly accepts what the kernel sends, text and binary,
    // with a small frame between chunks, and its receipts release the window.
    #[tokio::test]
    async fn the_relay_reassembles_chunked_uploads_and_their_receipts_free_the_window() {
        let text = format!(
            "{{\"kind\":\"daemon_response\",\"payload\":\"{}\"}}",
            "é".repeat(40_000)
        );
        let binary = vec![7u8; 100_000];
        let mut uploads = Uploads::new(false);
        assert!(uploads
            .start(Lane::Event, Message::Text(text.clone().into()))
            .is_none());
        assert!(uploads
            .start(Lane::Priority, Message::Binary(binary.clone().into()))
            .is_none());
        assert!(uploads
            .start(
                Lane::Priority,
                Message::Text("{\"kind\":\"daemon_heartbeat\"}".into())
            )
            .is_some());
        let mut relay = FrameReceiver::default();
        let mut complete = Vec::new();
        while uploads.busy(Lane::Priority) || uploads.busy(Lane::Event) {
            let lane = if uploads.ready(Lane::Priority) {
                Lane::Priority
            } else if uploads.ready(Lane::Event) {
                Lane::Event
            } else {
                panic!("the window stalled without unacknowledged chunks")
            };
            let chunk = uploads.next_chunk(lane).unwrap();
            assert!(chunk.len() <= CHUNK_BYTES);
            let (done, receipt) = match chunk {
                Message::Text(t) => {
                    let (d, r) = relay.receive(&t).unwrap();
                    (d.map(|d| d.into_bytes()), r)
                }
                Message::Binary(b) => relay.receive_binary(&b).unwrap(),
                _ => unreachable!(),
            };
            complete.extend(done);
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&receipt.unwrap()).unwrap()
            else {
                panic!("receipt expected")
            };
            assert!(uploads.receipt(transfer_id, offset));
        }
        assert_eq!(complete, vec![binary, text.into_bytes()]);
        assert!(!uploads.receipt(9, 0), "an unexpected receipt is refused");
    }
}
