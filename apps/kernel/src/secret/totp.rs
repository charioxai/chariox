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
/// captured element is not always the bare key. Finds the key in such text: an `otpauth://` URI,
/// or the one run of base32 words that only spaces, dashes and line breaks join. A run ends at the
/// edge of the text, at punctuation or at a word with a non-base32 character; a base32 label word
/// ("Seed", "CODE") joined by a space is part of the run, so the run is no key unless it is one
/// unseparated word or has a standard secret size (16 or 32 characters), and a run before a colon
/// is a label ("Or enter the secret:"), never a key. Capture never stores a
/// seed other than the one shown: when the text shows no such run, or several, there is no key.
/// Returns what to store, not yet normalized.
pub(crate) fn locate(text: &str) -> Option<Zeroizing<String>> {
    let defaults = TotpConfig::default();
    if let Some(start) = text.find("otpauth://") {
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
            .map_or(text.len(), |length| start + length);
        if normalize(&text[start..end], &defaults).is_ok() {
            return Some(Zeroizing::new(text[start..end].to_string()));
        }
    }
    let mut candidates = key_runs(text)
        .into_iter()
        .filter(|run| !run.label && (run.words == 1 || matches!(run.key.len(), 16 | 32)))
        .filter(|run| normalize(&run.key, &defaults).is_ok())
        .map(|run| run.key)
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    candidates.dedup_by(|a, b| a.as_str() == b.as_str());
    match candidates.len() {
        1 => candidates.pop(),
        _ => None,
    }
}

/// Base32 words joined only by spaces, dashes and line breaks, separators removed.
struct KeyRun {
    key: Zeroizing<String>,
    words: usize,
    /// A colon follows: the run names what comes next ("Or enter the secret:").
    label: bool,
}

/// Base32 in any letter case, with padding only at the end; a group may be padding alone.
fn key_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .trim_end_matches('=')
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || (b'2'..=b'7').contains(&b))
}

/// The maximal runs of key words in `text`. A word is a maximal run of alphanumeric characters
/// (`=` counts, for padding); anything between two words other than spaces, dashes and line
/// breaks ends a run, as does a word that is not base32.
fn key_runs(text: &str) -> Vec<KeyRun> {
    let joins = |gap: &str| gap.chars().all(|c| matches!(c, ' ' | '-' | '\n' | '\r'));
    let mut runs = Vec::new();
    let mut current: Option<KeyRun> = None;
    let mut gap_start = 0;
    for (word, start) in words(text) {
        let gap = &text[gap_start..start];
        gap_start = start + word.len();
        if let Some(run) = current.as_mut() {
            run.label = gap.trim_start().starts_with(':');
        }
        let joined = joins(gap);
        if !key_word(word) {
            runs.extend(current.take());
            continue;
        }
        match current.as_mut() {
            Some(run) if joined => {
                run.key.push_str(word);
                run.words += 1;
            }
            _ => {
                runs.extend(current.take());
                current = Some(KeyRun {
                    key: Zeroizing::new(word.to_string()),
                    words: 1,
                    label: false,
                });
            }
        }
    }
    if let Some(run) = current.as_mut() {
        run.label = text[gap_start..].trim_start().starts_with(':');
    }
    runs.extend(current);
    runs
}

/// The words of `text` with their byte offsets.
fn words(text: &str) -> impl Iterator<Item = (&str, usize)> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '='))
        .filter(|word| !word.is_empty())
        .map(move |word| (word, word.as_ptr() as usize - text.as_ptr() as usize))
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

    /// Fixed-seed xorshift, so a failing case is reproducible.
    struct Cases(u64);

    impl Cases {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }

        /// A base32 key of `bytes` random bytes, padded or not, with every letter's case flipped
        /// at random and separators between random-size groups.
        fn shown(&mut self, bytes: usize, padded: bool) -> (String, String) {
            let raw = (0..bytes).map(|_| self.next() as u8).collect::<Vec<_>>();
            let key = if padded {
                BASE32.encode(&raw)
            } else {
                BASE32_NOPAD.encode(&raw)
            };
            let mut shown = String::new();
            let group = [0, 4, 4, 8, 2 + self.below(6)][self.below(5)];
            for (index, c) in key.chars().enumerate() {
                if group > 0 && index > 0 && index % group == 0 {
                    shown.push_str([" ", "-", "\n", " - ", "\r\n"][self.below(5)]);
                }
                shown.push(if self.below(2) == 0 {
                    c.to_ascii_lowercase()
                } else {
                    c
                });
            }
            (BASE32_NOPAD.encode(&raw), shown)
        }
    }

    fn stored(text: &str) -> Option<Zeroizing<String>> {
        let located = locate(text)?;
        Some(normalize(&located, &TotpConfig::default()).unwrap().0)
    }

    #[test]
    fn a_mixed_case_grouped_key_is_captured_whole() {
        // RFC 6238 SHA-1 key, case-insensitive base32: the first four groups alone would also be
        // a valid 16-character seed, so only the whole element is the key that is shown.
        let config = TotpConfig {
            digits: 8,
            ..TotpConfig::default()
        };
        for shown in [
            "GEZD GNBV GY3T QOJQ gEzD gnbv gy3t qojq",
            "GEZD GNBV GY3T QOJQ gezd gnbv gy3t qojq",
        ] {
            let located = locate(shown).expect("the whole key is shown");
            let (stored, _) = normalize(&located, &config).unwrap();
            assert!(stored.as_str() == "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
            assert_eq!(code(&stored, &config, 59).unwrap(), "94287082");
        }
    }

    #[test]
    fn a_bare_key_is_captured_whole_in_any_case_and_grouping() {
        let mut cases = Cases(0x5eed_0001);
        for index in 0..300 {
            // 10 and 20 bytes are the 16- and 32-character keys; 16 bytes padded is 32 characters.
            let (bytes, padded) = [(10, false), (20, false), (16, true)][index % 3];
            let (key, shown) = cases.shown(bytes, padded);
            assert!(
                stored(&shown).is_some_and(|seed| *seed == key),
                "case {index}: a bare key was not captured whole"
            );
        }
    }

    #[test]
    fn a_key_inside_labels_is_captured_exactly_or_refused() {
        // (before, after, delimited): punctuation and words with a non-base32 character end a run;
        // base32 words joined by a space or dash are part of it, so the capture is refused.
        let labels: [(&str, &str, bool); 16] = [
            ("", "", true),
            ("Your seed: ", "", true),
            ("Key: ", ".", true),
            ("(", ")", true),
            ("Setup (", ") Done", true),
            ("Step 1 ", " 9", true),
            ("Seed: ", "\nThen type the code.", false),
            ("Seed ", "", false),
            ("", " code", false),
            ("SEED ", " CODE", false),
            ("seed ", "", false),
            ("Copy\n", "\nthen", false),
            ("Code - ", " - Here", false),
            ("Or enter the secret: ", "\nAnd enter the passcode:", false),
            ("Manual entry key ", " (spaces optional)", false),
            ("Key 2FA ", " and", false),
        ];
        let mut cases = Cases(0x5eed_0002);
        let mut captured = 0;
        for index in 0..300 {
            let (bytes, padded) = [(10, false), (20, false), (16, true)][index % 3];
            let (key, shown) = cases.shown(bytes, padded);
            for (before, after, delimited) in labels {
                let text = format!("{before}{shown}{after}");
                match stored(&text) {
                    Some(seed) => {
                        assert!(
                            *seed == key,
                            "case {index} {before:?}/{after:?}: another seed"
                        );
                        captured += 1;
                    }
                    None => assert!(!delimited, "case {index} {before:?}/{after:?}: refused"),
                }
            }
        }
        assert!(captured >= 300 * 6, "{captured} captures");
    }

    #[test]
    fn page_text_around_a_key_never_joins_the_seed() {
        // The RFC 6238 SHA-1 test key "12345678901234567890" shown as eight groups of four. Label
        // words have the length of a group, so only a delimiter keeps them out of the seed.
        let groups = "GEZD GNBV GY3T QOJQ GEZD GNBV GY3T QOJQ";
        let config = TotpConfig {
            digits: 8,
            ..TotpConfig::default()
        };
        let dashed = groups.replace(' ', "-");
        let lined = groups.replace(' ', "\n");
        let lower = groups.to_ascii_lowercase();
        for text in [
            format!("Your seed: {groups}"),
            format!("Key: {dashed}."),
            format!("Setup ({groups}) Done"),
            format!("Your secret key:\n{lined}"),
            format!("Enter this key: {lower}"),
        ] {
            let located = locate(&text).unwrap_or_else(|| panic!("no key in {text:?}"));
            let (stored, _) = normalize(&located, &config).unwrap();
            assert!(
                stored.as_str() == groups.replace(' ', ""),
                "{text:?} stored a different seed"
            );
            assert_eq!(code(&stored, &config, 59).unwrap(), "94287082", "{text:?}");
        }
        for text in [
            format!("Seed {groups} code"),
            format!("Code {groups}"),
            format!("Copy\n{lined}\nthen"),
            format!("Code - {groups} - Here"),
            format!("Your seed: {groups} and"),
            format!("Two keys: {groups}, JBSW Y3DP EHPK 3PXP"),
        ] {
            assert!(locate(&text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn locates_a_setup_key_in_page_text_and_never_in_prose() {
        let seed = "JBSWY3DPEHPK3PXP";
        for text in [
            seed.to_string(),
            seed.to_ascii_lowercase(),
            "jbsw y3dp ehpk 3pxp".to_string(),
            "Or enter the secret: jbsw y3dp ehpk 3pxp".to_string(),
            format!("Setup key: {}", "JBSW-Y3DP-EHPK-3PXP"),
            format!("Or enter the secret: {seed}.\nAnd enter the passcode shown:"),
            format!("Open otpauth://totp/Fixture:alice?secret={seed}&issuer=Fixture to add it"),
            format!("Schlüssel: {seed} öffnen"),
        ] {
            let located = locate(&text).unwrap_or_else(|| panic!("no key in {text:?}"));
            let (canonical, _) = normalize(&located, &TotpConfig::default()).unwrap();
            assert_eq!(canonical.as_str(), seed, "{text:?}");
        }
        for text in [
            "",
            "Or enter the secret:",
            "This page shows nothing that looks like a key at all",
            "Scan this image with your authentication application:",
            "This also gives game plan ideas around town",
        ] {
            assert!(locate(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn a_standard_size_run_is_the_key_as_shown() {
        // Documented limit: label words joined to a key by spaces only, adding up to a standard
        // size, cannot be told from key groups; the run is stored as shown.
        let shown = "Your secret code JBSW Y3DP EHPK 3PXP";
        assert!(stored(shown).is_none(), "30 characters is no standard size");
        let shown = "SEED CODE JBSW Y3DP EHPK 3PXP SEED CODE";
        assert!(stored(shown).is_some_and(|seed| seed.len() == 32));
        // The same holds for an element that shows only 16 base32 letters of prose.
        assert!(stored("Or enter the secret").is_some_and(|seed| seed.len() == 16));
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
