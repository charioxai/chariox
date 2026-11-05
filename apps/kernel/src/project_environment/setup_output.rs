//! Bounded Setup output decoding; incomplete UTF-8 and Secret matches wait for the next read.
use zeroize::Zeroizing;

pub(super) const READ_BYTES: usize = 4096;

/// Only a possible Secret prefix or an incomplete UTF-8 character survives a read.
#[derive(Default)]
pub(super) struct SetupOutput {
    pending: Zeroizing<Vec<u8>>,
}

impl SetupOutput {
    pub(super) fn push(
        &mut self,
        bytes: &[u8],
        secrets: &[(String, Zeroizing<String>)],
        eof: bool,
    ) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            let remaining = &self.pending[offset..];
            if !eof
                && secrets.iter().any(|(_, value)| {
                    remaining.len() < value.len() && value.as_bytes().starts_with(remaining)
                })
            {
                break;
            }
            if let Some((name, value)) = secrets
                .iter()
                .filter(|(_, value)| !value.is_empty() && remaining.starts_with(value.as_bytes()))
                .max_by_key(|(_, value)| value.len())
            {
                output.push('[');
                output.push_str(name);
                output.push(']');
                offset += value.len();
                continue;
            }
            let width = match remaining[0] {
                0..=0x7f => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            };
            match std::str::from_utf8(&remaining[..width.min(remaining.len())]) {
                Ok(text) => {
                    output.push_str(text);
                    offset += text.len();
                }
                Err(error) => match error.error_len() {
                    None if !eof => break,
                    length => {
                        output.push('\u{fffd}');
                        offset += length.unwrap_or(remaining.len());
                    }
                },
            }
        }
        self.pending.drain(..offset);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chunk_boundary_masks_values_and_preserves_utf8_and_progress() {
        let secrets = vec![
            ("KEY".into(), Zeroizing::new("synthetic-value".into())),
            ("LONG".into(), Zeroizing::new("synthetic-value-long".into())),
        ];
        let text = "é🦀\rsynthetic-value-long synthetic-value\nfin";
        let expected = "é🦀\r[LONG] [KEY]\nfin";
        for width in 1..=text.len() {
            let mut stream = SetupOutput::default();
            let mut output = String::new();
            for chunk in text.as_bytes().chunks(width) {
                output.push_str(&stream.push(chunk, &secrets, false));
                assert!(!output.contains("synthetic"));
                assert!(stream.pending.len() < "synthetic-value-long".len());
            }
            output.push_str(&stream.push(&[], &secrets, true));
            assert_eq!(output, expected, "chunk width {width}");
            assert!(stream.pending.is_empty());
        }
    }

    #[test]
    fn progress_with_a_possible_secret_prefix_waits_until_the_next_byte() {
        let secrets = vec![("KEY".into(), Zeroizing::new("synthetic-value".into()))];
        let mut stream = SetupOutput::default();
        assert_eq!(stream.push(b"\rprogress", &secrets, false), "\rprogres");
        assert_eq!(stream.push(b"!", &secrets, false), "s!");
        assert!(stream.pending.is_empty());
    }

    #[test]
    fn malformed_bytes_do_not_hide_the_start_of_a_secret_or_end_the_stream() {
        let secrets = vec![("KEY".into(), Zeroizing::new("secret".into()))];
        let mut stream = SetupOutput::default();
        let mut output = stream.push(b"\xe9secret\xffok\xf0\x9f", &secrets, false);
        output.push_str(&stream.push(&[], &secrets, true));
        assert_eq!(output, "�[KEY]�ok�");
        assert_eq!(stream.push(b"sec", &secrets, false), "");
        assert_eq!(stream.push(&[], &secrets, true), "sec");
    }
}
