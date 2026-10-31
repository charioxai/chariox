//! Bounded raw output with absolute byte offsets, so a reconnecting client
//! continues with no gap or learns that it must resync from a snapshot.

use std::collections::VecDeque;

const RETAINED_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct OutputLog {
    retained: VecDeque<u8>,
    start_offset: u64,
}

pub(super) struct OutputPage {
    pub bytes: Vec<u8>,
    pub next_offset: u64,
    pub gap: bool,
}

impl OutputLog {
    /// The retained output a hot reload hands over, at its original offsets.
    pub(super) fn restored(start_offset: u64, retained: Vec<u8>) -> Self {
        Self {
            retained: retained.into(),
            start_offset,
        }
    }

    pub(super) fn start_offset(&self) -> u64 {
        self.start_offset
    }

    pub(super) fn retained(&self) -> Vec<u8> {
        self.retained.iter().copied().collect()
    }

    pub(super) fn end_offset(&self) -> u64 {
        self.start_offset + self.retained.len() as u64
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        self.retained.extend(bytes);
        let excess = self.retained.len().saturating_sub(RETAINED_BYTES);
        if excess > 0 {
            self.retained.drain(..excess);
            self.start_offset += excess as u64;
        }
    }

    pub(super) fn read(&self, after: u64, max_bytes: usize) -> OutputPage {
        if after < self.start_offset || after > self.end_offset() {
            return OutputPage {
                bytes: Vec::new(),
                next_offset: self.end_offset(),
                gap: true,
            };
        }
        let skip = (after - self.start_offset) as usize;
        let bytes: Vec<u8> = self
            .retained
            .iter()
            .skip(skip)
            .take(max_bytes)
            .copied()
            .collect();
        OutputPage {
            next_offset: after + bytes.len() as u64,
            bytes,
            gap: false,
        }
    }
}
