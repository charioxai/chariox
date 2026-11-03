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
/// URI, one long token, or four or more equal-size groups (`ABCD-EFGH-…`). The whole text never
/// counts as a key, because lenient base32 would accept a plain sentence.
/// Sentences of unequal words never form a key. Returns what to store, not yet normalized.
pub(crate) fn locate(text: &str) -> Option<Zeroizing<String>> {
    let defaults = TotpConfig::default();
    let valid = |candidate: &str| normalize(candidate, &defaults).is_ok();
    let found = |candidate: &str| Some(Zeroizing::new(candidate.to_string()));
    if let Some(start) = text.find("otpauth://") {
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
            .map_or(text.len(), |length| start + length);
        if valid(&text[start..end]) {
            return found(&text[start..end]);
        }
    }
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '='))
        .filter(|token| !token.is_empty())
        .collect();
    let base32_only = |token: &str| {
        token
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || (b'2'..=b'7').contains(&b))
    };
    // A real key mixes in digits or is all capitals; a lowercase or capitalized word is prose.
    let looks_like_key = |token: &str| {
        token.bytes().any(|b| b.is_ascii_digit()) || !token.bytes().any(|b| b.is_ascii_lowercase())
    };
    let single = tokens
        .iter()
        .filter(|token| {
            token.len() >= 16 && base32_only(token) && looks_like_key(token) && valid(token)
        })
        .max_by_key(|token| token.len());
    if let Some(token) = single {
        return found(token);
    }
    let mut best: Option<String> = None;
    for start in 0..tokens.len() {
        let size = tokens[start].len();
        if !(2..=8).contains(&size) || !base32_only(tokens[start]) {
            continue;
        }
        let mut joined = String::new();
        let mut groups = 0;
        for token in &tokens[start..] {
            if token.len() != size || !base32_only(token) {
                break;
            }
            joined.push_str(token);
            groups += 1;
            if groups >= 4
                && joined.len() >= 16
                && valid(&joined)
                && best.as_ref().is_none_or(|best| joined.len() > best.len())
            {
                best = Some(joined.clone());
            }
        }
    }
    best.map(Zeroizing::new)
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
