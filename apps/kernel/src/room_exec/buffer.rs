use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use base64::Engine as _;

use super::{error, ExecChunk, ExecOutcome, ExecOutput, ExecStream, CHUNK_BYTES};
use crate::error::DaemonError;

/// One ceiling for unread output from every Room, including commands whose readers left.
pub(super) const OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub(super) const PIPE_READ_BYTES: usize = 32 * 1024;
const DROPPED: &[u8] = b"[Older unread command output was dropped because the kernel's shared output buffer filled. Read output promptly and acknowledge consumed cursors; unread records free themselves after their reader lease expires (300 seconds for commands, 600 for jobs).]\n";

#[derive(Default)]
pub(super) struct Pool {
    chunks: BTreeMap<u64, Vec<u8>>,
    bytes: usize,
    next_id: u64,
}

impl Pool {
    fn insert(&mut self, bytes: &[u8]) -> u64 {
        self.next_id = self.next_id.checked_add(1).expect("output sequence");
        let id = self.next_id;
        self.chunks.insert(id, bytes.to_vec());
        self.bytes += bytes.len();
        while self.bytes > OUTPUT_BYTES {
            let (_, dropped) = self.chunks.pop_first().expect("retained output");
            self.bytes -= dropped.len();
        }
        id
    }

    fn remove(&mut self, id: u64) {
        if let Some(bytes) = self.chunks.remove(&id) {
            self.bytes -= bytes.len();
        }
    }
}

struct Chunk {
    cursor: u64,
    stream: ExecStream,
    id: u64,
}

pub(super) struct Output {
    pool: Arc<Mutex<Pool>>,
    chunks: VecDeque<Chunk>,
    cursor: u64,
    acknowledged: u64,
    issued: u64,
    dropped_through: u64,
    pub(super) outcome: Option<ExecOutcome>,
}

impl Default for Output {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let mut pool = self.pool.lock().expect("shared exec output poisoned");
        for chunk in &self.chunks {
            pool.remove(chunk.id);
        }
    }
}

impl Output {
    pub(super) fn new(pool: Arc<Mutex<Pool>>) -> Self {
        Self {
            pool,
            chunks: VecDeque::new(),
            cursor: 0,
            acknowledged: 0,
            issued: 0,
            dropped_through: 0,
            outcome: None,
        }
    }

    fn discard_evicted(&mut self, pool: &Pool) {
        while self
            .chunks
            .front()
            .is_some_and(|chunk| !pool.chunks.contains_key(&chunk.id))
        {
            let chunk = self.chunks.pop_front().expect("front present");
            self.dropped_through = chunk.cursor;
        }
    }

    pub(super) fn push(&mut self, stream: ExecStream, bytes: &[u8]) {
        let shared = self.pool.clone();
        let mut pool = shared.lock().expect("shared exec output poisoned");
        let id = pool.insert(bytes);
        self.cursor += 1;
        self.chunks.push_back(Chunk {
            cursor: self.cursor,
            stream,
            id,
        });
        self.discard_evicted(&pool);
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
        let shared = self.pool.clone();
        let mut pool = shared.lock().expect("shared exec output poisoned");
        while self
            .chunks
            .front()
            .is_some_and(|chunk| chunk.cursor <= after)
        {
            let chunk = self.chunks.pop_front().expect("front present");
            pool.remove(chunk.id);
        }
        self.acknowledged = after;
        self.discard_evicted(&pool);
        let mut bytes = 0;
        let mut chunks = Vec::new();
        if self.dropped_through > after {
            chunks.push(ExecChunk {
                cursor: self.dropped_through,
                stream: ExecStream::Stderr,
                data_base64: base64::engine::general_purpose::STANDARD.encode(DROPPED),
            });
            bytes += DROPPED.len();
        }
        for chunk in &self.chunks {
            let data = pool.chunks.get(&chunk.id).expect("retained chunk");
            if bytes + data.len() > CHUNK_BYTES {
                break;
            }
            bytes += data.len();
            chunks.push(ExecChunk {
                cursor: chunk.cursor,
                stream: chunk.stream,
                data_base64: base64::engine::general_purpose::STANDARD.encode(data),
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

    pub(super) fn retained_bytes(&self) -> usize {
        let pool = self.pool.lock().expect("shared exec output poisoned");
        self.chunks
            .iter()
            .filter_map(|chunk| pool.chunks.get(&chunk.id))
            .map(Vec::len)
            .sum()
    }
}
