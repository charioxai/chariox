//! Self-hosted relay admission without Cloud (M5.5 "Token Issuers"). The
//! operator token admits the home kernel; clients present short-lived tokens
//! that kernel signs with an Ed25519 key whose PUBLIC half is pinned here, so
//! the relay holds no secret that could mint a client token.

use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::{
    current_unix_ms, parse_cloud_jwt_claims, validate_claims, RelayAction, RelayAuthError,
    RelayAuthRequest, RelaySubjectKind, VerifiedRelayIdentity,
};

/// Environment value format: `<issuer>=<base64url public key>[,<issuer>=<key>...]`.
pub const KERNEL_ISSUER_KEYS_ENV: &str = "CHARIOX_RELAY_KERNEL_ISSUER_KEYS";

const KERNEL_TOKEN_ALG: &str = "EdDSA";

/// Stable short id of an issuer public key, carried as the token `kid`.
pub fn kernel_issuer_key_id(public_key: &VerifyingKey) -> String {
    Sha256::digest(public_key.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn encode_kernel_issuer_public_key(public_key: &VerifyingKey) -> String {
    URL_SAFE_NO_PAD.encode(public_key.as_bytes())
}

/// Pinned kernel issuer keys. Several keys per issuer cover a staged rotation.
#[derive(Debug, Clone, Default)]
pub struct KernelIssuerKeys {
    keys: BTreeMap<String, Vec<VerifyingKey>>,
}

impl KernelIssuerKeys {
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut keys = BTreeMap::<String, Vec<VerifyingKey>>::new();
        for entry in value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (issuer, key) = entry.rsplit_once('=').ok_or_else(|| {
                format!("{KERNEL_ISSUER_KEYS_ENV} entries must be <issuer>=<public key>")
            })?;
            let issuer = issuer.trim();
            if issuer.is_empty() {
                return Err(format!(
                    "{KERNEL_ISSUER_KEYS_ENV} has an entry without an issuer"
                ));
            }
            let bytes: [u8; 32] = URL_SAFE_NO_PAD
                .decode(key.trim())
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or_else(|| {
                    format!("{KERNEL_ISSUER_KEYS_ENV} key for `{issuer}` is not a base64url Ed25519 public key")
                })?;
            let key = VerifyingKey::from_bytes(&bytes).map_err(|_| {
                format!(
                    "{KERNEL_ISSUER_KEYS_ENV} key for `{issuer}` is not a valid Ed25519 public key"
                )
            })?;
            keys.entry(issuer.to_string()).or_default().push(key);
        }
        if keys.is_empty() {
            return Err(format!("{KERNEL_ISSUER_KEYS_ENV} lists no issuer keys"));
        }
        Ok(Self { keys })
    }

    fn key(&self, issuer: &str, key_id: &str) -> Option<&VerifyingKey> {
        self.keys
            .get(issuer)?
            .iter()
            .find(|key| kernel_issuer_key_id(key) == key_id)
    }
}

#[derive(Debug, Clone)]
pub struct SelfHostedVerifier {
    operator_token: String,
    kernel_issuers: KernelIssuerKeys,
    pub(super) now_ms: Option<u64>,
}

impl SelfHostedVerifier {
    pub fn new(
        operator_token: String,
        kernel_issuers: KernelIssuerKeys,
        now_ms: Option<u64>,
    ) -> Self {
        Self {
            operator_token,
            kernel_issuers,
            now_ms,
        }
    }

    pub fn verify(
        &self,
        request: RelayAuthRequest<'_>,
    ) -> Result<VerifiedRelayIdentity, RelayAuthError> {
        let operator: bool = self
            .operator_token
            .as_bytes()
            .ct_eq(request.token.as_bytes())
            .into();
        if operator {
            return Ok(VerifiedRelayIdentity::bootstrap(request.action));
        }
        let claims = decode_kernel_issued_token(request.token, &self.kernel_issuers)?;
        // A kernel issues CLIENT grants only; daemon and peer admission stay
        // with the operator token.
        if claims.subject_kind != RelaySubjectKind::Client {
            return Err(RelayAuthError::ActionNotAllowed);
        }
        validate_claims(
            &claims,
            request.action,
            request.target,
            Some(self.now_ms.unwrap_or_else(current_unix_ms)),
        )?;
        Ok(super::identity_from_claims(claims))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KernelTokenHeader {
    alg: String,
    typ: String,
    kid: String,
}

/// Claims a kernel issuer signs, in the hosted JWT claim shape the relay and
/// clients already read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelIssuedClientClaims {
    pub issuer: String,
    pub realm_id: String,
    pub client_id: String,
    pub allowed_actions: Vec<RelayAction>,
    pub target_daemon_id: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub token_id: String,
    pub public_key_thumbprint: Option<String>,
}

pub fn encode_kernel_issued_client_token(
    claims: &KernelIssuedClientClaims,
    signing_key: &SigningKey,
) -> Result<String, RelayAuthError> {
    let header = KernelTokenHeader {
        alg: KERNEL_TOKEN_ALG.into(),
        typ: "JWT".into(),
        kid: kernel_issuer_key_id(&signing_key.verifying_key()),
    };
    let payload = serde_json::json!({
        "iss": claims.issuer,
        "sub": claims.client_id,
        "subject_kind": RelaySubjectKind::Client,
        "realm_id": claims.realm_id,
        "allowed_actions": claims.allowed_actions.iter().map(|action| action_name(*action)).collect::<Vec<_>>(),
        "allowed_targets": [claims.target_daemon_id],
        "iat": claims.issued_at_ms / 1000,
        "exp": claims.expires_at_ms / 1000,
        "jti": claims.token_id,
        "client_id": claims.client_id,
        "public_key_thumbprint": claims.public_key_thumbprint,
    });
    let header = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&header).map_err(|_| RelayAuthError::InvalidToken)?);
    let payload = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).map_err(|_| RelayAuthError::InvalidToken)?);
    let signing_input = format!("{header}.{payload}");
    let signature = signing_key.sign(signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

fn decode_kernel_issued_token(
    token: &str,
    issuers: &KernelIssuerKeys,
) -> Result<super::RelayTokenClaims, RelayAuthError> {
    let mut parts = token.trim().split('.');
    let (Some(header_b64), Some(claims_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(RelayAuthError::InvalidToken);
    };
    let header = URL_SAFE_NO_PAD
        .decode(header_b64)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<KernelTokenHeader>(&bytes).ok())
        .ok_or(RelayAuthError::InvalidToken)?;
    if header.alg != KERNEL_TOKEN_ALG || header.typ != "JWT" {
        return Err(RelayAuthError::InvalidToken);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    let claims = parse_cloud_jwt_claims(&payload)?;
    let key = issuers
        .key(&claims.issuer, &header.kid)
        .ok_or(RelayAuthError::InvalidToken)?;
    let signature: [u8; 64] = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(RelayAuthError::InvalidToken)?;
    key.verify_strict(
        format!("{header_b64}.{claims_b64}").as_bytes(),
        &Signature::from_bytes(&signature),
    )
    .map_err(|_| RelayAuthError::InvalidToken)?;
    Ok(claims)
}

fn action_name(action: RelayAction) -> &'static str {
    match action {
        RelayAction::DaemonRegister => "daemon.register",
        RelayAction::DaemonHeartbeat => "daemon.heartbeat",
        RelayAction::ClientMetadataRead => "client.metadata.read",
        RelayAction::ClientConnect => "client.connect",
        RelayAction::PacketRoute => "packet.route",
        RelayAction::PeerRequest => "peer.request",
        RelayAction::PeerEvent => "peer.event",
    }
}

#[cfg(test)]
mod tests;
