//! V-RUN-04: malformed or oversized worker IPC. Deterministic mutations of the
//! shared wire corpus must never panic the decoder; limits hold exactly.
use chariox_app_runtime::wire::{decode, Sender, WireError, MAX_FRAME_BYTES};
use serde_json::Value;

const CORPUS: &str = include_str!("../../app-sdk/test/wire-vectors.json");

/// xorshift64*: reproducible without a fuzzing dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn mutate(rng: &mut Rng, seed: &[u8], others: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..=rng.below(4) {
        if bytes.is_empty() {
            bytes.push(b'{');
        }
        let at = rng.below(bytes.len());
        match rng.below(6) {
            0 => bytes[at] ^= 1 << rng.below(8),
            1 => bytes.truncate(at),
            2 => bytes.insert(at, rng.next() as u8),
            3 => {
                let other = &others[rng.below(others.len())];
                let from = rng.below(other.len());
                bytes.splice(at..at, other[from..].iter().copied());
            }
            4 => {
                bytes.splice(at..at, b"[[[[[[[[[[[[[[[[".iter().copied());
            }
            _ => {
                bytes.splice(at..at, b"1e999999".iter().copied());
            }
        }
    }
    bytes
}

#[test]
fn mutated_frames_are_decoded_or_refused_never_a_panic() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap();
    let generation = corpus["generation"].as_str().unwrap();
    let seeds: Vec<Vec<u8>> = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["valid"] == true)
        .map(|case| serde_json::to_vec(&case["message"]).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let mut rng = Rng(0x5eed_c0de_1234_5678);
    let (mut accepted, mut refused) = (0, 0);
    for round in 0..20_000 {
        let frame = mutate(&mut rng, &seeds[round % seeds.len()], &seeds);
        let sender = if round % 2 == 0 {
            Sender::Worker
        } else {
            Sender::Supervisor
        };
        match decode(&frame, generation, sender) {
            Ok(_) => accepted += 1,
            Err(_) => refused += 1,
        }
    }
    // Mutations mostly break a frame; a few are still valid messages.
    assert!(refused > accepted, "{accepted} accepted, {refused} refused");
}

#[test]
fn frame_and_nesting_limits_are_exact() {
    let generation = "g1";
    let oversized = vec![b' '; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        decode(&oversized, generation, Sender::Worker),
        Err(WireError::FrameLimit)
    ));
    assert!(matches!(
        decode(b"", generation, Sender::Worker),
        Err(WireError::FrameLimit)
    ));
    // Far deeper than the decoder allows: refused without exhausting the stack.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(decode(deep.as_bytes(), generation, Sender::Worker).is_err());
    let nested = format!(
        r#"{{"kind":"event","version":1,"generation":"g1","name":"x","data":{}0{}}}"#,
        "[".repeat(70),
        "]".repeat(70)
    );
    assert!(decode(nested.as_bytes(), generation, Sender::Worker).is_err());
}
