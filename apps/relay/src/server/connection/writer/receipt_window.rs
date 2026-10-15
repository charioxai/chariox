//! MP-08 / MP-10 / MP-11: one bounded receipt window across sequential envelopes.
use crate::frame_transport::{CHUNK_BYTES, WINDOW_BYTES};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct ReceiptWindow {
    outstanding: VecDeque<(u32, u32, usize, usize)>,
    wire_bytes: usize,
    small_sent: usize,
    small_acked: usize,
}

impl ReceiptWindow {
    pub(super) fn is_empty(&self) -> bool {
        self.outstanding.is_empty()
    }
    pub(super) fn available_wire_bytes(&self) -> usize {
        WINDOW_BYTES - self.wire_bytes
    }
    pub(super) fn can_send_small(&self, bytes: usize) -> bool {
        self.small_sent - self.small_acked + bytes <= CHUNK_BYTES
    }
    pub(super) fn record_small(&mut self, bytes: usize) {
        self.small_sent += bytes;
    }
    pub(super) fn reset_small(&mut self) {
        debug_assert!(self.is_empty());
        self.small_sent = 0;
        self.small_acked = 0;
    }
    pub(super) fn record_chunk(&mut self, id: u32, offset: u32, bytes: usize) {
        debug_assert!(bytes <= self.available_wire_bytes());
        self.outstanding
            .push_back((id, offset, bytes, self.small_sent));
        self.wire_bytes += bytes;
    }
    pub(super) fn receipt(&mut self, id: u32, offset: u32) -> bool {
        if self.outstanding.front().map(|r| (r.0, r.1)) != Some((id, offset)) {
            return false;
        }
        let (_, _, bytes, small_acked) = self.outstanding.pop_front().unwrap();
        self.wire_bytes -= bytes;
        // A receipt proves only small traffic preceding that exact ordered chunk.
        self.small_acked = small_acked;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_transfers_keep_the_global_wire_window_charged() {
        let mut window = ReceiptWindow::default();
        window.record_chunk(1, 16000, CHUNK_BYTES);
        window.record_chunk(1, 32000, CHUNK_BYTES);
        window.record_chunk(2, 16000, CHUNK_BYTES);
        window.record_chunk(2, 32000, CHUNK_BYTES);
        assert_eq!(window.available_wire_bytes(), 0);
        assert!(!window.receipt(2, 16000));
        assert_eq!(window.available_wire_bytes(), 0);
        assert!(window.receipt(1, 16000));
        assert_eq!(window.available_wire_bytes(), CHUNK_BYTES);
    }
    #[test]
    fn credits_follow_exact_ordered_receipts_across_transfer_boundaries() {
        let mut window = ReceiptWindow::default();
        window.record_chunk(1, 4, 1000);
        window.record_small(CHUNK_BYTES);
        window.record_chunk(1, 8, 1000);
        window.record_chunk(2, 4, 1000);
        assert!(
            !window.receipt(2, 4),
            "a later transfer cannot forge earlier credit"
        );
        assert!(!window.receipt(1, 5), "offset must match the sent frame");
        assert!(window.receipt(1, 4));
        assert!(
            !window.can_send_small(1),
            "old chunks cannot acknowledge later small frames"
        );
        assert!(
            !window.receipt(1, 4),
            "duplicate receipt cannot release credit twice"
        );
        assert!(window.receipt(1, 8));
        assert!(window.can_send_small(CHUNK_BYTES));
        assert_eq!(window.available_wire_bytes(), WINDOW_BYTES - 1000);
        assert!(window.receipt(2, 4));
        assert!(window.is_empty());
        assert_eq!(window.available_wire_bytes(), WINDOW_BYTES);
    }
}
