//! V-RUN-04: arbitrary bytes on the worker IPC wire. The decoder must refuse
//! or accept without panicking, and an accepted message must survive
//! re-encoding unchanged whenever the writer would send it.
#![no_main]

use chariox_app_runtime::wire::{decode, Sender, WireError, MAX_FRAME_BYTES};
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

/// The seeds (`seed.mjs`) come from these vectors, so use their generation.
fn generation() -> &'static str {
    static GENERATION: OnceLock<String> = OnceLock::new();
    GENERATION.get_or_init(|| {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../../app-sdk/test/wire-vectors.json"))
                .expect("wire vectors parse");
        vectors["generation"]
            .as_str()
            .expect("vectors carry a generation")
            .to_owned()
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, frame)) = data.split_first() else {
        return;
    };
    let sender = if selector & 1 == 0 {
        Sender::Worker
    } else {
        Sender::Supervisor
    };
    if let Ok(message) = decode(frame, generation(), sender) {
        let encoded = serde_json::to_vec(&message).expect("an accepted message encodes");
        // Re-encoding can grow a frame (integral exponents are spelled out);
        // the writer refuses one over the limit, and so does the decoder.
        let again = decode(&encoded, generation(), sender);
        if encoded.len() <= MAX_FRAME_BYTES {
            assert_eq!(again.expect("a re-encoded message decodes"), message);
        } else {
            assert!(matches!(again, Err(WireError::FrameLimit)));
        }
    }
});
