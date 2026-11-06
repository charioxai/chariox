//! MP-08/MP-10/MP-11: the spellings of a protected value that observations redact.
//! A value shorter than `BARE_WORD_CHARS` (a short password, a PIN) is redacted only as a
//! whole word of visible text, so it never rewrites a URL, an id, a selector or a longer word.
use base64::Engine as _;
use zeroize::Zeroizing;

/// Values below this many characters are masked as whole words only.
pub(super) const BARE_WORD_CHARS: usize = 8;

pub(super) struct Spelling {
    text: Zeroizing<String>,
    whole_word: bool,
}

impl Spelling {
    pub(super) fn as_str(&self) -> &str {
        &self.text
    }

    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    /// `text` with every redactable occurrence of this spelling replaced.
    pub(super) fn redact_in(&self, text: &str) -> String {
        if !self.whole_word {
            return text.replace(self.as_str(), "[redacted]");
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        while let Some(found) = text[at..].find(self.as_str()) {
            let (start, end) = (at + found, at + found + self.len());
            out.push_str(&text[at..start]);
            out.push_str(if whole_word_at(text, start, end) {
                "[redacted]"
            } else {
                &text[start..end]
            });
            at = end;
        }
        out.push_str(&text[at..]);
        out
    }

    fn matches_at(&self, text: &str, start: usize) -> bool {
        text[start..].starts_with(self.as_str())
            && (!self.whole_word || whole_word_at(text, start, start + self.len()))
    }
}

/// The match [start, end) stands alone in prose: no letter, digit, `_`, `-`, `.`, `/` or other
/// URL, id or selector character touches it. Sentence punctuation after it is allowed.
fn whole_word_at(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let mut after = text[end..].chars();
    let next = after.next();
    let edge_before = before.is_none_or(|c| c.is_whitespace() || "\"'`([{<>,;|*!:".contains(c));
    let edge_after = match next {
        None => true,
        Some(c) if c.is_whitespace() => true,
        Some('.') => after.next().is_none_or(char::is_whitespace),
        Some(c) => "\"'`)]}<>,;|*!?:".contains(c),
    };
    edge_before && edge_after
}

pub(super) fn secret_spellings(secret: &str) -> Vec<Spelling> {
    let percent = secret
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    let hex = secret
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let json = serde_json::to_string(secret).unwrap_or_default();
    let html = secret
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    let whole_word = secret.chars().count() < BARE_WORD_CHARS;
    vec![
        secret.to_string(),
        secret.to_lowercase(),
        secret.to_uppercase(),
        percent.clone(),
        percent.to_lowercase(),
        hex.clone(),
        hex.to_uppercase(),
        html,
        json[1..json.len().saturating_sub(1)].to_string(),
        base64::engine::general_purpose::STANDARD.encode(secret),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(secret),
        base64::engine::general_purpose::URL_SAFE.encode(secret),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .map(|text| Spelling {
        text: Zeroizing::new(text),
        whole_word,
    })
    .collect()
}

/// Replaces every spelling occurrence in the parts joined together; a match that
/// crosses parts becomes one marker in the part where it starts. Returns whether
/// anything was replaced.
pub(super) fn redact_split_values(parts: &mut [String], variants: &[Spelling]) -> bool {
    let joined = parts.concat();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < joined.len() {
        if let Some(variant) = variants
            .iter()
            .find(|variant| variant.matches_at(&joined, at))
        {
            spans.push((at, at + variant.len()));
            at += variant.len();
        } else {
            at += joined[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    if spans.is_empty() {
        return false;
    }
    let mut part_start = 0;
    for part in parts.iter_mut() {
        let part_end = part_start + part.len();
        let mut rebuilt = String::new();
        let mut cursor = part_start;
        for &(start, end) in &spans {
            if end <= part_start || start >= part_end {
                continue;
            }
            rebuilt.push_str(&joined[cursor..start.max(part_start)]);
            if start >= part_start {
                rebuilt.push_str("[redacted]");
            }
            cursor = end.min(part_end);
        }
        rebuilt.push_str(&joined[cursor..part_end]);
        *part = rebuilt;
        part_start = part_end;
    }
    true
}
