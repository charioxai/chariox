//! RFC 6238. Only the seed is stored; each injection computes a fresh code.
use data_encoding::{BASE32, BASE32_NOPAD};
use hmac::{Hmac, Mac};
use zeroize::Zeroizing;

use crate::config::{TotpAlgorithm, TotpConfig};
use crate::error::DaemonError;

use super::secret_error;

fn invalid_input() -> DaemonError {
    secret_error(
        "credential_totp",
        "invalid TOTP key; use a base32 manual key or an otpauth://totp URI with SHA1, SHA256 or SHA512, 6 or 8 digits, and period 1-3600".into(),
    )
}

/// URI parameters are provisioning metadata, never part of a model-bound result.
/// Persist the canonical seed and the resulting non-secret configuration together.
pub(super) fn normalize(
    value: &str,
    configured: &TotpConfig,
) -> Result<(Zeroizing<String>, TotpConfig), DaemonError> {
    if value.len() > 16_384 {
        return Err(invalid_input());
    }
    let value = value.trim();
    let mut config = configured.clone();
    let mut seed = Zeroizing::new(value.to_string());
    if value.starts_with("otpauth:") {
        let uri = url::Url::parse(value).map_err(|_| invalid_input())?;
        if uri.scheme() != "otpauth"
            || uri.host_str() != Some("totp")
            || !uri.username().is_empty()
            || uri.password().is_some()
            || uri.port().is_some()
            || uri.fragment().is_some()
            || uri.path().trim_matches('/').is_empty()
        {
            return Err(invalid_input());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut uri_seed = None;
        for (name, value) in uri.query_pairs() {
            if !seen.insert(name.to_string()) {
                return Err(invalid_input());
            }
            match name.as_ref() {
                "secret" => uri_seed = Some(Zeroizing::new(value.into_owned())),
                "algorithm" => {
                    config.algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => TotpAlgorithm::Sha1,
                        "SHA256" => TotpAlgorithm::Sha256,
                        "SHA512" => TotpAlgorithm::Sha512,
                        _ => return Err(invalid_input()),
                    };
                }
                "digits" => config.digits = value.parse().map_err(|_| invalid_input())?,
                "period" => config.period_s = value.parse().map_err(|_| invalid_input())?,
                "issuer" => {}
                _ => return Err(invalid_input()),
            }
        }
        seed = uri_seed.ok_or_else(invalid_input)?;
    }
    config.validate()?;
    let canonical = Zeroizing::new(
        seed.chars()
            .filter(|c| !c.is_ascii_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>(),
    );
    let decoded = decode_seed(&canonical)?;
    Ok((Zeroizing::new(BASE32_NOPAD.encode(&decoded)), config))
}

fn decode_seed(seed: &str) -> Result<Zeroizing<Vec<u8>>, DaemonError> {
    let decoded = if seed.contains('=') {
        BASE32.decode(seed.as_bytes())
    } else {
        BASE32_NOPAD.decode(seed.as_bytes())
    }
    .map_err(|_| invalid_input())?;
    if decoded.len() < 10 || decoded.len() > 1024 {
        return Err(invalid_input());
    }
    Ok(Zeroizing::new(decoded))
}

/// A page shows its setup key inside a sentence ("Or enter the secret: ABCD EFGH …"), so a
/// captured element is not always the bare key. Finds the key in such text: an `otpauth://`
/// URI, one long token, or four or more equal-size groups (`ABCD-EFGH-…`). Capture never stores
/// a seed other than the one shown: when the text allows more than one key, there is no key.
/// Returns what to store, not yet normalized.
pub(crate) fn locate(text: &str) -> Option<Zeroizing<String>> {
    let defaults = TotpConfig::default();
    let valid = |candidate: &str| normalize(candidate, &defaults).is_ok();
    let found = |candidate: &str| Some(Zeroizing::new(candidate.to_string()));
    // A standalone manual key has no prose to disambiguate. The normalizer
    // accepts lowercase and valid padding; keep that contract here too.
    let standalone = text.trim();
    if !standalone.chars().any(char::is_whitespace) && valid(standalone) {
        return found(standalone);
    }
    if let Some(start) = text.find("otpauth://") {
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
            .map_or(text.len(), |length| start + length);
        if valid(&text[start..end]) {
            return found(&text[start..end]);
        }
    }
    let tokens = tokenize(text);
    // A real key mixes in digits or is all capitals; a lowercase or capitalized word is prose.
    let looks_like_key = |token: &str| {
        token.bytes().any(|b| b.is_ascii_digit()) || !token.bytes().any(|b| b.is_ascii_lowercase())
    };
    let mut singles = tokens
        .iter()
        .map(|(token, _)| *token)
        .filter(|token| {
            token.len() >= 16 && base32_only(token) && looks_like_key(token) && valid(token)
        })
        .collect::<Vec<_>>();
    singles.sort_unstable();
    singles.dedup();
    match singles.as_slice() {
        [] => {}
        [single] => return found(single),
        _ => return None,
    }
    let bare = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '=' || c == '-' || c.is_whitespace());
    let mut candidates = Vec::new();
    let mut start = 0;
    while start < tokens.len() {
        let run = group_run(&tokens, start);
        if run.is_empty() {
            start += 1;
            continue;
        }
        let next = start + run.len();
        let dangling = tokens.get(next).is_some_and(|(token, separator)| {
            joins(separator)
                && base32_only(token)
                && !mixed_case(token)
                && fits_group_tail(token, run[0].0.len())
        });
        start = next;
        if dangling {
            continue;
        }
        let groups = run.iter().map(|(token, _)| *token).collect::<Vec<_>>();
        if bare && groups.len() == tokens.len() {
            return bare_key(&groups, &valid).map(Zeroizing::new);
        }
        candidates.extend(segment_candidates(&groups, &valid));
    }
    candidates.sort_unstable();
    candidates.dedup();
    match candidates.as_slice() {
        [candidate] => found(candidate),
        _ => None,
    }
}

fn base32_only(token: &str) -> bool {
    token
        .bytes()
        .all(|b| b.is_ascii_alphabetic() || (b'2'..=b'7').contains(&b) || b == b'=')
}

fn mixed_case(token: &str) -> bool {
    token.bytes().any(|b| b.is_ascii_lowercase()) && token.bytes().any(|b| b.is_ascii_uppercase())
}

/// Only spaces, dashes and line breaks sit between the groups of a key.
fn joins(separator: &str) -> bool {
    separator.chars().all(|c| c.is_whitespace() || c == '-')
}

/// A word that continues a run of groups without being one: a shorter last group, or a padded
/// one. A key followed by it may be longer than the groups before it, so the run is no key.
fn fits_group_tail(token: &str, size: usize) -> bool {
    token.len() < size || token.contains('=')
}

/// The longest run of tokens at `start` that can be groups of one key: equal size, base32, joined
/// by spaces, dashes or line breaks only. A capitalized or otherwise mixed-case word is prose.
fn group_run<'a>(tokens: &[(&'a str, &'a str)], start: usize) -> Vec<(&'a str, &'a str)> {
    let size = tokens[start].0.len();
    if !(2..=8).contains(&size) {
        return Vec::new();
    }
    tokens[start..]
        .iter()
        .enumerate()
        .take_while(|(index, (token, separator))| {
            token.len() == size
                && base32_only(token)
                && !mixed_case(token)
                && (*index == 0 || joins(separator))
        })
        .map(|(_, token)| *token)
        .collect()
}

/// Letter case of a group: `Some(true)` lowercase, `Some(false)` uppercase, `None` neither
/// (digits and padding), which belongs to the neighbouring segment.
fn group_case(group: &str) -> Option<bool> {
    let lower = group.bytes().any(|b| b.is_ascii_lowercase());
    (lower || group.bytes().any(|b| b.is_ascii_uppercase())).then_some(lower)
}

/// Splits groups where the letter case changes; ranges of group indexes.
fn case_segments(groups: &[&str]) -> Vec<std::ops::Range<usize>> {
    let mut segments: Vec<std::ops::Range<usize>> = Vec::new();
    let mut current = None;
    for (index, group) in groups.iter().enumerate() {
        match (group_case(group), current) {
            (Some(case), Some(previous)) if case != previous => {
                segments.push(index..index + 1);
                current = Some(case);
            }
            (case, _) => {
                match segments.last_mut() {
                    Some(last) => last.end = index + 1,
                    None => segments.push(index..index + 1),
                }
                current = current.or(case);
            }
        }
    }
    segments
}

/// A window of groups makes a key when it keeps whole bytes (a multiple of 8 characters), has at
/// least four groups, and ends the run if the run carries padding.
fn window_key(
    groups: &[&str],
    window: std::ops::Range<usize>,
    valid: &impl Fn(&str) -> bool,
) -> Option<String> {
    let joined = groups[window.clone()].concat();
    let ends_run = window.end == groups.len() || !groups.iter().any(|group| group.contains('='));
    (window.len() >= 4 && joined.len() >= 16 && joined.len() % 8 == 0 && ends_run && valid(&joined))
        .then_some(joined)
}

/// Keys formed by whole case segments of a run inside prose. A change of letter case is accepted
/// as the edge of the key, never as proof of it.
fn segment_candidates(groups: &[&str], valid: &impl Fn(&str) -> bool) -> Vec<String> {
    let segments = case_segments(groups);
    let mut keys = Vec::new();
    for first in 0..segments.len() {
        for last in first..segments.len() {
            keys.extend(window_key(
                groups,
                segments[first].start..segments[last].end,
                valid,
            ));
        }
    }
    keys
}

/// The element shows nothing but groups. One case means any valid shorter run of the same groups
/// could be the key with label words stuck to it ("SEED ABCD … CODE"), so only the unique run
/// counts. Several cases mean a mixed-case key shown in a regular pattern: equal-size case blocks.
fn bare_key(groups: &[&str], valid: &impl Fn(&str) -> bool) -> Option<String> {
    let whole = window_key(groups, 0..groups.len(), valid)?;
    let segments = case_segments(groups);
    if segments.len() > 1 {
        let regular = segments
            .iter()
            .all(|segment| segment.len() == segments[0].len());
        return regular.then_some(whole);
    }
    // Padding marks the end of a deliberately formatted key, so a leading group is not a label.
    if groups.iter().any(|group| group.contains('=')) {
        return Some(whole);
    }
    let shorter = (0..groups.len()).any(|first| {
        (first + 1..=groups.len()).any(|last| {
            (first, last) != (0, groups.len()) && window_key(groups, first..last, valid).is_some()
        })
    });
    (!shorter).then_some(whole)
}

/// Splits `text` into alphanumeric words (`=` counts, for padding), each with the separator text
/// that precedes it, so a caller can tell a space or dash from a colon or bracket.
fn tokenize(text: &str) -> Vec<(&str, &str)> {
    let is_token = |c: char| c.is_ascii_alphanumeric() || c == '=';
    let mut tokens = Vec::new();
    let mut separator_start = 0;
    let mut token_start = None;
    for (index, c) in text.char_indices() {
        match (is_token(c), token_start) {
            (true, None) => token_start = Some(index),
            (false, Some(start)) => {
                tokens.push((&text[start..index], &text[separator_start..start]));
                token_start = None;
                separator_start = index;
            }
            _ => {}
        }
    }
    if let Some(start) = token_start {
        tokens.push((&text[start..], &text[separator_start..start]));
    }
    tokens
}

pub(super) fn code(
    seed: &str,
    config: &TotpConfig,
    unix_seconds: u64,
) -> Result<String, DaemonError> {
    config.validate()?;
    let key = decode_seed(seed)?;
    let counter = (unix_seconds / u64::from(config.period_s)).to_be_bytes();
    macro_rules! digest {
        ($algorithm:ty) => {{
            let mut mac = Hmac::<$algorithm>::new_from_slice(&key).map_err(|_| invalid_input())?;
            mac.update(&counter);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }};
    }
    let digest = match config.algorithm {
        TotpAlgorithm::Sha1 => digest!(sha1::Sha1),
        TotpAlgorithm::Sha256 => digest!(sha2::Sha256),
        TotpAlgorithm::Sha512 => digest!(sha2::Sha512),
    };
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    Ok(format!(
        "{:0width$}",
        binary % 10_u32.pow(u32::from(config.digits)),
        width = usize::from(config.digits),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_a_setup_key_in_page_text_and_never_in_prose() {
        let seed = "JBSWY3DPEHPK3PXP";
        for text in [
            seed.to_string(),
            "jbsw y3dp ehpk 3pxp".to_string(),
            format!(
                "Or enter the secret: {seed}\nAnd enter the passcode shown in the application:"
            ),
            format!("Setup key: {}", "JBSW-Y3DP-EHPK-3PXP"),
            "Manual entry key jbsw y3dp ehpk 3pxp (spaces optional)".to_string(),
            format!("Open otpauth://totp/Fixture:alice?secret={seed}&issuer=Fixture to add it"),
        ] {
            let located = locate(&text).unwrap_or_else(|| panic!("no key in {text:?}"));
            let (canonical, _) = normalize(&located, &TotpConfig::default()).unwrap();
            assert_eq!(canonical.as_str(), seed, "{text:?}");
        }
        for text in [
            "",
            "Or enter the secret",
            "This page shows nothing that looks like a key at all",
            "Scan this image with your authentication application:",
            "This also gives game plan ideas around town",
        ] {
            assert!(locate(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn page_text_around_a_key_never_joins_the_seed() {
        // The RFC 6238 SHA-1 test key "12345678901234567890" shown as eight groups of four. Label
        // words have the length of a group, so only the boundary rules keep them out of the seed.
        let groups = "GEZD GNBV GY3T QOJQ GEZD GNBV GY3T QOJQ";
        let config = TotpConfig {
            digits: 8,
            ..TotpConfig::default()
        };
        let dashed = groups.replace(' ', "-");
        let lined = groups.replace(' ', "\n");
        for text in [
            format!("Your seed: {groups}"),
            format!("Seed {groups} code"),
            format!("Key: {dashed}."),
            format!("Code {groups}"),
            format!("Setup ({groups}) Done"),
            format!("Copy\n{lined}\nthen"),
            format!("Code - {groups} - Here"),
        ] {
            let located = locate(&text).unwrap_or_else(|| panic!("no key in {text:?}"));
            let (stored, _) = normalize(&located, &config).unwrap();
            assert!(
                stored.as_str() == groups.replace(' ', ""),
                "{text:?} stored a different seed"
            );
            assert_eq!(code(&stored, &config, 59).unwrap(), "94287082", "{text:?}");
        }
    }

    #[test]
    fn a_mixed_case_grouped_key_is_captured_whole() {
        // RFC 6238 SHA-1 key, case-insensitive base32: the first four groups alone would also be
        // a valid 16-character seed, so only the whole element is the key that is shown.
        let shown = "GEZD GNBV GY3T QOJQ gezd gnbv gy3t qojq";
        let config = TotpConfig {
            digits: 8,
            ..TotpConfig::default()
        };
        let located = locate(shown).expect("the whole key is shown");
        let (stored, _) = normalize(&located, &config).unwrap();
        assert!(stored.as_str() == "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(code(&stored, &config, 59).unwrap(), "94287082");
    }

    #[test]
    fn capture_stores_the_key_as_shown_or_nothing() {
        let keys: [&[u8]; 4] = [
            b"1234567890",
            b"12345678901",
            b"123456789012345",
            b"12345678901234567890",
        ];
        let labels: [(&str, &str, bool); 9] = [
            ("", "", true),
            ("Seed ", "", true),
            ("", " Code", true),
            ("Your seed: ", "", true),
            ("Key: ", ".", true),
            ("(", ")", true),
            ("SEED ", "", false),
            ("", " CODE", false),
            ("SEED ", " CODE", false),
        ];
        let mut captured = 0;
        for bytes in keys {
            let padded = BASE32.encode(bytes);
            let plain = BASE32_NOPAD.encode(bytes);
            let groups = |key: &str| {
                key.as_bytes()
                    .chunks(4)
                    .map(|group| std::str::from_utf8(group).unwrap().to_string())
                    .collect::<Vec<_>>()
            };
            let mixed_halves = {
                let groups = groups(&plain);
                let half = groups.len() / 2;
                groups
                    .iter()
                    .enumerate()
                    .map(|(index, group)| {
                        if index < half {
                            group.clone()
                        } else {
                            group.to_ascii_lowercase()
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let alternating = groups(&plain)
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    if index % 2 == 0 {
                        group.clone()
                    } else {
                        group.to_ascii_lowercase()
                    }
                })
                .collect::<Vec<_>>();
            let forms: [(&str, String, bool); 8] = [
                ("upper", groups(&plain).join(" "), false),
                (
                    "lower",
                    groups(&plain.to_ascii_lowercase()).join(" "),
                    false,
                ),
                ("mixed halves", mixed_halves.join(" "), true),
                ("alternating", alternating.join(" "), true),
                ("dashed", groups(&plain).join("-"), false),
                ("line-broken", groups(&plain).join("\n"), false),
                ("padded", groups(&padded).join(" "), false),
                (
                    "lower padded",
                    groups(&padded.to_ascii_lowercase()).join(" "),
                    false,
                ),
            ];
            for (name, body, mixed_case) in forms {
                let shown_len = if name.contains("padded") {
                    padded.len()
                } else {
                    plain.len()
                };
                for (before, after, unambiguous) in labels {
                    let text = format!("{before}{body}{after}");
                    let Some(located) = locate(&text) else {
                        // A refusal stores nothing; only an unambiguous display may not refuse.
                        // Mixed case has no unambiguous edge inside prose.
                        let bare = before.is_empty() && after.is_empty();
                        let must_capture = unambiguous
                            && shown_len % 8 == 0
                            && (!mixed_case || (bare && name == "mixed halves"))
                            && !(bare
                                && !mixed_case
                                && shown_len > 16
                                && !(name.contains("padded") && padded.contains('=')));
                        assert!(!must_capture, "{name} {before:?}/{after:?} refused");
                        continue;
                    };
                    captured += 1;
                    let (stored, _) = normalize(&located, &TotpConfig::default()).unwrap();
                    assert!(
                        stored.as_str() == plain,
                        "{name} {before:?}/{after:?} stored a different seed"
                    );
                }
            }
        }
        assert!(captured > 100, "{captured} captures");
    }

    #[test]
    fn rfc6238_vectors() {
        // Appendix B uses different-length keys for each hash, including time > 2038.
        let times = [
            59,
            1_111_111_109,
            1_111_111_111,
            1_234_567_890,
            2_000_000_000,
            20_000_000_000,
        ];
        let vectors = [
            (
                TotpAlgorithm::Sha1,
                "12345678901234567890",
                [
                    "94287082", "07081804", "14050471", "89005924", "69279037", "65353130",
                ],
            ),
            (
                TotpAlgorithm::Sha256,
                "12345678901234567890123456789012",
                [
                    "46119246", "68084774", "67062674", "91819424", "90698825", "77737706",
                ],
            ),
            (
                TotpAlgorithm::Sha512,
                "1234567890123456789012345678901234567890123456789012345678901234",
                [
                    "90693936", "25091201", "99943326", "93441116", "38618901", "47863826",
                ],
            ),
        ];
        for (algorithm, key, expected) in vectors {
            let config = TotpConfig {
                algorithm,
                digits: 8,
                period_s: 30,
            };
            for (time, expected) in times.into_iter().zip(expected) {
                assert!(
                    code(&BASE32_NOPAD.encode(key.as_bytes()), &config, time).unwrap() == expected,
                    "RFC 6238 vector at {time}, {algorithm:?}"
                );
            }
        }
    }

    #[test]
    fn manual_key_and_uri_normalization() {
        let (seed, config) = normalize(
            " gezd gnbv gy3t qojq gezd gnbv gy3t qojq ",
            &TotpConfig::default(),
        )
        .unwrap();
        assert!(code(&seed, &config, 59).unwrap() == "287082");
        let (uri_seed, uri_config) = normalize("otpauth://totp/Example:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example&digits=8&period=60&algorithm=SHA256", &config).unwrap();
        assert!(*seed == *uri_seed);
        assert_eq!(
            uri_config,
            TotpConfig {
                algorithm: TotpAlgorithm::Sha256,
                digits: 8,
                period_s: 60
            }
        );
        assert!(code(&seed, &config, 60).unwrap() != code(&seed, &config, 59).unwrap());
    }

    #[test]
    fn invalid_keys_and_parameters_never_echo_input() {
        for value in [
            "invalid-private-value",
            "AAAA",
            "otpauth://hotp/service?secret=PRIVATE",
            "otpauth://totp/service?secret=PRIVATE&secret=PRIVATE",
            "otpauth://totp/service?secret=PRIVATE&algorithm=MD5",
            "otpauth://totp/service?secret=PRIVATE&period=0",
            "otpauth://totp/service?secret=PRIVATE&digits=9",
        ] {
            let error = normalize(value, &TotpConfig::default())
                .unwrap_err()
                .to_string();
            assert!(!error.contains(value) && !error.contains("PRIVATE"));
        }
    }
}
