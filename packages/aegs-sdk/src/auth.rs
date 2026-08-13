use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use chariox_event_protocol::{
    AegsProviderResource, AegsProviderResourcePage, AegsProviderResourceQuery,
};

pub const AUTHORIZATION_TTL_MS: u64 = 10 * 60 * 1_000;
pub const PROVIDER_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct CredentialCipher {
    key: [u8; 32],
}

impl CredentialCipher {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let bytes = if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            (0..value.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "credential key is not valid hexadecimal".to_string())?
        } else {
            STANDARD.decode(value).map_err(|_| {
                "credential key must be 32-byte base64 or 64-character hex".to_string()
            })?
        };
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "credential key must contain exactly 32 bytes".to_string())?;
        Ok(Self { key })
    }

    pub fn encrypt<T: Serialize>(&self, value: &T) -> Result<Vec<u8>, String> {
        let plaintext = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| "credential cipher initialization failed".to_string())?;
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| "credential encryption failed".to_string())?;
        let mut envelope = nonce.to_vec();
        envelope.extend(ciphertext);
        Ok(envelope)
    }

    pub fn decrypt<T: for<'de> Deserialize<'de>>(&self, value: &[u8]) -> Result<T, String> {
        if value.len() < 13 {
            return Err("encrypted credential envelope is invalid".to_string());
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| "credential cipher initialization failed".to_string())?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&value[..12]), &value[12..])
            .map_err(|_| "credential decryption failed".to_string())?;
        serde_json::from_slice(&plaintext).map_err(|error| error.to_string())
    }
}

pub fn environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn parse_public_base_url(value: &str) -> Result<Url, String> {
    let url = parse_base_url(value)?;
    let safe = url.scheme() == "https"
        || (url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1")));
    if !safe {
        return Err("CHARIOX_AEGS_PUBLIC_BASE_URL must use HTTPS or loopback HTTP".to_string());
    }
    Ok(url)
}

pub fn parse_base_url(value: &str) -> Result<Url, String> {
    let mut url = parse_url(value)?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

pub fn parse_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("provider URL `{value}` must use HTTP or HTTPS"));
    }
    Ok(url)
}

pub fn random_opaque(prefix: &str) -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}-{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn verify_hmac_sha256_hex(body: &[u8], signature: &str, secret: &str) -> bool {
    let Ok(signature) = decode_hex(signature) else {
        return false;
    };
    <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).is_ok_and(|mut mac| {
        mac.update(body);
        mac.verify_slice(&signature).is_ok()
    })
}

pub fn sha256_occurrence_id(body: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(body))
}

pub fn decode_page(cursor: Option<&str>) -> Result<u32, String> {
    match cursor {
        None => Ok(1),
        Some(cursor) => cursor
            .strip_prefix("page:")
            .ok_or_else(|| "resource cursor is invalid".to_string())?
            .parse::<u32>()
            .ok()
            .filter(|page| *page > 0)
            .ok_or_else(|| "resource cursor is invalid".to_string()),
    }
}

pub fn filter_resources(
    mut page: AegsProviderResourcePage,
    query: &AegsProviderResourceQuery,
) -> AegsProviderResourcePage {
    if let Some(search) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let search = search.to_ascii_lowercase();
        page.resources.retain(|resource| {
            resource.name.to_ascii_lowercase().contains(&search)
                || resource
                    .connection_scope
                    .to_ascii_lowercase()
                    .contains(&search)
        });
    }
    page
}

pub fn slice_resources(
    resources: Vec<AegsProviderResource>,
    query: &AegsProviderResourceQuery,
) -> Result<AegsProviderResourcePage, String> {
    let page = decode_page(query.cursor.as_deref())?;
    let offset = (page.saturating_sub(1) as usize).saturating_mul(query.limit as usize);
    let end = offset
        .saturating_add(query.limit as usize)
        .min(resources.len());
    let values = if offset >= resources.len() {
        Vec::new()
    } else {
        resources[offset..end].to_vec()
    };
    Ok(AegsProviderResourcePage {
        resources: values,
        next_cursor: (end < resources.len()).then(|| format!("page:{}", page + 1)),
    })
}

pub fn bearer_json(request: ureq::Request, token: &str) -> Result<Value, String> {
    http_json(
        request
            .timeout(PROVIDER_HTTP_TIMEOUT)
            .set("accept", "application/json")
            .set("authorization", &format!("Bearer {token}"))
            .call(),
    )
}

pub fn http_json(response: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
    match response {
        Ok(response) => response
            .into_json::<Value>()
            .map_err(|error| format!("provider response is invalid JSON: {error}")),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!(
                "provider API returned HTTP {status}: {}",
                bounded(&body, 512)
            ))
        }
        Err(error) => Err(format!("provider API request failed: {error}")),
    }
}

pub fn http_empty(response: Result<ureq::Response, ureq::Error>) -> Result<(), String> {
    match response {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!(
                "provider API returned HTTP {status}: {}",
                bounded(&body, 512)
            ))
        }
        Err(error) => Err(format!("provider API request failed: {error}")),
    }
}

fn bounded(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn decode_hex(value: &str) -> Result<Vec<u8>, ()> {
    if !value.len().is_multiple_of(2) {
        return Err(());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(value: u8) -> Result<u8, ()> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_cipher_round_trips_without_exposing_plaintext() {
        let cipher = CredentialCipher::parse(&"ab".repeat(32)).unwrap();
        let encrypted = cipher
            .encrypt(&serde_json::json!({"token": "secret"}))
            .unwrap();
        assert!(!encrypted
            .windows(b"secret".len())
            .any(|window| window == b"secret"));
        let value: Value = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(value["token"], "secret");
    }
}
