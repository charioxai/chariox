//! Store only canonical Base64 ciphertext as bytes inside the relay queue.
//! Chunks reconstruct the original serialized reply without a second full copy.
use std::mem::size_of;

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::frame_transport::{
    binary_chunk, binary_chunk_payload, peer_reply_block_size, peer_reply_chunk,
    peer_reply_ciphertext_chunk, BINARY_CHUNK_PAYLOAD_BYTES,
};
use tokio_tungstenite::tungstenite::Message;

pub(crate) struct PackedReplyPlan<'a> {
    envelope: &'a str,
    start: usize,
    encoded: &'a str,
    decoded_len: usize,
}

type CiphertextBlock = Option<Box<[u8]>>;

pub(crate) struct PackedPeerReply {
    envelope: Box<str>,
    start: usize,
    end: usize,
    blocks: Box<[CiphertextBlock]>,
    block_index: usize,
    block_offset: usize,
    charge: usize,
}

fn block_encoded_len() -> usize {
    peer_reply_block_size() / 4 * 4
}

pub(crate) fn decoded_len_lower_bound(len: usize) -> usize {
    (len / 4 * 3).saturating_sub(2)
}

fn canonical_decoded_len(encoded: &[u8]) -> Option<usize> {
    if encoded.is_empty() {
        return Some(0);
    }
    if !encoded.len().is_multiple_of(4) {
        return None;
    }
    let padding = if encoded.ends_with(b"==") {
        2
    } else {
        usize::from(encoded.ends_with(b"="))
    };
    let data = &encoded[..encoded.len() - padding];
    if !data
        .iter()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/'))
    {
        return None;
    }
    // If the unused bits are set, decoding would change the spelling on replay.
    let last = *data.last()?;
    let clean = match padding {
        2 => b"AQgw".contains(&last),
        1 => b"AEIMQUYcgkosw048".contains(&last),
        _ => true,
    };
    clean.then_some(encoded.len() / 4 * 3 - padding)
}

impl<'a> PackedReplyPlan<'a> {
    pub(crate) fn new(envelope: &'a str, encoded: &'a str) -> Option<Self> {
        let field = r#""ciphertext":"""#;
        let (prefix, _) = envelope.split_once(field)?;
        let start = prefix.len() + field.len() - 1;
        Some(Self {
            envelope,
            start,
            encoded,
            decoded_len: canonical_decoded_len(encoded.as_bytes())?,
        })
    }

    pub(crate) fn charge(&self) -> usize {
        self.envelope.len()
            + self.decoded_len
            + self.encoded.len().div_ceil(block_encoded_len()) * size_of::<CiphertextBlock>()
    }

    pub(crate) fn into_reply(self) -> PackedPeerReply {
        let charge = self.charge();
        let blocks = self
            .encoded
            .as_bytes()
            .chunks(block_encoded_len())
            .map(|part| {
                let mut block = vec![0; part.len() / 4 * 3];
                let size = STANDARD
                    .decode_slice(part, &mut block)
                    .expect("canonical ciphertext fits its decoded block");
                block.truncate(size);
                Some(block.into_boxed_slice())
            })
            .collect();
        PackedPeerReply {
            envelope: self.envelope.into(),
            start: self.start,
            end: self.start + self.encoded.len(),
            blocks,
            block_index: 0,
            block_offset: 0,
            charge,
        }
    }
}

impl PackedPeerReply {
    pub(crate) fn len(&self) -> usize {
        self.envelope.len() + self.end - self.start
    }

    pub(crate) fn charge(&self) -> usize {
        self.charge
    }

    pub(crate) fn next_chunk(
        &self,
        id: u32,
        offset: usize,
        limit: usize,
    ) -> Option<(String, usize)> {
        if offset < self.start {
            return peer_reply_chunk(
                id,
                &self.envelope[offset..self.start],
                offset,
                self.len(),
                limit,
            );
        }
        if offset >= self.end {
            let tail = self.start + offset - self.end;
            return peer_reply_chunk(id, self.envelope.get(tail..)?, offset, self.len(), limit);
        }
        let block = self.blocks.get(self.block_index)?.as_deref()?;
        let in_block = (offset - self.start - self.block_offset) / 4 * 3;
        peer_reply_ciphertext_chunk(id, &block[in_block..], offset, self.len(), limit)
    }

    pub(crate) fn written(&mut self, end: usize) -> Option<usize> {
        let block = self.blocks.get_mut(self.block_index)?;
        let block_len = block.as_ref()?.len();
        let next = self.block_offset + base64::encoded_len(block_len, true)?;
        if end != self.start + next {
            return None;
        }
        drop(block.take());
        self.block_index += 1;
        self.block_offset = next;
        Some(block_len)
    }

    #[cfg(test)]
    pub(crate) fn into_test_text(mut self) -> String {
        let mut text = String::with_capacity(self.len());
        let mut offset = 0;
        while let Some((frame, end)) =
            self.next_chunk(1, offset, crate::frame_transport::CHUNK_BYTES)
        {
            let crate::frame_transport::TransportFrame::TransportChunk { payload, .. } =
                serde_json::from_str(&frame).expect("valid packed transport chunk")
            else {
                unreachable!("packed replies are text chunks")
            };
            text.push_str(&payload);
            self.written(end);
            offset = end;
        }
        text
    }
}

/// A peer reply as one binary message (`CXR2` header, then the ciphertext as bytes) kept as
/// independently owned blocks, each exactly one full binary chunk, so a written chunk frees
/// its block and the reply is never held twice.
pub(crate) struct BinaryPeerReply {
    head: Box<[u8]>,
    blocks: Box<[CiphertextBlock]>,
    len: usize,
    block_index: usize,
    charge: usize,
}

impl BinaryPeerReply {
    /// `head` is the encoded header with no ciphertext yet; `encoded` is canonical base64.
    pub(crate) fn new(head: Vec<u8>, encoded: &str) -> Option<Self> {
        let decoded_len = canonical_decoded_len(encoded.as_bytes())?;
        let block_encoded = BINARY_CHUNK_PAYLOAD_BYTES / 3 * 4;
        let block_raw = block_encoded / 4 * 3;
        let blocks: Box<[CiphertextBlock]> = encoded
            .as_bytes()
            .chunks(block_encoded)
            .map(|part| {
                let mut block = vec![0; part.len() / 4 * 3];
                let size = STANDARD
                    .decode_slice(part, &mut block)
                    .expect("canonical ciphertext fits its decoded block");
                block.truncate(size);
                Some(block.into_boxed_slice())
            })
            .collect();
        debug_assert!(blocks
            .iter()
            .rev()
            .skip(1)
            .all(|block| block.as_ref().is_some_and(|b| b.len() == block_raw)));
        let len = head.len() + decoded_len;
        Some(Self {
            charge: len + blocks.len() * size_of::<CiphertextBlock>(),
            head: head.into_boxed_slice(),
            blocks,
            len,
            block_index: 0,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn charge(&self) -> usize {
        self.charge
    }

    fn block_raw() -> usize {
        BINARY_CHUNK_PAYLOAD_BYTES / 3 * 4 / 4 * 3
    }

    pub(crate) fn next_chunk(
        &self,
        id: u32,
        offset: usize,
        limit: usize,
        adaptive: bool,
    ) -> Option<(Message, usize)> {
        let room = binary_chunk_payload(limit, adaptive);
        let payload: &[u8] = if offset < self.head.len() {
            let end = (offset + room).min(self.head.len());
            &self.head[offset..end]
        } else {
            let in_blocks = offset - self.head.len();
            let index = in_blocks / Self::block_raw();
            let block = self.blocks.get(index)?.as_deref()?;
            let from = in_blocks - index * Self::block_raw();
            &block[from..(from + room).min(block.len())]
        };
        if payload.is_empty() {
            return None;
        }
        let end = offset + payload.len();
        Some((
            Message::Binary(binary_chunk(id, offset, self.len, payload).into()),
            end,
        ))
    }

    pub(crate) fn written(&mut self, end: usize) -> Option<usize> {
        let block = self.blocks.get_mut(self.block_index)?;
        let block_len = block.as_ref()?.len();
        let block_end = self.head.len() + self.block_index * Self::block_raw() + block_len;
        if end < block_end {
            return None;
        }
        drop(block.take());
        self.block_index += 1;
        Some(block_len)
    }

    #[cfg(test)]
    pub(crate) fn into_test_bytes(mut self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.len);
        let mut offset = 0;
        while let Some((Message::Binary(frame), end)) =
            self.next_chunk(1, offset, crate::frame_transport::CHUNK_BYTES, false)
        {
            bytes.extend_from_slice(&frame[crate::frame_transport::BINARY_CHUNK_HEADER_BYTES..]);
            self.written(end);
            offset = end;
        }
        bytes
    }
}
