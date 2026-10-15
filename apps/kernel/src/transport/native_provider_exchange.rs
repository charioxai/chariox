//! MP-08 / MP-10 / MP-11: one acknowledged, byte-budgeted native exchange receipt.
use tokio::sync::mpsc;

use super::{native_provider_buffer::NativeBufferedFrame, native_provider_socket::error};
use crate::{error::DaemonError, local::NativeProviderTransportResult};

struct Receipt {
    sequence: u64,
    input: Vec<String>,
    output: Vec<NativeBufferedFrame>,
    closed: bool,
}

pub(super) struct NativeExchangeState {
    receiver: mpsc::Receiver<NativeBufferedFrame>,
    last_sequence: u64,
    receipt: Option<Receipt>,
}

impl NativeExchangeState {
    pub(super) fn new(receiver: mpsc::Receiver<NativeBufferedFrame>) -> Self {
        Self {
            receiver,
            last_sequence: 0,
            receipt: None,
        }
    }

    pub(super) fn begin(
        &mut self,
        connection_id: &str,
        sequence: u64,
        input: &[String],
    ) -> Result<Option<NativeProviderTransportResult>, DaemonError> {
        if sequence == 0 || sequence > 9_007_199_254_740_991 {
            return Err(error(
                "native exchange sequence is outside the supported range",
            ));
        }
        if sequence == self.last_sequence {
            let receipt = self
                .receipt
                .as_ref()
                .ok_or_else(|| error("native exchange receipt was already acknowledged"))?;
            if receipt.input != input {
                return Err(error(
                    "native exchange sequence was reused with different input",
                ));
            }
            return Ok(Some(receipt.result(connection_id)));
        }
        if sequence != self.last_sequence + 1 {
            return Err(error(
                "native exchange sequence is not the next acknowledged operation",
            ));
        }
        // The next sequence acknowledges the previous reply. Keep its byte
        // permits until this point, including while a relay reply is lost.
        self.receipt = None;
        Ok(None)
    }

    pub(super) fn take_frame(&mut self) -> Option<NativeBufferedFrame> {
        self.receiver.try_recv().ok()
    }

    pub(super) fn complete(
        &mut self,
        connection_id: &str,
        sequence: u64,
        input: Vec<String>,
        output: Vec<NativeBufferedFrame>,
    ) -> NativeProviderTransportResult {
        let receipt = Receipt {
            sequence,
            input,
            output,
            closed: self.receiver.is_closed() && self.receiver.is_empty(),
        };
        let result = receipt.result(connection_id);
        self.last_sequence = sequence;
        self.receipt = Some(receipt);
        result
    }
}

impl Receipt {
    fn result(&self, connection_id: &str) -> NativeProviderTransportResult {
        NativeProviderTransportResult {
            connection_id: connection_id.into(),
            sequence: self.sequence,
            frames: self.output.iter().map(|frame| frame.data.clone()).collect(),
            closed: self.closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::native_provider_buffer::NativeOutputBudget;

    #[test]
    fn lost_reply_replays_exact_frame_and_never_drains_the_next_one() {
        let budget = NativeOutputBudget::default().connection();
        let (sender, receiver) = mpsc::channel(4);
        let mut state = NativeExchangeState::new(receiver);
        assert!(state
            .begin("connection", 1, &["initialize".into()])
            .unwrap()
            .is_none());
        let first = state.complete(
            "connection",
            1,
            vec!["initialize".into()],
            vec![budget.reserve("private-first-frame".into()).unwrap()],
        );
        assert!(sender
            .try_send(budget.reserve("private-next-frame".into()).unwrap())
            .is_ok());
        let replay = state
            .begin("connection", 1, &["initialize".into()])
            .unwrap()
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(state.take_frame().unwrap().data, "private-next-frame");
        assert!(state
            .begin("connection", 1, &["different-input".into()])
            .is_err());
    }

    #[test]
    fn next_sequence_acknowledges_receipt_and_old_or_skipped_sequences_fail() {
        let (_, receiver) = mpsc::channel(1);
        let mut state = NativeExchangeState::new(receiver);
        assert!(state.begin("connection", 0, &[]).is_err());
        assert!(state.begin("connection", 2, &[]).is_err());
        assert!(state.begin("connection", 1, &[]).unwrap().is_none());
        state.complete("connection", 1, vec![], vec![]);
        assert!(state.begin("connection", 2, &[]).unwrap().is_none());
        assert!(state.begin("connection", 1, &[]).is_err());
        state.complete("connection", 2, vec![], vec![]);
        assert!(state.begin("connection", 1, &[]).is_err());
        assert!(state.begin("connection", 4, &[]).is_err());
    }

    #[test]
    fn upstream_close_does_not_skip_buffered_frames() {
        let budget = NativeOutputBudget::default().connection();
        let (sender, receiver) = mpsc::channel(2);
        assert!(sender
            .try_send(budget.reserve("first".into()).unwrap())
            .is_ok());
        assert!(sender
            .try_send(budget.reserve("last".into()).unwrap())
            .is_ok());
        drop(sender);
        let mut state = NativeExchangeState::new(receiver);
        state.begin("connection", 1, &[]).unwrap();
        let frame = state.take_frame().unwrap();
        let first = state.complete("connection", 1, vec![], vec![frame]);
        assert!(!first.closed);
        assert_eq!(first.frames, ["first"]);
        state.begin("connection", 2, &[]).unwrap();
        let frame = state.take_frame().unwrap();
        let last = state.complete("connection", 2, vec![], vec![frame]);
        assert!(last.closed);
        assert_eq!(last.frames, ["last"]);
    }

    #[test]
    fn retained_receipt_keeps_its_byte_permits_until_acknowledged() {
        let budget = NativeOutputBudget::default().connection();
        let (_, receiver) = mpsc::channel(1);
        let mut state = NativeExchangeState::new(receiver);
        state.begin("connection", 1, &[]).unwrap();
        state.complete(
            "connection",
            1,
            vec![],
            vec![budget.reserve("a".repeat(16 * 1024 * 1024)).unwrap()],
        );
        let queued = budget.reserve("b".repeat(16 * 1024 * 1024)).unwrap();
        assert!(budget.reserve("x".into()).is_err());
        state.begin("connection", 2, &[]).unwrap();
        assert!(budget.reserve("x".into()).is_ok());
        drop(queued);
    }
}
