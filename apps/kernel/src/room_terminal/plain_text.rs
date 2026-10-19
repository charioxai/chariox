//! Output with terminal control sequences removed, for "output contains" watches.

const RETAINED_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sequence {
    None,
    Escape,
    Csi,
    Osc,
    OscEscape,
    /// `ESC (`, `ESC )` and similar: one more byte belongs to the sequence.
    Charset,
}

pub(super) struct PlainText {
    sequence: Sequence,
    carry: Vec<u8>,
    text: String,
    total_bytes: u64,
}

impl PlainText {
    pub(super) fn new() -> Self {
        Self {
            sequence: Sequence::None,
            carry: Vec::new(),
            text: String::new(),
            total_bytes: 0,
        }
    }

    pub(super) fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.advance_sequence(byte) {
                continue;
            }
            if byte == b'\n' || byte == b'\t' || byte >= 0x20 && byte != 0x7f {
                self.carry.push(byte);
            }
        }
        let decoded = self.decode_carry();
        self.total_bytes += decoded.len() as u64;
        self.text.push_str(&decoded);
        if self.text.len() > RETAINED_BYTES {
            let mut cut = self.text.len() - RETAINED_BYTES;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }

    /// Decode what is complete. An invalid sequence becomes U+FFFD so it never
    /// blocks later output; only an incomplete sequence at the end (at most
    /// three bytes) waits for its next bytes.
    fn decode_carry(&mut self) -> String {
        let mut decoded = String::new();
        loop {
            match std::str::from_utf8(&self.carry) {
                Ok(text) => {
                    decoded.push_str(text);
                    self.carry.clear();
                    return decoded;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    decoded
                        .push_str(std::str::from_utf8(&self.carry[..valid]).expect("valid prefix"));
                    match error.error_len() {
                        Some(invalid) => {
                            decoded.push('\u{fffd}');
                            self.carry.drain(..valid + invalid);
                        }
                        None => {
                            self.carry.drain(..valid);
                            return decoded;
                        }
                    }
                }
            }
        }
    }

    /// The text produced after `armed_at_bytes` (a prior `total_bytes`).
    pub(super) fn since(&self, armed_at_bytes: u64) -> &str {
        let new_bytes = (self.total_bytes - armed_at_bytes) as usize;
        let mut start = self.text.len().saturating_sub(new_bytes);
        while !self.text.is_char_boundary(start) {
            start += 1;
        }
        &self.text[start..]
    }

    /// Whether `byte` belongs to a control sequence.
    fn advance_sequence(&mut self, byte: u8) -> bool {
        self.sequence = match (self.sequence, byte) {
            (Sequence::None, 0x1b) => Sequence::Escape,
            (Sequence::None, _) => return false,
            (Sequence::Escape, b'[') => Sequence::Csi,
            (Sequence::Escape, b']') => Sequence::Osc,
            (Sequence::Escape, b'(' | b')' | b'*' | b'+' | b'#') => Sequence::Charset,
            (Sequence::Escape, _) | (Sequence::Charset, _) => Sequence::None,
            (Sequence::Csi, 0x40..=0x7e) => Sequence::None,
            (Sequence::Csi, _) => Sequence::Csi,
            (Sequence::Osc, 0x07) => Sequence::None,
            (Sequence::Osc, 0x1b) => Sequence::OscEscape,
            (Sequence::Osc, _) => Sequence::Osc,
            (Sequence::OscEscape, b'\\') => Sequence::None,
            (Sequence::OscEscape, _) => Sequence::Osc,
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sequences_and_keeps_split_utf8() {
        let mut plain = PlainText::new();
        plain.append(b"\x1b[1;32mhel");
        plain.append(b"lo\x1b[0m \x1b]0;title\x07w\xc3");
        plain.append(b"\xb6rld\r\n");
        assert_eq!(plain.since(0), "hello wörld\n");
    }

    #[test]
    fn invalid_utf8_is_replaced_and_does_not_block_later_output() {
        let mut plain = PlainText::new();
        plain.append(b"a\xffb");
        plain.append(b"still matches");
        assert_eq!(plain.since(0), "a\u{fffd}bstill matches");
        assert!(plain.carry.is_empty());
        plain.append(b"\xe2\x82");
        assert_eq!(plain.carry.len(), 2);
    }

    #[test]
    fn since_returns_only_text_after_arming() {
        let mut plain = PlainText::new();
        plain.append(b"old text ");
        let armed = plain.total_bytes();
        plain.append(b"new text");
        assert_eq!(plain.since(armed), "new text");
    }
}
