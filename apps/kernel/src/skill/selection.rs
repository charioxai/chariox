//! MP-08 / MP-11: one explicit-token parser for ordinary and native clients.
use std::collections::BTreeSet;

/// Lowercase skill tokens are standalone. Shell variables (including $HOME),
/// escaped dollars, code spans/fences, quoted code and currency are excluded.
/// Names are validated by the same collision key as catalog mutation.
pub fn explicit_skill_names(prompt: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    explicit_skill_tokens(prompt)
        .into_iter()
        .filter_map(|token| seen.insert(token.name.clone()).then_some(token.name))
        .collect()
}

/// MP-08 / MP-10 / MP-11: provider-facing labels use the same admitted token
/// ranges, preserving dollar syntax in source examples, paths and attachments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitSkillToken {
    pub name: String,
    pub range: std::ops::Range<usize>,
}
pub fn explicit_skill_tokens(prompt: &str) -> Vec<ExplicitSkillToken> {
    explicit_skill_tokens_with_names(prompt, &[])
}

/// Typed, kernel-validated chips may name e.g. UPPER or 123 without making
/// unrelated shell variables/currency into ordinary typed skill invocation.
pub fn explicit_skill_tokens_with_names(
    prompt: &str,
    selected_names: &[&str],
) -> Vec<ExplicitSkillToken> {
    let bytes = prompt.as_bytes();
    let mut i = 0;
    let mut names = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut span: Option<usize> = None;
    for line in prompt.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied();
        let count = marker
            .map(|m| trimmed.as_bytes().iter().take_while(|b| **b == m).count())
            .unwrap_or(0);
        if span.is_none() && indent <= 3 && count >= 3 && matches!(marker, Some(b'`' | b'~')) {
            if let Some((m, n)) = fence {
                if marker == Some(m) && count >= n && trimmed[count..].trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = Some((marker.unwrap(), count));
            }
            i += line.len();
            continue;
        }
        if fence.is_some() || (indent >= 4 && span.is_none()) {
            i += line.len();
            continue;
        }
        let end = i + line.len();
        while i < end {
            if let Some(n) = span {
                if bytes[i] == b'`' {
                    let start = i;
                    while i < end && bytes[i] == b'`' {
                        i += 1;
                    }
                    if i - start == n {
                        span = None;
                    }
                } else {
                    i += 1;
                }
                continue;
            }
            match bytes[i] {
                b'[' => {
                    if let Some(close) = prompt[i..end].find("](") {
                        if let Some(destination_end) = prompt[i + close + 2..end].find(')') {
                            i += close + 2 + destination_end + 1;
                            continue;
                        }
                    }
                    i += 1;
                }
                b'\\' => {
                    i += 1;
                    if i < end {
                        i += 1;
                    }
                }
                b'`' => {
                    let start = i;
                    while i < end && bytes[i] == b'`' {
                        i += 1;
                    }
                    span = Some(i - start);
                }
                b'\'' | b'"' => {
                    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
                        i += 1;
                        continue;
                    }
                    let quote = bytes[i];
                    i += 1;
                    while i < end {
                        if bytes[i] == b'\\' {
                            i = (i + 2).min(end);
                        } else if bytes[i] == quote {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                }
                b'$' => {
                    let start = i;
                    i += 1;
                    if start > 0
                        && !bytes[start - 1].is_ascii_whitespace()
                        && !matches!(bytes[start - 1], b'(' | b'[')
                    {
                        continue;
                    }
                    let name_start = i;
                    while i < end
                        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_'))
                    {
                        i += 1;
                    }
                    let name = &prompt[name_start..i];
                    let selected = selected_names
                        .iter()
                        .any(|selected| selected.eq_ignore_ascii_case(name));
                    if name.is_empty()
                        || (!selected
                            && (!name.as_bytes()[0].is_ascii_alphabetic()
                                || name.bytes().all(|b| {
                                    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
                                })))
                    {
                        continue;
                    }
                    if i < end
                        && !bytes[i].is_ascii_whitespace()
                        && !matches!(
                            bytes[i],
                            b')' | b']' | b',' | b'.' | b'!' | b'?' | b';' | b':'
                        )
                    {
                        continue;
                    }
                    // Markdown attachment/link/image destinations are never invocation.
                    if i < end && bytes[i] == b']' && bytes.get(i + 1) == Some(&b'(') {
                        continue;
                    }
                    if super::catalog::collision_key(name).is_ok() {
                        names.push(ExplicitSkillToken {
                            name: name.to_string(),
                            range: start..i,
                        });
                    }
                }
                _ => i += 1,
            }
        }
    }
    names
}
