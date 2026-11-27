//! The mask every Vault command tool puts on its output before the model sees it.
use zeroize::Zeroizing;

/// Returned per stream; the tail is kept, where failures usually are.
pub(super) const RETURNED_BYTES: usize = 32 * 1024;

/// Masks the injected values in the whole stream before cutting it, so no
/// partial value survives at the cut; then sanitizes each line. A stream the
/// kernel stopped collecting at its limit (`at_limit`) may end inside a value:
/// whatever there begins one is dropped too.
pub(super) fn masked(
    raw: &[u8],
    masks: &[(String, Zeroizing<String>)],
    at_limit: bool,
) -> (String, bool) {
    let mut bytes = Zeroizing::new(raw.to_vec());
    for (id, value) in masks {
        for value in [value.as_str(), value.trim()] {
            if !value.is_empty() {
                bytes = replaced(&bytes, value.as_bytes(), format!("[vault:{id}]").as_bytes());
            }
        }
    }
    if at_limit {
        let kept = bytes.len() - value_start_at_end(&bytes, masks);
        bytes.truncate(kept);
    }
    let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
    let text = text
        .split('\n')
        .map(|line| super::super::agent_process_output::sanitize(line.as_bytes()))
        .collect::<Vec<_>>()
        .join("\n");
    match text
        .char_indices()
        .rev()
        .find(|(i, _)| text.len() - i >= RETURNED_BYTES)
    {
        Some((start, _)) => (text[start..].to_string(), true),
        None => (text, false),
    }
}

/// `bytes` with every occurrence of `value` replaced, left to right.
fn replaced(bytes: &[u8], value: &[u8], with: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len()));
    let mut rest = bytes;
    while let Some(at) = rest.windows(value.len()).position(|w| w == value) {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(with);
        rest = &rest[at + value.len()..];
    }
    out.extend_from_slice(rest);
    out
}

/// The length of the longest end of `bytes` that is the start of an injected value.
fn value_start_at_end(bytes: &[u8], masks: &[(String, Zeroizing<String>)]) -> usize {
    masks
        .iter()
        .flat_map(|(_, value)| [value.as_bytes(), value.trim().as_bytes()])
        .filter_map(|value| {
            (1..value.len())
                .rev()
                .find(|&len| bytes.ends_with(&value[..len]))
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn use_env_masks_every_injected_value_before_cutting() {
        let masks = vec![
            (
                "long".to_string(),
                Zeroizing::new("abc-secret-xyz".to_string()),
            ),
            ("short".to_string(), Zeroizing::new("secret".to_string())),
        ];
        let (text, cut) = masked(
            b"got abc-secret-xyz and secret\n\x1b[31mred\x1b[0m",
            &masks,
            false,
        );
        assert_eq!(text, "got [vault:long] and [vault:short]\nred");
        assert!(!cut);
        let mut long = vec![b'a'; RETURNED_BYTES * 2];
        long.extend_from_slice(b"secret");
        let (text, cut) = masked(&long, &masks, false);
        assert!(cut && text.ends_with("[vault:short]") && !text.contains("secret"));
    }

    fn mask(id: &str, value: &str) -> (String, Zeroizing<String>) {
        (id.to_string(), Zeroizing::new(value.to_string()))
    }

    /// The raw stream as collected: `before` then `value`, cut after `kept` bytes of it.
    fn cut_inside(before: &[u8], value: &str, kept: usize) -> Vec<u8> {
        let mut raw = before.to_vec();
        raw.extend_from_slice(&value.as_bytes()[..kept]);
        raw
    }

    #[test]
    fn a_value_cut_at_the_collection_limit_leaves_no_start_of_it() {
        let value = "sk_test_51StraddleValue9f8e7d";
        let masks = vec![mask("key", value)];
        for kept in 1..value.len() {
            let raw = cut_inside(b"pad\npad\n", value, kept);
            let (text, _) = masked(&raw, &masks, true);
            assert_eq!(text, "pad\npad\n", "{kept} bytes of the value at the cut");
        }
        // The value's trimmed form is masked as well, so its start is dropped too.
        let masks = vec![mask("key", "  padded-secret-value\n")];
        let (text, _) = masked(b"out padded-sec", &masks, true);
        assert_eq!(text, "out ");
    }

    #[test]
    fn a_value_ending_exactly_at_the_limit_is_masked_whole() {
        let masks = vec![mask("key", "abab-secret-abab")];
        let (text, _) = masked(b"out abab-secret-abab", &masks, true);
        assert_eq!(text, "out [vault:key]");
        // A value repeating its own start is masked whole, and the start after it dropped.
        let (text, _) = masked(b"out abab-secret-ababab", &masks, true);
        assert_eq!(text, "out [vault:key]");
    }

    #[test]
    fn of_two_values_the_whole_one_is_masked_and_the_cut_one_dropped() {
        let masks = vec![
            mask("long", "second-secret-value-longer"),
            mask("short", "first-secret"),
        ];
        let (text, _) = masked(b"a first-secret b second-secret-va", &masks, true);
        assert_eq!(text, "a [vault:short] b ");
        let (text, _) = masked(b"a second-secret-value-longer b first-sec", &masks, true);
        assert_eq!(text, "a [vault:long] b ");
    }

    #[test]
    fn a_cut_inside_a_multi_byte_character_of_a_value_leaves_no_byte_of_it() {
        let value = "pässwört-geheim-wert";
        let masks = vec![mask("key", value)];
        // 'ä' is two bytes: keep "p" and the first byte of 'ä' only.
        let raw = cut_inside(b"log ", value, 2);
        let (text, _) = masked(&raw, &masks, true);
        assert_eq!(text, "log ");
        let raw = cut_inside(b"log ", value, "pässw".len() + 1);
        let (text, _) = masked(&raw, &masks, true);
        assert_eq!(text, "log ");
    }

    #[test]
    fn a_stream_that_ended_on_its_own_keeps_its_last_bytes() {
        let masks = vec![mask("key", "secret-value")];
        let (text, cut) = masked(b"build failed: see sec", &masks, false);
        assert_eq!(text, "build failed: see sec");
        assert!(!cut);
        // At the limit, an end that begins no value is kept as well.
        let (text, _) = masked(b"build failed: see log", &masks, true);
        assert_eq!(text, "build failed: see log");
    }
}
