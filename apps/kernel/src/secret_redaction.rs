//! Secret-shaped substrings in text the kernel keeps but did not write: App
//! log entries, and tokens in provider diagnostics. A match becomes
//! `[redacted:<kind>]`.
//!
//! One left-to-right pass, with no regular expressions and no backtracking.
//! Every recognizer is anchored at a word start, on a fixed prefix or a key
//! name, and a failed candidate is not retried from inside the text it looked
//! at, so each byte is examined a small constant number of times. A flood of
//! hostile lines costs the same per byte as ordinary text.
//!
//! Recognized:
//! - vendor-prefixed tokens: `sk-ant-` (anthropic-key), `sk-` with a digit
//!   (openai-key), `sk_live_`/`rk_live_` and their `_test_` forms
//!   (stripe-key), `xox[abeprs]-` and `xapp-` (slack-token), `ghp_`, `gho_`,
//!   `ghs_`, `ghu_`, `ghr_` and `github_pat_` (github-token), `AKIA`/`ASIA`
//!   plus 16 (aws-access-key), `AIza` (google-api-key), JSON web tokens
//!   `eyJ….….…` (jwt), and PEM private key blocks (private-key; a block with
//!   no END line is redacted to the end of the text);
//! - by context: `Bearer <token>` (bearer-token), `Authorization: <scheme>
//!   <credentials>`, the password in a URL's user info (url-password), and the
//!   value after a secret-named key: `password=…`, `"token": "…"`,
//!   `client_secret: …`, `AWS_SECRET_ACCESS_KEY=…` (aws-secret-key), …
//!
//! Trade-offs:
//! - Recognizers need a vendor prefix or a key name, so bare high-entropy
//!   strings stay: hex digests, UUIDs, base64 blobs and an AWS secret key
//!   without its key name look like ids and hashes.
//! - Token families need a minimum length (and `sk-` a digit), so short
//!   look-alikes such as `sk-learn` or `ghp_x` stay.
//! - A secret-named key's value is redacted even when it is ordinary text
//!   (`secret: sauce`), except booleans, null, and a few status words
//!   (`missing`, `expired`, …) that log lines use as prose. Keys that only
//!   contain such a word (`password_hash`, `max_tokens`, `secretary`) are not
//!   secret-named.
//! - An unquoted value ends at whitespace or one of `& , ; ) ] } < > " '`. A
//!   flag and its value separated only by a space (`--password hunter2`) is
//!   not recognized.
use std::borrow::Cow;

/// Replaces every secret-shaped substring of `text` with `[redacted:<kind>]`.
/// Borrows when nothing matched; applying it again changes nothing.
pub(crate) fn redact_secrets(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut redacted: Option<String> = None;
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let hit = if is_run_byte(bytes[index]) {
            let end = run_end(bytes, index);
            match scan_run(bytes, index, end) {
                Some(hit) => hit,
                None => {
                    index = end;
                    continue;
                }
            }
        } else if bytes[index..].starts_with(b"://") {
            match url_password(bytes, index + 3) {
                Some(hit) => hit,
                None => {
                    index += 3;
                    continue;
                }
            }
        } else {
            index += 1;
            continue;
        };
        // Hits start and end on ASCII bytes or the end, so both are char
        // boundaries.
        let out = redacted.get_or_insert_with(|| String::with_capacity(text.len()));
        out.push_str(&text[copied..hit.start]);
        out.push_str("[redacted:");
        out.push_str(hit.kind);
        out.push(']');
        copied = hit.end;
        index = hit.end;
    }
    match redacted {
        None => Cow::Borrowed(text),
        Some(mut out) => {
            out.push_str(&text[copied..]);
            Cow::Owned(out)
        }
    }
}

/// Whether `text` holds anything `redact_secrets` would replace.
pub(crate) fn contains_secret(text: &str) -> bool {
    matches!(redact_secrets(text), Cow::Owned(_))
}

/// Redacts every string in `value`, object keys included, and replaces the
/// string or number under a secret-named key outright.
pub(crate) fn redact_json_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if let Cow::Owned(redacted) = redact_secrets(text) {
                *text = redacted;
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_json_secrets),
        serde_json::Value::Object(fields) => {
            for (key, mut value) in std::mem::take(fields) {
                let kind = secret_key_kind(key.as_bytes());
                let scalar = match &value {
                    serde_json::Value::String(text) => !keeps_value(text.as_bytes()),
                    serde_json::Value::Number(_) => true,
                    _ => false,
                };
                match kind {
                    Some(kind) if scalar => value = marker(kind).into(),
                    _ => redact_json_secrets(&mut value),
                }
                fields.insert(redact_secrets(&key).into_owned(), value);
            }
        }
        _ => {}
    }
}

fn marker(kind: &str) -> String {
    format!("[redacted:{kind}]")
}

#[derive(Debug, PartialEq)]
struct Hit {
    start: usize,
    end: usize,
    kind: &'static str,
}

/// Bytes of a word run: base64url, which covers every token family.
fn is_run_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn run_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|byte| !is_run_byte(*byte))
        .map_or(bytes.len(), |offset| from + offset)
}

/// The first secret in the run `bytes[start..end]` or in what follows it
/// (a key's value, a bearer token, a PEM block, a JWT's later segments).
fn scan_run(bytes: &[u8], start: usize, end: usize) -> Option<Hit> {
    let word = &bytes[start..end];
    // `-----BEGIN` is followed by a space, so it can only end a run.
    if word.ends_with(b"-----BEGIN") {
        if let Some(hit) = private_key_block(bytes, end - b"-----BEGIN".len()) {
            return Some(hit);
        }
    }
    if let Some(kind) = secret_key_kind(word) {
        if let Some(hit) = assigned_value(bytes, end, kind) {
            return Some(hit);
        }
    }
    if word.eq_ignore_ascii_case(b"bearer") {
        if let Some(hit) = bearer_token(bytes, end) {
            return Some(hit);
        }
    }
    // Candidates start at the run and after each `-` or `_` in it, so every
    // position is a candidate at most once.
    let last_digit = word
        .iter()
        .rposition(u8::is_ascii_digit)
        .map(|offset| start + offset);
    let mut jwt_tail = None;
    let mut candidate = start;
    while candidate < end {
        if let Some(hit) = prefixed_token(bytes, candidate, end, last_digit, &mut jwt_tail) {
            return Some(hit);
        }
        match bytes[candidate..end]
            .iter()
            .position(|byte| matches!(byte, b'-' | b'_'))
        {
            Some(offset) => candidate += offset + 1,
            None => break,
        }
    }
    None
}

/// A vendor-prefixed token starting at `start` in the run ending at `end`.
fn prefixed_token(
    bytes: &[u8],
    start: usize,
    end: usize,
    last_digit: Option<usize>,
    jwt_tail: &mut Option<Option<usize>>,
) -> Option<Hit> {
    let token = &bytes[start..end];
    let hit = |kind| Some(Hit { start, end, kind });
    let body = |prefix: &[u8], minimum: usize| {
        token.starts_with(prefix) && token.len() - prefix.len() >= minimum
    };
    if body(b"sk-ant-", 20) {
        return hit("anthropic-key");
    }
    if body(b"sk-", 20) && last_digit.is_some_and(|digit| digit >= start + 3) {
        return hit("openai-key");
    }
    if [b"sk_live_", b"sk_test_", b"rk_live_", b"rk_test_"]
        .iter()
        .any(|prefix| body(*prefix, 16))
    {
        return hit("stripe-key");
    }
    if (token.len() > 4
        && token.starts_with(b"xox")
        && matches!(token[3], b'a' | b'b' | b'e' | b'p' | b'r' | b's')
        && token[4] == b'-'
        && token.len() - 5 >= 10)
        || body(b"xapp-", 10)
    {
        return hit("slack-token");
    }
    if body(b"github_pat_", 22)
        || ([b"ghp_", b"gho_", b"ghs_", b"ghu_", b"ghr_"]
            .iter()
            .any(|prefix| body(*prefix, 30)))
    {
        return hit("github-token");
    }
    if (token.starts_with(b"AKIA") || token.starts_with(b"ASIA"))
        && token.len() >= 20
        && token[4..20]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && !bytes.get(start + 20).is_some_and(u8::is_ascii_alphanumeric)
    {
        return Some(Hit {
            start,
            end: start + 20,
            kind: "aws-access-key",
        });
    }
    if body(b"AIza", 30) {
        return hit("google-api-key");
    }
    if body(b"eyJ", 7) {
        // Every candidate in a run shares the segments after it.
        if let Some(tail) = *jwt_tail.get_or_insert_with(|| jwt_tail_end(bytes, end)) {
            return Some(Hit {
                start,
                end: tail,
                kind: "jwt",
            });
        }
    }
    None
}

/// The end of a JWT's payload and signature segments after its header run
/// ends at `header_end`: `.payload.signature`, the signature possibly empty.
fn jwt_tail_end(bytes: &[u8], header_end: usize) -> Option<usize> {
    if bytes.get(header_end) != Some(&b'.') {
        return None;
    }
    let payload_end = run_end(bytes, header_end + 1);
    if payload_end - (header_end + 1) < 10 || bytes.get(payload_end) != Some(&b'.') {
        return None;
    }
    Some(run_end(bytes, payload_end + 1))
}

/// A PEM private key block from `-----BEGIN … PRIVATE KEY-----` through its
/// END line, or to the end of the text when that line is missing.
fn private_key_block(bytes: &[u8], start: usize) -> Option<Hit> {
    let label_start = start + b"-----BEGIN".len();
    if bytes.get(label_start) != Some(&b' ') {
        return None;
    }
    let window = &bytes[label_start..bytes.len().min(label_start + 64)];
    let label_end = label_start + find(window, b"-----")?;
    find(&bytes[label_start..label_end], b"PRIVATE KEY")?;
    let body = label_end + b"-----".len();
    let end = find(&bytes[body..], b"-----END ")
        .and_then(|offset| {
            let after = body + offset + b"-----END ".len();
            let window = &bytes[after..bytes.len().min(after + 64)];
            find(window, b"-----").map(|close| after + close + b"-----".len())
        })
        .unwrap_or(bytes.len());
    Some(Hit {
        start,
        end,
        kind: "private-key",
    })
}

/// The kind of secret a key names, compared case-insensitively with `-`
/// and `_` alike, by suffix: `db_password`, `clientSecret`, `x-api-key`.
fn secret_key_kind(key: &[u8]) -> Option<&'static str> {
    const KINDS: &[(&[u8], &str)] = &[
        (b"secret_access_key", "aws-secret-key"),
        (b"secretaccesskey", "aws-secret-key"),
        (b"password", "password"),
        (b"passwd", "password"),
        (b"passphrase", "password"),
        (b"authorization", "authorization"),
        (b"token", "token"),
        (b"api_key", "api-key"),
        (b"apikey", "api-key"),
        (b"private_key", "private-key"),
        (b"privatekey", "private-key"),
        (b"secret", "secret"),
        (b"secret_key", "secret"),
        (b"secretkey", "secret"),
        (b"access_key", "secret"),
        (b"accesskey", "secret"),
        (b"credential", "secret"),
        (b"credentials", "secret"),
        (b"cookie", "secret"),
    ];
    let start = key.iter().position(|byte| !matches!(byte, b'-' | b'_'))?;
    let end = key.iter().rposition(|byte| !matches!(byte, b'-' | b'_'))? + 1;
    let key = &key[start..end];
    if key.len() > 64 {
        return None;
    }
    if key.eq_ignore_ascii_case(b"pwd") {
        return Some("password");
    }
    KINDS.iter().find_map(|(suffix, kind)| {
        (key.len() >= suffix.len()
            && key[key.len() - suffix.len()..]
                .iter()
                .zip(suffix.iter())
                .all(|(byte, expected)| {
                    let byte = if *byte == b'-' { b'_' } else { *byte };
                    byte.eq_ignore_ascii_case(expected)
                }))
        .then_some(*kind)
    })
}

#[derive(Clone, Copy)]
struct Quote {
    byte: u8,
    /// JSON inside a string: `\"…\"`.
    escaped: bool,
}

fn quote_at(bytes: &[u8], at: usize) -> Option<(usize, Quote)> {
    match (bytes.get(at), bytes.get(at + 1)) {
        (Some(byte @ (b'"' | b'\'')), _) => Some((
            at + 1,
            Quote {
                byte: *byte,
                escaped: false,
            },
        )),
        (Some(b'\\'), Some(byte @ (b'"' | b'\''))) => Some((
            at + 2,
            Quote {
                byte: *byte,
                escaped: true,
            },
        )),
        _ => None,
    }
}

fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}

/// The end of a quoted value: its closing quote, a line end, or the end.
fn quoted_end(bytes: &[u8], mut at: usize, quote: Quote) -> usize {
    while at < bytes.len() {
        match bytes[at] {
            b'\n' | b'\r' => return at,
            b'\\' if quote.escaped && bytes.get(at + 1) == Some(&quote.byte) => return at,
            b'\\' => at += 2,
            // Escaped quoting: a bare quote ends the enclosing string.
            byte if byte == quote.byte => return at,
            _ => at += 1,
        }
    }
    bytes.len()
}

fn unquoted_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|byte| {
            byte.is_ascii_whitespace()
                || matches!(
                    byte,
                    b'&' | b',' | b';' | b')' | b']' | b'}' | b'<' | b'>' | b'"' | b'\''
                )
        })
        .map_or(bytes.len(), |offset| from + offset)
}

/// Values a secret-named key keeps: empty, already redacted, boolean or
/// null, or a status word a log line uses as prose (`token: expired`).
fn keeps_value(value: &[u8]) -> bool {
    const PROSE: &[&[u8]] = &[
        b"true",
        b"false",
        b"null",
        b"nil",
        b"none",
        b"undefined",
        b"missing",
        b"empty",
        b"unset",
        b"set",
        b"required",
        b"invalid",
        b"expired",
        b"revoked",
        b"rotated",
        b"redacted",
        b"present",
        b"absent",
        b"provided",
        b"changed",
        b"updated",
        b"refreshed",
        b"saved",
        b"loaded",
        b"accepted",
        b"rejected",
        b"denied",
        b"failed",
        b"valid",
        b"unknown",
        b"not",
        b"ok",
    ];
    value.is_empty()
        || value.starts_with(b"[redacted:")
        || PROSE.iter().any(|word| value.eq_ignore_ascii_case(word))
}

/// The value after a secret-named key ending at `key_end`: `key=value`,
/// `key: value`, `"key": "value"`, `\"key\":\"value\"`.
fn assigned_value(bytes: &[u8], key_end: usize, kind: &'static str) -> Option<Hit> {
    let mut at = quote_at(bytes, key_end).map_or(key_end, |(after, _)| after);
    at = skip_spaces(bytes, at);
    match bytes.get(at) {
        Some(b'=') => {}
        // Not a path (`secret::Vault`) or a URL scheme (`token://`).
        Some(b':') if !matches!(bytes.get(at + 1), Some(b':' | b'/')) => {}
        _ => return None,
    }
    at = skip_spaces(bytes, at + 1);
    let quote = quote_at(bytes, at).map(|(after, quote)| {
        at = after;
        quote
    });
    let value_end = |from| match quote {
        Some(quote) => quoted_end(bytes, from, quote),
        None => unquoted_end(bytes, from),
    };
    if kind == "authorization" {
        // `Authorization: Bearer <token>` keeps its scheme.
        let scheme_len = bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphabetic())
            .take(17)
            .count();
        let scheme_end = at + scheme_len;
        if (1..=16).contains(&scheme_len) && bytes.get(scheme_end) == Some(&b' ') {
            let start = skip_spaces(bytes, scheme_end);
            let end = value_end(start);
            if end == start || keeps_value(&bytes[start..end]) {
                return None;
            }
            let bearer = bytes[at..scheme_end].eq_ignore_ascii_case(b"bearer");
            return Some(Hit {
                start,
                end,
                kind: if bearer { "bearer-token" } else { kind },
            });
        }
    }
    let end = value_end(at);
    (!keeps_value(&bytes[at..end])).then_some(Hit {
        start: at,
        end,
        kind,
    })
}

/// `Bearer <token>` outside an Authorization header. The token needs a digit
/// or 20 characters, so prose such as "Bearer authentication" stays.
fn bearer_token(bytes: &[u8], word_end: usize) -> Option<Hit> {
    let start = skip_spaces(bytes, word_end);
    if start == word_end {
        return None;
    }
    let end = bytes[start..]
        .iter()
        .position(|byte| {
            !(byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'='))
        })
        .map_or(bytes.len(), |offset| start + offset);
    let token = &bytes[start..end];
    (token.len() >= 8 && (token.len() >= 20 || token.iter().any(u8::is_ascii_digit))).then_some(
        Hit {
            start,
            end,
            kind: "bearer-token",
        },
    )
}

/// The password in a URL's user info, `scheme://user:password@host`, read
/// from just after `://`.
fn url_password(bytes: &[u8], from: usize) -> Option<Hit> {
    let mut colon = None;
    for at in from..bytes.len().min(from + 256) {
        match bytes[at] {
            b'@' => {
                let colon = colon?;
                return (at > colon + 1).then_some(Hit {
                    start: colon + 1,
                    end: at,
                    kind: "url-password",
                });
            }
            b':' if colon.is_none() => colon = Some(at),
            b'/' | b'?' | b'#' | b'[' | b']' | b'"' | b'\'' | b'<' | b'>' | b'\\' => return None,
            byte if byte.is_ascii_whitespace() => return None,
            _ => {}
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests;
