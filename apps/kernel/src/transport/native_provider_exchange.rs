//! MP-08 / MP-10 / MP-11: one acknowledged, byte-budgeted native exchange receipt.
use base64::Engine as _;
use std::io::Write;
use tokio::sync::mpsc;

use super::{native_provider_buffer::NativeBufferedFrame, native_provider_socket::error};
use crate::{
    error::DaemonError,
    local::{NativeProviderTransportGzipFragment, NativeProviderTransportResult},
};

const PAGE_BYTES: usize = 64 * 1024;
const MAX_COMPRESSED_BYTES: usize = super::native_provider_buffer::OUTPUT_FRAME_BYTES + PAGE_BYTES;

struct PartialFrame {
    frame: NativeBufferedFrame,
    decoded_bytes: usize,
    compressed: Vec<u8>,
    offset: usize,
}

struct Receipt {
    sequence: u64,
    input: Vec<String>,
    output: Vec<NativeBufferedFrame>,
    closed: bool,
    gzip_fragment: Option<NativeProviderTransportGzipFragment>,
    // The last fragment's receipt retains the original frame byte permits.
    _completed_frame: Option<NativeBufferedFrame>,
}

pub(super) struct NativeExchangeState {
    receiver: mpsc::Receiver<NativeBufferedFrame>,
    last_sequence: u64,
    receipt: Option<Receipt>,
    partial: Option<PartialFrame>,
}

impl NativeExchangeState {
    pub(super) fn new(receiver: mpsc::Receiver<NativeBufferedFrame>) -> Self {
        Self {
            receiver,
            last_sequence: 0,
            receipt: None,
            partial: None,
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
        if self.partial.is_some() {
            None
        } else {
            self.receiver.try_recv().ok()
        }
    }

    pub(super) fn complete(
        &mut self,
        connection_id: &str,
        sequence: u64,
        input: Vec<String>,
        mut output: Vec<NativeBufferedFrame>,
    ) -> Result<NativeProviderTransportResult, DaemonError> {
        if self.partial.is_none() && output.len() == 1 && output[0].data.len() > PAGE_BYTES {
            let mut frame = output.pop().expect("single frame");
            let decoded_bytes = frame.data.len();
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder
                .write_all(frame.data.as_bytes())
                .map_err(|_| error("cannot encode native output"))?;
            let compressed = encoder
                .finish()
                .map_err(|_| error("cannot finish native output encoding"))?;
            if compressed.len() > MAX_COMPRESSED_BYTES {
                return Err(error("compressed native output exceeds its byte bound"));
            }
            // Retain decoded-byte permits, not a second copy of private JSON.
            frame.data = String::new();
            self.partial = Some(PartialFrame {
                frame,
                decoded_bytes,
                compressed,
                offset: 0,
            });
        }
        let mut completed_frame = None;
        let gzip_fragment = self.partial.take().map(|mut partial| {
            let end = (partial.offset + PAGE_BYTES).min(partial.compressed.len());
            let fragment = NativeProviderTransportGzipFragment {
                offset: partial.offset as u32,
                total_bytes: partial.compressed.len() as u32,
                decoded_bytes: partial.decoded_bytes as u32,
                data: base64::engine::general_purpose::STANDARD
                    .encode(&partial.compressed[partial.offset..end]),
            };
            partial.offset = end;
            if end == partial.compressed.len() {
                completed_frame = Some(partial.frame);
            } else {
                self.partial = Some(partial);
            }
            fragment
        });
        let receipt = Receipt {
            sequence,
            input,
            output,
            closed: self.receiver.is_closed() && self.receiver.is_empty() && self.partial.is_none(),
            gzip_fragment,
            _completed_frame: completed_frame,
        };
        let result = receipt.result(connection_id);
        self.last_sequence = sequence;
        self.receipt = Some(receipt);
        Ok(result)
    }
}

impl Receipt {
    fn result(&self, connection_id: &str) -> NativeProviderTransportResult {
        NativeProviderTransportResult {
            connection_id: connection_id.into(),
            sequence: self.sequence,
            frames: self.output.iter().map(|frame| frame.data.clone()).collect(),
            gzip_fragment: self.gzip_fragment.clone(),
            closed: self.closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::native_provider_buffer::NativeOutputBudget;

    #[test]
    fn mp08_large_native_reply_pages_replay_exactly_before_the_next_frame() {
        use rand::{RngCore, SeedableRng};
        use std::io::Read;
        let mut entropy = vec![0u8; 1024 * 1024];
        rand::rngs::StdRng::seed_from_u64(7).fill_bytes(&mut entropy);
        let text = serde_json::json!({"id":1,"result":{
            "private":"private-native-canary",
            "data":base64::engine::general_purpose::STANDARD.encode(entropy)
        }})
        .to_string();
        let budget = NativeOutputBudget::default().connection();
        let (sender, receiver) = mpsc::channel(2);
        assert!(sender
            .try_send(budget.reserve("next-native-frame".into()).unwrap())
            .is_ok());
        drop(sender);
        let mut state = NativeExchangeState::new(receiver);
        let mut compressed = Vec::new();
        let mut sequence = 1;
        state.begin("connection", sequence, &[]).unwrap();
        let mut output = vec![budget.reserve(text.clone()).unwrap()];
        loop {
            let result = state
                .complete("connection", sequence, vec![], output)
                .unwrap();
            assert!(serde_json::to_vec(&result).unwrap().len() < 128 * 1024);
            assert!(result.frames.is_empty());
            assert!(!result.closed, "the next upstream frame is still queued");
            let page = result.gzip_fragment.as_ref().unwrap();
            assert_eq!(page.offset as usize, compressed.len());
            assert_eq!(page.decoded_bytes as usize, text.len());
            compressed.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(&page.data)
                    .unwrap(),
            );
            assert_eq!(
                state.begin("connection", sequence, &[]).unwrap().unwrap(),
                result
            );
            sequence += 1;
            state.begin("connection", sequence, &[]).unwrap();
            if state.partial.is_none() {
                break;
            }
            assert!(
                state.take_frame().is_none(),
                "do not overtake the partial frame"
            );
            output = vec![];
        }
        assert!(sequence > 2, "exercise multiple retained pages");
        let mut decoded = String::new();
        flate2::read::GzDecoder::new(compressed.as_slice())
            .read_to_string(&mut decoded)
            .unwrap();
        assert_eq!(decoded, text);
        let next = state.take_frame().unwrap();
        let final_reply = state
            .complete("connection", sequence, vec![], vec![next])
            .unwrap();
        assert_eq!(final_reply.frames, ["next-native-frame"]);
        assert!(final_reply.closed);
    }

    #[test]
    fn lost_reply_replays_exact_frame_and_never_drains_the_next_one() {
        let budget = NativeOutputBudget::default().connection();
        let (sender, receiver) = mpsc::channel(4);
        let mut state = NativeExchangeState::new(receiver);
        assert!(state
            .begin("connection", 1, &["initialize".into()])
            .unwrap()
            .is_none());
        let first = state
            .complete(
                "connection",
                1,
                vec!["initialize".into()],
                vec![budget.reserve("private-first-frame".into()).unwrap()],
            )
            .unwrap();
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
        state.complete("connection", 1, vec![], vec![]).unwrap();
        assert!(state.begin("connection", 2, &[]).unwrap().is_none());
        assert!(state.begin("connection", 1, &[]).is_err());
        state.complete("connection", 2, vec![], vec![]).unwrap();
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
        let first = state
            .complete("connection", 1, vec![], vec![frame])
            .unwrap();
        assert!(!first.closed);
        assert_eq!(first.frames, ["first"]);
        state.begin("connection", 2, &[]).unwrap();
        let frame = state.take_frame().unwrap();
        let last = state
            .complete("connection", 2, vec![], vec![frame])
            .unwrap();
        assert!(last.closed);
        assert_eq!(last.frames, ["last"]);
    }

    #[test]
    fn retained_receipt_keeps_its_byte_permits_until_acknowledged() {
        let budget = NativeOutputBudget::default().connection();
        let (_, receiver) = mpsc::channel(1);
        let mut state = NativeExchangeState::new(receiver);
        state.begin("connection", 1, &[]).unwrap();
        state
            .complete(
                "connection",
                1,
                vec![],
                vec![budget.reserve("a".repeat(16 * 1024 * 1024)).unwrap()],
            )
            .unwrap();
        let queued = budget.reserve("b".repeat(16 * 1024 * 1024)).unwrap();
        assert!(budget.reserve("x".into()).is_err());
        state.begin("connection", 2, &[]).unwrap();
        let mut sequence = 2;
        while state.partial.is_some() {
            state
                .complete("connection", sequence, vec![], vec![])
                .unwrap();
            sequence += 1;
            state.begin("connection", sequence, &[]).unwrap();
        }
        assert!(budget.reserve("x".into()).is_ok());
        drop(queued);
    }
}
