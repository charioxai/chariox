//! What Excalidraw stamps on an element it creates or changes: a new version, a version nonce that
//! differs between versions, the time and, for a new element, a seed and an ID.

use serde_json::{json, Map, Value};

pub struct Stamp {
    state: u64,
    pub now_ms: u64,
}

const ID_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

impl Stamp {
    pub fn new(seed: u64, now_ms: u64) -> Self {
        Self {
            state: seed | 1,
            now_ms,
        }
    }

    /// xorshift64*: nothing here needs more than values that differ between versions and elements.
    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Excalidraw's `randomInteger`: up to 2^31.
    pub fn integer(&mut self) -> u32 {
        (self.next() >> 33) as u32
    }

    /// An element ID of 21 URL-safe characters, like Excalidraw's `randomId`.
    pub fn id(&mut self) -> String {
        (0..21)
            .map(|_| ID_ALPHABET[(self.next() >> 58) as usize] as char)
            .collect()
    }

    /// `element.version++` with a new nonce and time, as `mutateElement` does after a change.
    pub fn bump(&mut self, fields: &mut Map<String, Value>) {
        let version = fields.get("version").and_then(Value::as_u64).unwrap_or(0);
        fields.insert("version".into(), json!(version + 1));
        fields.insert("versionNonce".into(), json!(self.integer()));
        fields.insert("updated".into(), json!(self.now_ms));
    }
}
