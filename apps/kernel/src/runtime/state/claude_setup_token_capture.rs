use zeroize::Zeroizing;

/// Every Anthropic credential printed by `claude setup-token` starts with this
/// prefix. Any occurrence is withheld from projected output, captured or not.
const TOKEN_PREFIX: &[u8] = b"sk-ant-";
const MIN_CAPTURED_TOKEN_BYTES: usize = 40;
/// A token run longer than this is treated as complete even before a
/// terminator arrives, so a hostile stream cannot grow the withheld buffer.
const MAX_WITHHELD_BYTES: usize = 4 * 1024;
pub(super) const REDACTED_TOKEN_MARKER: &[u8] = b"[setup token captured; hidden]";

/// Filters `claude setup-token` PTY output before it is recorded. Tokens are
/// captured into zeroizing memory and replaced by a marker; a possible token
/// split across PTY chunks is withheld until the next chunk resolves it.
#[derive(Default)]
pub(in crate::runtime) struct ClaudeSetupTokenCapture {
    withheld: Zeroizing<Vec<u8>>,
    token: Option<Zeroizing<String>>,
}

impl ClaudeSetupTokenCapture {
    pub fn filter(&mut self, input: &[u8], finished: bool) -> Vec<u8> {
        let mut buffer = Zeroizing::new(std::mem::take(&mut *self.withheld));
        buffer.extend_from_slice(input);
        let mut output = Vec::with_capacity(buffer.len());
        let mut cursor = 0;
        while let Some(offset) = find(&buffer[cursor..], TOKEN_PREFIX) {
            let start = cursor + offset;
            output.extend_from_slice(&buffer[cursor..start]);
            let mut end = start + TOKEN_PREFIX.len();
            while end < buffer.len() && is_token_byte(buffer[end]) {
                end += 1;
            }
            if end == buffer.len() && !finished && end - start < MAX_WITHHELD_BYTES {
                self.withheld.extend_from_slice(&buffer[start..]);
                return output;
            }
            if end - start >= MIN_CAPTURED_TOKEN_BYTES {
                if let Ok(token) = std::str::from_utf8(&buffer[start..end]) {
                    self.token = Some(Zeroizing::new(token.to_string()));
                }
            }
            output.extend_from_slice(REDACTED_TOKEN_MARKER);
            cursor = end;
        }
        let keep = if finished {
            0
        } else {
            partial_prefix_suffix_len(&buffer[cursor..])
        };
        let split = buffer.len() - keep;
        output.extend_from_slice(&buffer[cursor..split]);
        self.withheld.extend_from_slice(&buffer[split..]);
        output
    }

    pub fn token(&self) -> Option<&Zeroizing<String>> {
        self.token.as_ref()
    }

    pub fn discard_token(&mut self) {
        self.token = None;
    }
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Length of the longest proper prefix of `sk-ant-` that ends `bytes`.
fn partial_prefix_suffix_len(bytes: &[u8]) -> usize {
    (1..TOKEN_PREFIX.len())
        .rev()
        .find(|len| bytes.ends_with(&TOKEN_PREFIX[..*len]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str =
        "sk-ant-oat01-TESTONLYaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_bbbb-cccc";

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        find(haystack, needle).is_some()
    }

    #[test]
    fn captures_and_redacts_a_token_printed_in_one_chunk() {
        let mut capture = ClaudeSetupTokenCapture::default();
        let output = capture.filter(
            format!("Your OAuth token (valid for 1 year):\r\n\x1b[1m{TOKEN}\x1b[22m\r\nStore this token securely.\r\n").as_bytes(),
            false,
        );

        assert!(!contains(&output, b"sk-ant-"));
        assert!(contains(&output, REDACTED_TOKEN_MARKER));
        assert!(contains(&output, b"Store this token securely."));
        assert_eq!(capture.token().map(|token| token.as_str()), Some(TOKEN));
    }

    #[test]
    fn a_token_split_across_every_byte_boundary_never_leaks() {
        let stream = format!("token:\r\n{TOKEN}\r\ndone\r\n");
        for split in 0..stream.len() {
            let mut capture = ClaudeSetupTokenCapture::default();
            let mut output = capture.filter(&stream.as_bytes()[..split], false);
            output.extend(capture.filter(&stream.as_bytes()[split..], false));
            output.extend(capture.filter(&[], true));

            let text = String::from_utf8_lossy(&output);
            assert!(!text.contains("sk-ant"), "split {split} leaked: {text}");
            assert!(!text.contains("TESTONLY"), "split {split} leaked: {text}");
            assert_eq!(
                text, "token:\r\n[setup token captured; hidden]\r\ndone\r\n",
                "split {split}"
            );
            assert_eq!(capture.token().map(|token| token.as_str()), Some(TOKEN));
        }
    }

    #[test]
    fn a_token_at_the_end_of_the_stream_is_released_only_redacted() {
        let mut capture = ClaudeSetupTokenCapture::default();
        let pending = capture.filter(format!("token: {TOKEN}").as_bytes(), false);
        assert_eq!(pending, b"token: ");

        let finished = capture.filter(&[], true);
        assert_eq!(finished, REDACTED_TOKEN_MARKER);
        assert_eq!(capture.token().map(|token| token.as_str()), Some(TOKEN));
    }

    #[test]
    fn short_prefix_matches_are_redacted_without_being_captured() {
        let mut capture = ClaudeSetupTokenCapture::default();
        let output = capture.filter(b"example sk-ant-xyz here", true);

        assert_eq!(output, b"example [setup token captured; hidden] here");
        assert!(capture.token().is_none());
    }

    #[test]
    fn ordinary_output_passes_through_except_a_possible_prefix_tail() {
        let mut capture = ClaudeSetupTokenCapture::default();
        assert_eq!(
            capture.filter(b"Paste code here if prompted > sk", false),
            b"Paste code here if prompted > "
        );
        assert_eq!(capture.filter(b"y is blue", false), b"sky is blue");
        assert!(capture.token().is_none());
    }
}
