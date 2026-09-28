//! V-RUN-04: arbitrary bytes on the worker IPC wire. The decoder must refuse
//! or accept without panicking, never accept an oversized frame, and an
//! accepted message must survive re-encoding unchanged.
#![no_main]

use chariox_app_runtime::wire::{decode, Sender, MAX_FRAME_BYTES};
use libfuzzer_sys::fuzz_target;

const GENERATION: &str = "generation-7"; // packages/app-sdk/test/wire-vectors.json

fuzz_target!(|data: &[u8]| {
    let Some((&selector, frame)) = data.split_first() else {
        return;
    };
    let sender = if selector & 1 == 0 { Sender::Worker } else { Sender::Supervisor };
    if let Ok(message) = decode(frame, GENERATION, sender) {
        assert!(frame.len() <= MAX_FRAME_BYTES);
        let encoded = serde_json::to_vec(&message).expect("an accepted message encodes");
        // Re-encoding can grow a frame (integral exponents are spelled out),
        // and the writer refuses one over the limit, so only a frame it would
        // send must decode again.
        if encoded.len() <= MAX_FRAME_BYTES {
            let again = decode(&encoded, GENERATION, sender).expect("a re-encoded message decodes");
            assert_eq!(again, message);
        }
    }
});
