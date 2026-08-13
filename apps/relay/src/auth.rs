use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

pub type RelayRealmId = String;
pub type RelayIssuerId = String;
pub type RelaySubjectId = String;

pub const DEFAULT_RELAY_REALM_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRealm {
    pub realm_id: RelayRealmId,
    pub issuer_id: RelayIssuerId,
    pub display_name: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelaySubjectKind {
    Client,
    Kernel,
    Machine,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayAction {
    DaemonRegister,
    DaemonHeartbeat,
    ClientMetadataRead,
    ClientConnect,
    PacketRoute,
    PeerRequest,
    PeerEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayTokenClaims {
    pub issuer: String,
    pub subject: RelaySubjectId,
    pub subject_kind: RelaySubjectKind,
    pub realm_id: RelayRealmId,
    pub allowed_actions: Vec<RelayAction>,
    pub allowed_targets: Option<Vec<String>>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub token_id: String,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
    pub user_id: Option<String>,
    pub device_id: Option<String>,
    pub machine_id: Option<String>,
    pub client_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub public_key_thumbprint: Option<String>,
    pub entitlements_version: Option<String>,
}

impl RelayTokenClaims {
    pub fn allows_action(&self, action: RelayAction) -> bool {
        self.allowed_actions.contains(&action)
    }

    pub fn allows_target(&self, target: Option<&str>) -> bool {
        match (self.allowed_targets.as_ref(), target) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(allowed), Some(target)) => allowed.iter().any(|candidate| candidate == target),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRelayIdentity {
    pub realm_id: RelayRealmId,
    pub subject: RelaySubjectId,
    pub subject_kind: RelaySubjectKind,
    pub allowed_actions: Vec<RelayAction>,
    pub allowed_targets: Option<Vec<String>>,
    pub expires_at_ms: u64,
    pub token_id: Option<String>,
    pub user_id: Option<String>,
    pub public_key_thumbprint: Option<String>,
}

impl VerifiedRelayIdentity {
    pub fn bootstrap(action: RelayAction) -> Self {
        let subject_kind = subject_kind_for_action(action);
        Self {
            realm_id: DEFAULT_RELAY_REALM_ID.to_string(),
            subject: "shared-token-bootstrap".to_string(),
            subject_kind,
            allowed_actions: vec![action],
            allowed_targets: None,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: (subject_kind == RelaySubjectKind::Client).then(|| "local".to_string()),
            public_key_thumbprint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayAuthRequest<'a> {
    pub token: &'a str,
    pub action: RelayAction,
    pub target: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelayAuthError {
    #[error("invalid relay token")]
    InvalidToken,
    #[error("relay token does not allow requested action")]
    ActionNotAllowed,
    #[error("relay token does not allow requested target")]
    TargetNotAllowed,
    #[error("relay token is expired")]
    TokenExpired,
    #[error("relay token has been revoked")]
    TokenRevoked,
    #[error("scoped relay tokens are not enabled")]
    ScopedTokensUnavailable,
}

/// Live, bounded denylist for scoped relay tokens. Cloud revocations are fed
/// in keyed by token id (`jti`) or account id; each entry carries the moment
/// past which it can be dropped (the underlying token would have expired
/// anyway), keeping the registry bounded by outstanding-token count.
#[derive(Debug, Clone, Default)]
pub struct RelayRevocationRegistry {
    inner: std::sync::Arc<std::sync::Mutex<RevocationState>>,
}

#[derive(Debug, Default)]
struct RevocationState {
    revoked_token_ids: BTreeMap<String, u64>,
    revoked_accounts: BTreeMap<String, u64>,
    // The hosted control plane revokes paired identities by their client or
    // machine subject id, so the registry mirrors those against a token's
    // client_id / machine_id claims.
    revoked_subjects: BTreeMap<String, u64>,
}

impl RelayRevocationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RevocationState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Revoke a specific token by its `jti`; `expires_at_ms` should be the
    /// token's own expiry so the entry can be pruned once it is moot.
    pub fn revoke_token_id(&self, token_id: impl Into<String>, expires_at_ms: u64) {
        self.lock()
            .revoked_token_ids
            .insert(token_id.into(), expires_at_ms);
    }

    /// Revoke every token bound to an account until `expires_at_ms` (use the
    /// furthest outstanding token expiry, or a bounded horizon).
    pub fn revoke_account(&self, account_id: impl Into<String>, expires_at_ms: u64) {
        self.lock()
            .revoked_accounts
            .insert(account_id.into(), expires_at_ms);
    }

    /// Revoke every token whose `client_id` or `machine_id` matches this
    /// subject id, mirroring how the hosted control plane revokes paired
    /// client/machine identities.
    pub fn revoke_subject(&self, subject: impl Into<String>, expires_at_ms: u64) {
        self.lock()
            .revoked_subjects
            .insert(subject.into(), expires_at_ms);
    }

    /// Drop entries whose expiry has passed so the registry stays bounded.
    pub fn prune(&self, now_ms: u64) {
        let mut state = self.lock();
        state
            .revoked_token_ids
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        state
            .revoked_accounts
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        state
            .revoked_subjects
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
    }

    fn is_revoked(&self, claims: &RelayTokenClaims, now_ms: u64) -> bool {
        let state = self.lock();
        let active = |map: &BTreeMap<String, u64>, key: &str| {
            map.get(key)
                .is_some_and(|expires_at_ms| *expires_at_ms > now_ms)
        };
        if active(&state.revoked_token_ids, &claims.token_id) {
            return true;
        }
        if claims
            .account_id
            .as_deref()
            .is_some_and(|account_id| active(&state.revoked_accounts, account_id))
        {
            return true;
        }
        [claims.client_id.as_deref(), claims.machine_id.as_deref()]
            .into_iter()
            .flatten()
            .any(|subject| active(&state.revoked_subjects, subject))
    }
}

#[derive(Debug, Clone)]
pub enum RelayAuthVerifier {
    SharedToken(SharedTokenVerifier),
    ScopedToken(ScopedTokenVerifier),
}

impl RelayAuthVerifier {
    pub fn shared(expected_token: Option<String>) -> Self {
        Self::SharedToken(SharedTokenVerifier::new(expected_token))
    }

    pub fn scoped_hmac(issuer_secrets: BTreeMap<String, String>, now_ms: Option<u64>) -> Self {
        Self::ScopedToken(ScopedTokenVerifier::new(
            BTreeMap::new(),
            issuer_secrets,
            now_ms,
        ))
    }

    /// Attach a live revocation registry to the scoped verifier so verification
    /// rejects revoked tokens. A no-op for the shared-token verifier, which has
    /// no per-token identity to revoke.
    pub fn with_revocations(self, revocations: RelayRevocationRegistry) -> Self {
        match self {
            Self::ScopedToken(verifier) => {
                Self::ScopedToken(verifier.with_revocations(revocations))
            }
            other => other,
        }
    }

    pub fn verify(
        &self,
        request: RelayAuthRequest<'_>,
    ) -> Result<VerifiedRelayIdentity, RelayAuthError> {
        match self {
            Self::SharedToken(verifier) => verifier.verify(request),
            Self::ScopedToken(verifier) => verifier.verify(request),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SharedTokenVerifier {
    expected_token: Option<String>,
}

impl SharedTokenVerifier {
    pub fn new(expected_token: Option<String>) -> Self {
        Self { expected_token }
    }

    pub fn verify(
        &self,
        request: RelayAuthRequest<'_>,
    ) -> Result<VerifiedRelayIdentity, RelayAuthError> {
        if let Some(expected_token) = self.expected_token.as_deref() {
            let matches: bool = expected_token
                .as_bytes()
                .ct_eq(request.token.as_bytes())
                .into();
            if !matches {
                return Err(RelayAuthError::InvalidToken);
            }
        }
        Ok(VerifiedRelayIdentity::bootstrap(request.action))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScopedTokenVerifier {
    accepted_tokens: BTreeMap<String, RelayTokenClaims>,
    issuer_secrets: BTreeMap<String, String>,
    now_ms: Option<u64>,
    revocations: Option<RelayRevocationRegistry>,
}

impl ScopedTokenVerifier {
    pub fn new(
        accepted_tokens: BTreeMap<String, RelayTokenClaims>,
        issuer_secrets: BTreeMap<String, String>,
        now_ms: Option<u64>,
    ) -> Self {
        Self {
            accepted_tokens,
            issuer_secrets,
            now_ms,
            revocations: None,
        }
    }

    /// Attach a live revocation registry so verification rejects tokens whose
    /// `jti` or account has been revoked by the hosted control plane.
    pub fn with_revocations(mut self, revocations: RelayRevocationRegistry) -> Self {
        self.revocations = Some(revocations);
        self
    }

    pub fn verify(
        &self,
        request: RelayAuthRequest<'_>,
    ) -> Result<VerifiedRelayIdentity, RelayAuthError> {
        if self.accepted_tokens.is_empty() && self.issuer_secrets.is_empty() {
            return Err(RelayAuthError::ScopedTokensUnavailable);
        }
        let claims = match self.accepted_tokens.get(request.token) {
            Some(claims) => claims.clone(),
            None => self.verify_signed_token(request.token)?,
        };
        let now_ms = self.now_ms.unwrap_or_else(current_unix_ms);
        validate_claims(&claims, request.action, request.target, Some(now_ms))?;
        if let Some(revocations) = &self.revocations {
            if revocations.is_revoked(&claims, now_ms) {
                return Err(RelayAuthError::TokenRevoked);
            }
        }
        Ok(identity_from_claims(claims))
    }

    fn verify_signed_token(&self, token: &str) -> Result<RelayTokenClaims, RelayAuthError> {
        if let Ok(signed) = decode_scoped_token_parts(token) {
            let claims = serde_json::from_slice::<RelayTokenClaims>(&signed.claims_payload)
                .map_err(|_| RelayAuthError::InvalidToken)?;
            let secret = self
                .issuer_secrets
                .get(&claims.issuer)
                .ok_or(RelayAuthError::InvalidToken)?;
            verify_hmac_signature(
                secret.as_bytes(),
                signed.signing_input.as_bytes(),
                &signed.signature,
            )?;
            // The bespoke `chariox-scoped-v1` format is being retired in favor
            // of the JWT form the cloud issuer emits. Count and warn on its use
            // so the format can be dropped once the metric reaches zero.
            note_legacy_scoped_token_verification();
            return Ok(claims);
        }

        let signed = decode_jwt_hmac_token_parts(token)?;
        let claims = parse_cloud_jwt_claims(&signed.claims_payload)?;
        let secret = self
            .issuer_secrets
            .get(&claims.issuer)
            .ok_or(RelayAuthError::InvalidToken)?;
        verify_hmac_signature(
            secret.as_bytes(),
            signed.signing_input.as_bytes(),
            &signed.signature,
        )?;
        Ok(claims)
    }
}

static LEGACY_SCOPED_TOKEN_VERIFICATIONS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

fn note_legacy_scoped_token_verification() {
    let previous =
        LEGACY_SCOPED_TOKEN_VERIFICATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if previous == 0 {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "chariox-relay",
                "level": "warn",
                "event": "deprecated_scoped_token_format",
                "fields": {
                    "format": "chariox-scoped-v1",
                    "message": "accepting a deprecated chariox-scoped-v1 relay token; migrate issuers to the JWT format",
                },
            })
        );
    }
}

/// Number of `chariox-scoped-v1` (pre-JWT) tokens verified this process. The
/// format can be removed once this stays at zero across a release.
pub fn legacy_scoped_token_verification_count() -> u64 {
    LEGACY_SCOPED_TOKEN_VERIFICATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn encode_scoped_hmac_token(
    claims: &RelayTokenClaims,
    issuer_secret: &str,
) -> Result<String, RelayAuthError> {
    let payload = serde_json::to_vec(claims).map_err(|_| RelayAuthError::InvalidToken)?;
    let claims_b64 = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_hmac(issuer_secret.as_bytes(), claims_b64.as_bytes())?;
    Ok(format!(
        "chariox-scoped-v1.{claims_b64}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

struct DecodedScopedToken {
    signing_input: String,
    claims_payload: Vec<u8>,
    signature: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
struct JwtHeader {
    alg: String,
    #[serde(default)]
    typ: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CloudJwtClaims {
    iss: String,
    sub: String,
    subject_kind: RelaySubjectKind,
    realm_id: RelayRealmId,
    allowed_actions: Vec<String>,
    #[serde(default)]
    allowed_targets: Option<Vec<String>>,
    iat: u64,
    exp: u64,
    jti: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    machine_id: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    public_key_thumbprint: Option<String>,
    #[serde(default)]
    entitlements_version: Option<String>,
}

fn decode_scoped_token_parts(token: &str) -> Result<DecodedScopedToken, RelayAuthError> {
    let mut parts = token.trim().split('.');
    let Some(prefix) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    let Some(claims_b64) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    let Some(signature_b64) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    if parts.next().is_some() || prefix != "chariox-scoped-v1" {
        return Err(RelayAuthError::InvalidToken);
    }
    let claims_payload = URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    Ok(DecodedScopedToken {
        signing_input: claims_b64.to_string(),
        claims_payload,
        signature,
    })
}

fn decode_jwt_hmac_token_parts(token: &str) -> Result<DecodedScopedToken, RelayAuthError> {
    let mut parts = token.trim().split('.');
    let Some(header_b64) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    let Some(claims_b64) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    let Some(signature_b64) = parts.next() else {
        return Err(RelayAuthError::InvalidToken);
    };
    if parts.next().is_some() {
        return Err(RelayAuthError::InvalidToken);
    }
    let header_payload = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    let header = serde_json::from_slice::<JwtHeader>(&header_payload)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    if header.alg != "HS256" || header.typ.as_deref().unwrap_or("JWT") != "JWT" {
        return Err(RelayAuthError::InvalidToken);
    }
    let claims_payload = URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    Ok(DecodedScopedToken {
        signing_input: format!("{header_b64}.{claims_b64}"),
        claims_payload,
        signature,
    })
}

fn parse_cloud_jwt_claims(payload: &[u8]) -> Result<RelayTokenClaims, RelayAuthError> {
    let claims = serde_json::from_slice::<CloudJwtClaims>(payload)
        .map_err(|_| RelayAuthError::InvalidToken)?;
    Ok(RelayTokenClaims {
        issuer: claims.iss,
        subject: claims.sub,
        subject_kind: claims.subject_kind,
        realm_id: claims.realm_id,
        allowed_actions: claims
            .allowed_actions
            .into_iter()
            .map(parse_cloud_action)
            .collect::<Result<Vec<_>, _>>()?,
        allowed_targets: claims.allowed_targets,
        issued_at_ms: claims.iat.saturating_mul(1000),
        expires_at_ms: claims.exp.saturating_mul(1000),
        token_id: claims.jti,
        account_id: claims.account_id,
        organization_id: claims.organization_id,
        user_id: claims.user_id,
        device_id: claims.device_id,
        machine_id: claims.machine_id,
        client_id: claims.client_id,
        session_id: claims.session_id,
        public_key_thumbprint: claims.public_key_thumbprint,
        entitlements_version: claims.entitlements_version,
    })
}

fn parse_cloud_action(action: String) -> Result<RelayAction, RelayAuthError> {
    match action.as_str() {
        "daemon.register" => Ok(RelayAction::DaemonRegister),
        "daemon.heartbeat" => Ok(RelayAction::DaemonHeartbeat),
        "client.metadata.read" => Ok(RelayAction::ClientMetadataRead),
        "client.connect" => Ok(RelayAction::ClientConnect),
        "packet.route" => Ok(RelayAction::PacketRoute),
        "peer.request" => Ok(RelayAction::PeerRequest),
        "peer.event" => Ok(RelayAction::PeerEvent),
        _ => Err(RelayAuthError::InvalidToken),
    }
}

// Accept a small amount of clock skew between the issuer and the relay so a
// token minted moments ago on a slightly-ahead issuer clock is not rejected
// as not-yet-valid.
const RELAY_CLOCK_SKEW_TOLERANCE_MS: u64 = 60_000;

fn validate_claims(
    claims: &RelayTokenClaims,
    action: RelayAction,
    target: Option<&str>,
    now_ms: Option<u64>,
) -> Result<(), RelayAuthError> {
    if let Some(now_ms) = now_ms {
        if claims.expires_at_ms <= now_ms {
            return Err(RelayAuthError::TokenExpired);
        }
        // Reject tokens whose issue time is implausibly far in the future
        // (beyond clock skew): a sign of a forged or misissued token.
        if claims.issued_at_ms > now_ms.saturating_add(RELAY_CLOCK_SKEW_TOLERANCE_MS) {
            return Err(RelayAuthError::InvalidToken);
        }
    }
    if !claims.allows_action(action) {
        return Err(RelayAuthError::ActionNotAllowed);
    }
    if !claims.allows_target(target) {
        return Err(RelayAuthError::TargetNotAllowed);
    }
    Ok(())
}

fn identity_from_claims(claims: RelayTokenClaims) -> VerifiedRelayIdentity {
    VerifiedRelayIdentity {
        realm_id: claims.realm_id,
        subject: claims.subject,
        subject_kind: claims.subject_kind,
        allowed_actions: claims.allowed_actions,
        allowed_targets: claims.allowed_targets,
        expires_at_ms: claims.expires_at_ms,
        token_id: Some(claims.token_id),
        user_id: claims.user_id,
        public_key_thumbprint: claims.public_key_thumbprint,
    }
}

fn sign_hmac(secret: &[u8], signing_input: &[u8]) -> Result<Vec<u8>, RelayAuthError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|_| RelayAuthError::InvalidToken)?;
    mac.update(signing_input);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_hmac_signature(
    secret: &[u8],
    signing_input: &[u8],
    signature: &[u8],
) -> Result<(), RelayAuthError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|_| RelayAuthError::InvalidToken)?;
    mac.update(signing_input);
    mac.verify_slice(signature)
        .map_err(|_| RelayAuthError::InvalidToken)
}

fn current_unix_ms() -> u64 {
    fail_closed_unix_ms(SystemTime::now().duration_since(UNIX_EPOCH))
}

// A broken clock must reject expired tokens, not accept them: map failure to
// the end of time so every expiry check fails closed.
fn fail_closed_unix_ms(now: Result<Duration, SystemTimeError>) -> u64 {
    now.map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(u64::MAX)
}

fn subject_kind_for_action(action: RelayAction) -> RelaySubjectKind {
    match action {
        RelayAction::DaemonRegister
        | RelayAction::DaemonHeartbeat
        | RelayAction::PeerRequest
        | RelayAction::PeerEvent => RelaySubjectKind::Kernel,
        RelayAction::ClientMetadataRead | RelayAction::ClientConnect | RelayAction::PacketRoute => {
            RelaySubjectKind::Client
        }
    }
}

#[cfg(test)]
mod tests;
