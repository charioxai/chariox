//! The mask every Vault command tool puts on its output before the model sees it.
use zeroize::Zeroizing;

/// Returned per stream; the tail is kept, where failures usually are.
pub(super) const RETURNED_BYTES: usize = 32 * 1024;

/// Masks the injected values in the whole stream before cutting it, so no
/// partial value survives at the cut; then sanitizes each line.
pub(super) fn masked(raw: &[u8], masks: &[(String, Zeroizing<String>)]) -> (String, bool) {
    let mut text = Zeroizing::new(String::from_utf8_lossy(raw).into_owned());
    for (id, value) in masks {
        for value in [value.as_str(), value.trim()] {
            if !value.is_empty() && text.contains(value) {
                text = Zeroizing::new(text.replace(value, &format!("[vault:{id}]")));
            }
        }
    }
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
        let (text, cut) = masked(b"got abc-secret-xyz and secret\n\x1b[31mred\x1b[0m", &masks);
        assert_eq!(text, "got [vault:long] and [vault:short]\nred");
        assert!(!cut);
        let mut long = vec![b'a'; RETURNED_BYTES * 2];
        long.extend_from_slice(b"secret");
        let (text, cut) = masked(&long, &masks);
        assert!(cut && text.ends_with("[vault:short]") && !text.contains("secret"));
    }
}
