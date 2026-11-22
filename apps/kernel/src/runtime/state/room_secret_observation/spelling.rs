//! MP-08/MP-10/MP-11: the spellings of a protected value that observations redact.
//! A value shorter than `BARE_WORD_CHARS` (a short password, a PIN) is redacted only as a
//! whole word of visible text, so it never rewrites a URL, an id, a selector or a longer word.
use base64::Engine as _;
use zeroize::{Zeroize, Zeroizing};

/// Values below this many characters are masked as whole words only.
pub(super) const BARE_WORD_CHARS: usize = 8;

#[derive(Clone)]
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

#[cfg(test)]
thread_local! {
    /// How many times this thread computed a value's spellings.
    pub(super) static SPELLINGS_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn secret_spellings(secret: &str) -> Vec<Spelling> {
    #[cfg(test)]
    SPELLINGS_COMPUTED.with(|count| count.set(count.get() + 1));
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
    redact_split_with(parts, variants, &Prefixes::of(variants))
}

fn redact_split_with(parts: &mut [String], variants: &[Spelling], prefixes: &Prefixes) -> bool {
    let joined = parts.concat();
    if !prefixes.any_spelling_in(&joined, variants) {
        return false;
    }
    let mut spans = Vec::new();
    let mut at = 0;
    while at < joined.len() {
        if let Some(variant) = prefixes
            .may_start_at(joined.as_bytes(), at)
            .then(|| {
                variants
                    .iter()
                    .find(|variant| variant.matches_at(&joined, at))
            })
            .flatten()
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

/// The first two bytes of every spelling, as bits: a scan rejects nearly every
/// text position with one lookup and compares spellings only where one could
/// start. Zeroed on drop like the spellings it was made from.
struct Prefixes {
    pairs: Zeroizing<Vec<u64>>,
    singles: Zeroizing<[u64; 4]>,
}

impl Prefixes {
    fn of(spellings: &[Spelling]) -> Self {
        let mut prefixes = Self {
            pairs: Zeroizing::new(vec![0; 1 << 10]),
            singles: Zeroizing::new([0; 4]),
        };
        for spelling in spellings {
            match spelling.as_str().as_bytes() {
                [] => {}
                [only] => prefixes.singles[usize::from(*only) >> 6] |= 1 << (only & 63),
                [first, second, ..] => {
                    let pair = usize::from(*first) << 8 | usize::from(*second);
                    prefixes.pairs[pair >> 6] |= 1 << (pair & 63);
                }
            }
        }
        prefixes
    }

    fn may_start_at(&self, text: &[u8], at: usize) -> bool {
        let first = text[at];
        if self.singles[usize::from(first) >> 6] & (1 << (first & 63)) != 0 {
            return true;
        }
        text.get(at + 1).is_some_and(|second| {
            let pair = usize::from(first) << 8 | usize::from(*second);
            self.pairs[pair >> 6] & (1 << (pair & 63)) != 0
        })
    }

    /// Whether any spelling occurs anywhere in `text`, whole word or not.
    fn any_spelling_in(&self, text: &str, spellings: &[Spelling]) -> bool {
        let bytes = text.as_bytes();
        (0..bytes.len()).any(|at| {
            self.may_start_at(bytes, at)
                && spellings
                    .iter()
                    .any(|spelling| bytes[at..].starts_with(spelling.as_str().as_bytes()))
        })
    }
}

/// Everything a Room's protected values redact with, computed once per change
/// of its value lists and shared by every scrub until the next change.
pub(super) struct Redactor {
    /// Live, retired, then stored values' spellings, each list longest first:
    /// the order each string is redacted in.
    ordered: Vec<Spelling>,
    /// All spellings longest first, for values split across stream chunks.
    longest_first: Vec<Spelling>,
    prefixes: Prefixes,
}

impl Redactor {
    pub(super) fn new(value_lists: [&[Zeroizing<String>]; 3]) -> Self {
        let mut ordered = Vec::new();
        let mut longest_first = Vec::new();
        for values in value_lists {
            let mut spellings = values
                .iter()
                .flat_map(|value| secret_spellings(value))
                .collect::<Vec<_>>();
            longest_first.extend(spellings.iter().cloned());
            spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
            ordered.extend(spellings);
        }
        longest_first.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
        let prefixes = Prefixes::of(&ordered);
        Self {
            ordered,
            longest_first,
            prefixes,
        }
    }

    /// The longest spelling less one byte: how much text a later append can
    /// still complete into a match.
    pub(super) fn boundary_bytes(&self) -> usize {
        self.ordered
            .iter()
            .map(|spelling| spelling.len().saturating_sub(1))
            .max()
            .unwrap_or(0)
    }

    /// `text` with every protected spelling redacted, or `None` when it holds none.
    pub(super) fn redact(&self, text: &str) -> Option<String> {
        if !self.prefixes.any_spelling_in(text, &self.ordered) {
            return None;
        }
        let mut text = text.to_string();
        for spelling in &self.ordered {
            text = spelling.redact_in(&text);
        }
        Some(text)
    }

    /// See `redact_split_values`.
    pub(super) fn redact_split(&self, parts: &mut [String]) -> bool {
        redact_split_with(parts, &self.longest_first, &self.prefixes)
    }

    pub(super) fn scrub_value(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => {
                if let Some(redacted) = self.redact(text) {
                    text.zeroize();
                    *text = redacted;
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    self.scrub_value(value);
                }
            }
            serde_json::Value::Object(values) => {
                if values.keys().all(|key| self.redact(key).is_none()) {
                    for value in values.values_mut() {
                        self.scrub_value(value);
                    }
                    return;
                }
                let entries = std::mem::take(values);
                for (key, mut value) in entries {
                    let key = self.redact(&key).unwrap_or(key);
                    self.scrub_value(&mut value);
                    values.insert(key, value);
                }
            }
            _ => {}
        }
    }
}
