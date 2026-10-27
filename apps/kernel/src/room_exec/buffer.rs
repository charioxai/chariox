use std::collections::VecDeque;

use base64::Engine as _;

use super::{error, ExecChunk, ExecOutcome, ExecOutput, ExecStream, CHUNK_BYTES};
use crate::error::DaemonError;

pub(super) const OUTPUT_BYTES: usize = 1024 * 1024;
pub(super) const PIPE_READ_BYTES: usize = 32 * 1024;

struct Chunk {
    cursor: u64,
    stream: ExecStream,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Output {
    chunks: VecDeque<Chunk>,
    bytes: usize,
    cursor: u64,
    acknowledged: u64,
    issued: u64,
    pub(super) outcome: Option<ExecOutcome>,
}

impl Output {
    pub(super) fn push(&mut self, stream: ExecStream, bytes: &[u8]) -> bool {
        if self.bytes + bytes.len() > OUTPUT_BYTES || self.chunks.len() >= 128 {
            return false;
        }
        self.cursor += 1;
        self.bytes += bytes.len();
        self.chunks.push_back(Chunk {
            cursor: self.cursor,
            stream,
            bytes: bytes.to_vec(),
        });
        true
    }

    pub(super) fn page(&mut self, after: u64) -> Result<ExecOutput, DaemonError> {
        if after < self.acknowledged || after > self.issued {
            return Err(error(
                "exec_cursor_invalid",
                format!(
                    "output cursor must be between {} and {}; resume from the last consumed cursor",
                    self.acknowledged, self.issued
                ),
            ));
        }
        while self
            .chunks
            .front()
            .is_some_and(|chunk| chunk.cursor <= after)
        {
            let chunk = self.chunks.pop_front().expect("front present");
            self.bytes -= chunk.bytes.len();
        }
        self.acknowledged = after;
        let mut bytes = 0;
        let mut chunks = Vec::new();
        for chunk in &self.chunks {
            if bytes + chunk.bytes.len() > CHUNK_BYTES {
                break;
            }
            bytes += chunk.bytes.len();
            chunks.push(ExecChunk {
                cursor: chunk.cursor,
                stream: chunk.stream,
                data_base64: base64::engine::general_purpose::STANDARD.encode(&chunk.bytes),
            });
        }
        let next_cursor = chunks.last().map_or(after, |chunk| chunk.cursor);
        self.issued = self.issued.max(next_cursor);
        Ok(ExecOutput {
            chunks,
            next_cursor,
            outcome: (next_cursor == self.cursor)
                .then(|| self.outcome.clone())
                .flatten(),
        })
    }

    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes
    }
}
