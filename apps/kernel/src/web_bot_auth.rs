//! Ed25519 HTTP Message Signatures for Web Bot Auth requests.
//!
//! The wire format follows draft-ietf-webbotauth-httpsig-protocol-01 and RFC 9421.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::error::DaemonError;

const MAX_KEY_LIFETIME_SECS: u64 = 24 * 60 * 60;
const MAX_SIGNATURE_LIFETIME_SECS: u64 = 5 * 60;
const DIRECTORY_PATH: &str = "/.well-known/http-message-signatures-directory";

/// Public Ed25519 key material suitable for a Web Bot Auth JWK Set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicJwk {
    pub kty: String,
    pub crv: String,
    pub kid: String,
    pub x: String,
    #[serde(rename = "use")]
    pub public_use: String,
    pub alg: String,
    pub nbf: u64,
    pub exp: u64,
}

#[derive(Serialize)]
struct JwkSet<'a> {
    keys: [&'a PublicJwk; 1],
}

/// A signer whose private seed is neither serializable nor printable.
pub struct WebBotAuthSigner {
    seed: Zeroizing<[u8; 32]>,
    signature_agent_origin: String,
    not_before: u64,
    not_after: u64,
    jwk: PublicJwk,
}

impl WebBotAuthSigner {
    /// Creates a signer from a 32-byte Ed25519 seed and bounded key validity.
    pub fn from_seed(
        seed: [u8; 32],
        signature_agent_origin: &str,
        not_before: u64,
        not_after: u64,
    ) -> Result<Self, DaemonError> {
        let seed = Zeroizing::new(seed);
        validate_validity(not_before, not_after)?;
        let signature_agent_origin = validate_origin(signature_agent_origin)?;
        let signing_key = SigningKey::from_bytes(&seed);
        let public_key = signing_key.verifying_key();
        let jwk = make_jwk(&public_key, not_before, not_after);
        Ok(Self {
            seed,
            signature_agent_origin,
            not_before,
            not_after,
            jwk,
        })
    }

    /// Creates a self-hosted signer using fresh operating-system randomness.
    pub fn generate(
        signature_agent_origin: &str,
        not_before: u64,
        not_after: u64,
    ) -> Result<Self, DaemonError> {
        let mut seed = [0_u8; 32];
        OsRng.fill_bytes(&mut seed);
        Self::from_seed(seed, signature_agent_origin, not_before, not_after)
    }

    pub fn kid(&self) -> &str {
        &self.jwk.kid
    }

    pub(crate) fn private_key_base64url(&self) -> Zeroizing<String> {
        Zeroizing::new(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.seed[..]))
    }

    pub(crate) fn signature_agent_origin(&self) -> &str {
        &self.signature_agent_origin
    }

    fn signature_agent_url(&self) -> String {
        format!("{}{DIRECTORY_PATH}", self.signature_agent_origin)
    }

    pub fn public_jwk(&self) -> &PublicJwk {
        &self.jwk
    }

    /// Returns the compact JWK Set bytes served at the signature-agent URL.
    pub fn directory_json(&self) -> Result<Vec<u8>, DaemonError> {
        serde_json::to_vec(&JwkSet { keys: [&self.jwk] }).map_err(|_| internal_error())
    }

    /// Signs a directory response against the authority that requested it.
    pub fn signed_directory_headers(
        &self,
        request_authority: &str,
        body: &[u8],
        now_s: u64,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        if now_s < self.not_before || now_s >= self.not_after {
            return Err(invalid_request(
                "the signing key is outside its validity period",
            ));
        }
        let authority = validate_authority(request_authority)?;
        let expires = now_s
            .saturating_add(MAX_SIGNATURE_LIFETIME_SECS)
            .min(self.not_after);
        if expires <= now_s {
            return Err(invalid_request(
                "the signing key expires before a signature can be made",
            ));
        }
        let digest = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(body)));
        let params = format!(
            "(\"@authority\";req \"content-digest\");created={now_s};expires={expires};keyid=\"{}\";tag=\"http-message-signatures-directory\"",
            self.jwk.kid,
        );
        let base = format!(
            "\"@authority\";req: {authority}\n\"content-digest\": {digest}\n\"@signature-params\": {params}"
        );
        let signing_key = SigningKey::from_bytes(&self.seed);
        let signature = signing_key.sign(base.as_bytes());
        drop(signing_key);

        let mut headers = BTreeMap::new();
        headers.insert("Content-Digest".to_owned(), digest);
        headers.insert("Signature-Input".to_owned(), format!("binding={params}"));
        headers.insert(
            "Signature".to_owned(),
            format!("binding=:{}:", STANDARD.encode(signature.to_bytes())),
        );
        Ok(headers)
    }

    /// Signs a request and returns the three Web Bot Auth headers plus a
    /// Content-Digest header when a body was supplied.
    pub fn sign_request(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        now_s: u64,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        if now_s < self.not_before || now_s >= self.not_after {
            return Err(invalid_request(
                "the signing key is outside its validity period",
            ));
        }
        if !is_http_token(method) {
            return Err(invalid_request("the HTTP method is invalid"));
        }
        let target =
            url::Url::parse(url).map_err(|_| invalid_request("the request URL is invalid"))?;
        validate_request_url(&target)?;
        let target_uri = target_without_fragment(&target);
        let authority = authority(&target)?;
        let expires = now_s
            .saturating_add(MAX_SIGNATURE_LIFETIME_SECS)
            .min(self.not_after);
        if expires <= now_s {
            return Err(invalid_request(
                "the signing key expires before a signature can be made",
            ));
        }

        let mut nonce_bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = STANDARD.encode(nonce_bytes);
        nonce_bytes.zeroize();

        let content_digest = body.map(|bytes| {
            let digest = Sha256::digest(bytes);
            format!("sha-256=:{}:", STANDARD.encode(digest))
        });
        let signature_agent_url = self.signature_agent_url();

        let mut components = vec![
            "\"@method\"".to_owned(),
            "\"@target-uri\"".to_owned(),
            "\"@authority\"".to_owned(),
            "\"signature-agent\";key=\"sig1\"".to_owned(),
        ];
        if content_digest.is_some() {
            components.push("\"content-digest\"".to_owned());
        }
        let params = format!(
            "({});created={now_s};expires={expires};keyid=\"{}\";alg=\"ed25519\";nonce=\"{nonce}\";tag=\"web-bot-auth\"",
            components.join(" "),
            self.jwk.kid,
        );

        let mut base = format!(
            "\"@method\": {method}\n\"@target-uri\": {target_uri}\n\"@authority\": {authority}\n\"signature-agent\";key=\"sig1\": \"{}\"",
            signature_agent_url,
        );
        if let Some(digest) = &content_digest {
            base.push_str(&format!("\n\"content-digest\": {digest}"));
        }
        base.push_str(&format!("\n\"@signature-params\": {params}"));

        let signing_key = SigningKey::from_bytes(&self.seed);
        let signature = signing_key.sign(base.as_bytes());
        drop(signing_key);

        let mut headers = BTreeMap::new();
        headers.insert(
            "Signature-Agent".to_owned(),
            format!("sig1=\"{signature_agent_url}\""),
        );
        headers.insert("Signature-Input".to_owned(), format!("sig1={params}"));
        headers.insert(
            "Signature".to_owned(),
            format!("sig1=:{}:", STANDARD.encode(signature.to_bytes())),
        );
        if let Some(digest) = content_digest {
            headers.insert("Content-Digest".to_owned(), digest);
        }
        Ok(headers)
    }
}

fn make_jwk(key: &VerifyingKey, not_before: u64, not_after: u64) -> PublicJwk {
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_bytes());
    let kid = ed25519_thumbprint(&x);
    PublicJwk {
        kty: "OKP".into(),
        crv: "Ed25519".into(),
        kid,
        x,
        public_use: "sig".into(),
        alg: "EdDSA".into(),
        nbf: not_before,
        exp: not_after,
    }
}

fn ed25519_thumbprint(x: &str) -> String {
    let canonical = format!("{{\"crv\":\"Ed25519\",\"kty\":\"OKP\",\"x\":\"{x}\"}}");
    let thumbprint = Sha256::digest(canonical.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(thumbprint)
}

fn validate_validity(not_before: u64, not_after: u64) -> Result<(), DaemonError> {
    if not_after <= not_before || not_after - not_before > MAX_KEY_LIFETIME_SECS {
        return Err(invalid_request(
            "key validity must be positive and no longer than 24 hours",
        ));
    }
    Ok(())
}

pub(crate) fn validate_origin(value: &str) -> Result<String, DaemonError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| invalid_request("the signature-agent origin is invalid"))?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
        || (parsed.scheme() == "http" && !is_loopback(&parsed))
    {
        return Err(invalid_request(
            "the signature-agent must be an HTTPS origin (HTTP is allowed only on loopback)",
        ));
    }
    Ok(parsed.origin().ascii_serialization())
}

fn validate_request_url(target: &url::Url) -> Result<(), DaemonError> {
    if !matches!(target.scheme(), "https" | "http")
        || target.username() != ""
        || target.password().is_some()
        || (target.scheme() == "http" && !is_loopback(target))
    {
        return Err(invalid_request("signed requests require HTTPS (HTTP is allowed only on loopback) and cannot contain credentials"));
    }
    Ok(())
}

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn target_without_fragment(url: &url::Url) -> String {
    let mut target = url.clone();
    target.set_fragment(None);
    target.to_string()
}

fn authority(url: &url::Url) -> Result<String, DaemonError> {
    let host = url
        .host()
        .ok_or_else(|| invalid_request("the request URL has no host"))?;
    let mut value = match host {
        url::Host::Domain(domain) => domain.to_ascii_lowercase(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => format!("[{ip}]"),
    };
    if let Some(port) = url.port() {
        value.push(':');
        value.push_str(&port.to_string());
    }
    Ok(value)
}

fn validate_authority(value: &str) -> Result<String, DaemonError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'?' | b'#' | b'@'))
    {
        return Err(invalid_request("the request authority is invalid"));
    }
    let target = url::Url::parse(&format!("https://{value}/"))
        .map_err(|_| invalid_request("the request authority is invalid"))?;
    if target.username() != "" || target.password().is_some() {
        return Err(invalid_request("the request authority is invalid"));
    }
    let normalized = authority(&target)?;
    if normalized != value.to_ascii_lowercase() {
        return Err(invalid_request("the request authority is invalid"));
    }
    Ok(normalized)
}

fn is_http_token(method: &str) -> bool {
    !method.is_empty()
        && method.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn invalid_request(message: &'static str) -> DaemonError {
    DaemonError::MachineRequest {
        code: "web_bot_auth_invalid_request".to_owned(),
        message: message.to_owned(),
    }
}

fn internal_error() -> DaemonError {
    DaemonError::InternalInvariant {
        operation: "web_bot_auth_directory_serialization",
        message: "public key directory could not be serialized".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ed25519_dalek::{Signature, Verifier};

    fn signer() -> WebBotAuthSigner {
        WebBotAuthSigner::from_seed([7; 32], "https://operator.example", 100, 20_000).unwrap()
    }

    fn verify(
        headers: &BTreeMap<String, String>,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> bool {
        let Some(agent) = headers["Signature-Agent"]
            .strip_prefix("sig1=\"")
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        let input = headers["Signature-Input"].strip_prefix("sig1=").unwrap();
        let start = input.find('(').unwrap();
        let end = input.find(')').unwrap();
        let component_text = &input[start + 1..end];
        let components: Vec<&str> = component_text.split(' ').collect();
        let target = url::Url::parse(url).unwrap();
        let mut base = String::new();
        for (i, component) in components.iter().enumerate() {
            if i > 0 {
                base.push('\n');
            }
            let value = match *component {
                "\"@method\"" => method.to_owned(),
                "\"@target-uri\"" => target_without_fragment(&target),
                "\"@authority\"" => authority(&target).unwrap(),
                "\"signature-agent\";key=\"sig1\"" => format!("\"{agent}\""),
                "\"content-digest\"" => headers["Content-Digest"].clone(),
                _ => return false,
            };
            base.push_str(component);
            base.push_str(": ");
            base.push_str(&value);
        }
        base.push_str("\n\"@signature-params\": ");
        base.push_str(input);
        let sig_value = headers["Signature"]
            .strip_prefix("sig1=:")
            .unwrap()
            .strip_suffix(':')
            .unwrap();
        let sig_bytes = STANDARD.decode(sig_value).unwrap();
        let signature = Signature::from_slice(&sig_bytes).unwrap();
        let jwk = serde_json::from_slice::<serde_json::Value>(&signer().directory_json().unwrap())
            .unwrap();
        let x = URL_SAFE_NO_PAD
            .decode(jwk["keys"][0]["x"].as_str().unwrap())
            .unwrap();
        let public = VerifyingKey::from_bytes(x.as_slice().try_into().unwrap()).unwrap();
        public.verify(base.as_bytes(), &signature).is_ok()
            && body.is_none_or(|bytes| {
                headers["Content-Digest"]
                    == format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(bytes)))
            })
    }

    #[test]
    fn signs_and_verifies_requests_and_binds_body_and_url() {
        let signer = signer();
        let url = "https://example.com/a?b=1#ignored";
        let body = b"request body";
        let headers = signer.sign_request("POST", url, Some(body), 500).unwrap();
        assert_eq!(
            headers["Signature-Agent"],
            "sig1=\"https://operator.example/.well-known/http-message-signatures-directory\""
        );
        assert!(verify(&headers, "POST", url, Some(body)));
        assert!(!verify(&headers, "GET", url, Some(body)));
        assert!(!verify(
            &headers,
            "POST",
            "https://example.com/changed?b=1",
            Some(body)
        ));
        assert!(!verify(&headers, "POST", url, Some(b"tampered")));
        let mut tampered_agent = headers.clone();
        tampered_agent.insert(
            "Signature-Agent".to_owned(),
            "sig1=\"https://other.example\"".to_owned(),
        );
        assert!(!verify(&tampered_agent, "POST", url, Some(body)));
    }

    #[test]
    fn uses_rfc7638_thumbprint_and_publishes_key_validity() {
        let signer =
            WebBotAuthSigner::from_seed([7; 32], "https://operator.example", 100, 200).unwrap();
        assert_eq!(signer.public_jwk().kty, "OKP");
        assert_eq!(signer.public_jwk().crv, "Ed25519");
        assert_eq!(signer.public_jwk().alg, "EdDSA");
        assert_eq!(signer.public_jwk().nbf, 100);
        assert_eq!(signer.public_jwk().exp, 200);
        let json = signer.directory_json().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["keys"][0]["kid"], signer.kid());
        assert_eq!(value["keys"][0]["nbf"], 100);
        assert_eq!(value["keys"][0]["exp"], 200);
        let canonical = format!(
            "{{\"crv\":\"Ed25519\",\"kty\":\"OKP\",\"x\":\"{}\"}}",
            signer.public_jwk().x
        );
        assert_eq!(
            signer.kid(),
            URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()))
        );
    }

    #[test]
    fn matches_the_web_bot_auth_ed25519_thumbprint_vector() {
        assert_eq!(
            ed25519_thumbprint("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"),
            "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U"
        );
    }

    #[test]
    fn signs_directory_body_and_request_authority() {
        let signer = signer();
        let body = signer.directory_json().unwrap();
        let headers = signer
            .signed_directory_headers("operator.example", &body, 500)
            .unwrap();
        let digest = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(&body)));
        assert_eq!(headers["Content-Digest"], digest);

        let params = headers["Signature-Input"].strip_prefix("binding=").unwrap();
        let base = format!(
            "\"@authority\";req: operator.example\n\"content-digest\": {digest}\n\"@signature-params\": {params}"
        );
        let encoded = headers["Signature"]
            .strip_prefix("binding=:")
            .unwrap()
            .strip_suffix(':')
            .unwrap();
        let signature = Signature::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        assert!(signing_key
            .verifying_key()
            .verify(base.as_bytes(), &signature)
            .is_ok());
        assert!(signer
            .signed_directory_headers("operator.example/elsewhere", &body, 500)
            .is_err());
    }

    #[test]
    fn rejects_invalid_origins_urls_and_expired_keys() {
        assert!(
            WebBotAuthSigner::from_seed([1; 32], "https://operator.example/path", 1, 2).is_err()
        );
        assert!(WebBotAuthSigner::from_seed([1; 32], "http://operator.example", 1, 2).is_err());
        assert!(signer()
            .sign_request("GET", "http://example.com/", None, 500)
            .is_err());
        assert!(signer()
            .sign_request("GET", "http://localhost:8080/", None, 500)
            .is_ok());
        assert!(signer()
            .sign_request("GET", "https://example.com/", None, 20_000)
            .is_err());
        assert!(
            WebBotAuthSigner::from_seed([1; 32], "https://operator.example", 1, 86_402).is_err()
        );
    }
}
