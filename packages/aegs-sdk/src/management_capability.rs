use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use serde_json::Value;

const CLOCK_SKEW_SECONDS: u64 = 30;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManagementCapabilityClaims {
    pub(crate) issuer: String,
    pub(crate) audience: String,
    pub(crate) subject: String,
    pub(crate) generator_id: String,
    pub(crate) manifest_digest: String,
    pub(crate) management_url: String,
    pub(crate) owner_ids: Vec<String>,
    pub(crate) user_id: String,
    pub(crate) issued_at: u64,
    pub(crate) expires_at: u64,
    pub(crate) token_id: String,
}

pub(crate) fn parse_public_key(value: &str) -> Result<VerifyingKey, String> {
    let value = value.trim();
    let bytes = if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid management public key hex: {error}"))?
    } else {
        URL_SAFE_NO_PAD
            .decode(value)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(value))
            .map_err(|error| format!("management public key must be raw 32-byte base64: {error}"))?
    };
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "management public key must contain exactly 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|error| format!("invalid management public key: {error}"))
}

pub(crate) fn verify_management_capability(
    token: &str,
    public_key: &VerifyingKey,
    expected_issuer: &str,
    expected_generator_id: &str,
    now_seconds: u64,
) -> Result<ManagementCapabilityClaims, String> {
    verify_management_capability_scoped(
        token,
        public_key,
        expected_issuer,
        expected_generator_id,
        now_seconds,
        None,
        None,
    )
}

pub(crate) fn verify_management_capability_scoped(
    token: &str,
    public_key: &VerifyingKey,
    expected_issuer: &str,
    expected_generator_id: &str,
    now_seconds: u64,
    expected_management_url: Option<&str>,
    expected_manifest_digest: Option<&str>,
) -> Result<ManagementCapabilityClaims, String> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err("management capability must contain three segments".to_string());
    }
    let header: Value = decode_json(parts[0], "management capability header")?;
    let payload: Value = decode_json(parts[1], "management capability payload")?;
    if header.get("alg").and_then(Value::as_str) != Some("EdDSA")
        || header.get("typ").and_then(Value::as_str) != Some("JWT")
    {
        return Err("management capability uses an unsupported algorithm".to_string());
    }
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|error| format!("management capability signature is not base64url: {error}"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| format!("management capability signature is invalid: {error}"))?;
    public_key
        .verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .map_err(|_| "management capability signature verification failed".to_string())?;
    let claims = ManagementCapabilityClaims {
        issuer: required_string(&payload, "iss")?,
        audience: required_string(&payload, "aud")?,
        subject: required_string(&payload, "sub")?,
        generator_id: required_string(&payload, "generator_id")?,
        manifest_digest: required_string(&payload, "manifest_digest")?,
        management_url: required_string(&payload, "management_url")?,
        owner_ids: required_string_array(&payload, "owner_ids")?,
        user_id: required_string(&payload, "user_id")?,
        issued_at: required_u64(&payload, "iat")?,
        expires_at: required_u64(&payload, "exp")?,
        token_id: required_string(&payload, "jti")?,
    };
    if claims.issuer != expected_issuer {
        return Err("management capability issuer is not trusted".to_string());
    }
    if claims.audience != "aegs-management" || claims.generator_id != expected_generator_id {
        return Err("management capability audience is not valid for this AEGS".to_string());
    }
    if expected_management_url.is_some_and(|expected| expected != claims.management_url) {
        return Err("management capability endpoint scope is not trusted".to_string());
    }
    if expected_manifest_digest.is_some_and(|expected| expected != claims.manifest_digest) {
        return Err("management capability manifest scope is not trusted".to_string());
    }
    if claims.expires_at <= now_seconds || claims.expires_at <= claims.issued_at {
        return Err("management capability is expired".to_string());
    }
    if claims.issued_at > now_seconds.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err("management capability was issued in the future".to_string());
    }
    Ok(claims)
}

fn decode_json(segment: &str, name: &str) -> Result<Value, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|error| format!("{name} is not base64url: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{name} is invalid JSON: {error}"))
}

fn required_string(value: &Value, name: &str) -> Result<String, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("management capability claim {name} is required"))
}

fn required_u64(value: &Value, name: &str) -> Result<u64, String> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("management capability claim {name} is required"))
}

fn required_string_array(value: &Value, name: &str) -> Result<Vec<String>, String> {
    let owners = value
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("management capability claim {name} is required"))?
        .iter()
        .map(|owner| {
            owner
                .as_str()
                .filter(|owner| !owner.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("management capability claim {name} must contain strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if owners.is_empty() {
        return Err(format!(
            "management capability claim {name} must not be empty"
        ));
    }
    Ok(owners)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn token(signing_key: &SigningKey, expires_at: u64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT","kid":"test"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "iss": "chariox-cloud",
                "aud": "aegs-management",
                "sub": "kernel-1",
                "generator_id": "dev.chariox.slack",
                "manifest_digest": "sha256:abc",
                "management_url": "https://aegs.example.test",
                "owner_ids": ["owner-1", "kernel-1"],
                "user_id": "user-1",
                "iat": 100,
                "exp": expires_at,
                "jti": "cap-1"
            }))
            .unwrap(),
        );
        let unsigned = format!("{header}.{payload}");
        let signature = signing_key.sign(unsigned.as_bytes());
        format!(
            "{unsigned}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        )
    }

    #[test]
    fn verifies_generator_scoped_capability() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let capability = verify_management_capability(
            &token(&signing_key, 200),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.slack",
            150,
        )
        .unwrap();
        assert_eq!(capability.subject, "kernel-1");
        assert_eq!(capability.manifest_digest, "sha256:abc");
        assert_eq!(capability.owner_ids, ["owner-1", "kernel-1"]);
    }

    #[test]
    fn rejects_expired_or_wrong_generator_capability() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let expired = verify_management_capability(
            &token(&signing_key, 150),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.slack",
            150,
        );
        assert!(expired.is_err());
        let wrong_generator = verify_management_capability(
            &token(&signing_key, 200),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.github",
            150,
        );
        assert!(wrong_generator.is_err());
    }

    #[test]
    fn enforces_management_endpoint_and_manifest_scopes() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        assert!(verify_management_capability_scoped(
            &token(&signing_key, 200),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.slack",
            150,
            Some("https://aegs.example.test"),
            Some("sha256:abc"),
        )
        .is_ok());
        assert!(verify_management_capability_scoped(
            &token(&signing_key, 200),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.slack",
            150,
            Some("https://other.example.test"),
            Some("sha256:abc"),
        )
        .unwrap_err()
        .contains("endpoint scope"));
        assert!(verify_management_capability_scoped(
            &token(&signing_key, 200),
            &signing_key.verifying_key(),
            "chariox-cloud",
            "dev.chariox.slack",
            150,
            Some("https://aegs.example.test"),
            Some("sha256:other"),
        )
        .unwrap_err()
        .contains("manifest scope"));
    }
}
