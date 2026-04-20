use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
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
    pub device_id: Option<String>,
    pub machine_id: Option<String>,
    pub client_id: Option<String>,
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
    pub token_id: Option<String>,
    pub public_key_thumbprint: Option<String>,
}

impl VerifiedRelayIdentity {
    pub fn bootstrap(action: RelayAction) -> Self {
        Self {
            realm_id: DEFAULT_RELAY_REALM_ID.to_string(),
            subject: "shared-token-bootstrap".to_string(),
            subject_kind: subject_kind_for_action(action),
            allowed_actions: vec![action],
            allowed_targets: None,
            token_id: None,
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
    #[error("scoped relay tokens are not enabled")]
    ScopedTokensUnavailable,
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
            if expected_token != request.token {
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
        }
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
        validate_claims(&claims, request.action, request.target, self.now_ms)?;
        Ok(identity_from_claims(claims))
    }

    fn verify_signed_token(&self, token: &str) -> Result<RelayTokenClaims, RelayAuthError> {
        let signed = decode_scoped_token_parts(token)?;
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
        Ok(claims)
    }
}

pub fn encode_scoped_hmac_token(
    claims: &RelayTokenClaims,
    issuer_secret: &str,
) -> Result<String, RelayAuthError> {
    let payload = serde_json::to_vec(claims).map_err(|_| RelayAuthError::InvalidToken)?;
    let claims_b64 = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_hmac(issuer_secret.as_bytes(), claims_b64.as_bytes())?;
    Ok(format!(
        "arroba-scoped-v1.{claims_b64}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

struct DecodedScopedToken {
    signing_input: String,
    claims_payload: Vec<u8>,
    signature: Vec<u8>,
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
    if parts.next().is_some() || prefix != "arroba-scoped-v1" {
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
        token_id: Some(claims.token_id),
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
mod tests {
    use super::*;

    #[test]
    fn shared_verifier_accepts_matching_token() {
        let verifier = RelayAuthVerifier::shared(Some("secret".to_string()));
        let identity = verifier
            .verify(RelayAuthRequest {
                token: "secret",
                action: RelayAction::ClientConnect,
                target: Some("daemon-1"),
            })
            .expect("matching shared token should be accepted");

        assert_eq!(identity.realm_id, DEFAULT_RELAY_REALM_ID);
        assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    }

    #[test]
    fn shared_verifier_rejects_mismatched_token() {
        let verifier = RelayAuthVerifier::shared(Some("secret".to_string()));
        let error = verifier
            .verify(RelayAuthRequest {
                token: "wrong",
                action: RelayAction::DaemonRegister,
                target: None,
            })
            .expect_err("mismatched shared token should be rejected");

        assert_eq!(error, RelayAuthError::InvalidToken);
    }

    #[test]
    fn shared_verifier_preserves_open_bootstrap_mode_when_no_token_configured() {
        let verifier = RelayAuthVerifier::shared(None);
        let identity = verifier
            .verify(RelayAuthRequest {
                token: "",
                action: RelayAction::DaemonRegister,
                target: None,
            })
            .expect("unconfigured shared-token mode should preserve current open behavior");

        assert_eq!(identity.realm_id, DEFAULT_RELAY_REALM_ID);
        assert_eq!(identity.subject_kind, RelaySubjectKind::Kernel);
    }

    #[test]
    fn claims_check_actions_and_targets() {
        let claims = RelayTokenClaims {
            issuer: "issuer".to_string(),
            subject: "client-1".to_string(),
            subject_kind: RelaySubjectKind::Client,
            realm_id: "realm-1".to_string(),
            allowed_actions: vec![RelayAction::ClientConnect],
            allowed_targets: Some(vec!["daemon-1".to_string()]),
            issued_at_ms: 10,
            expires_at_ms: 20,
            token_id: "token-1".to_string(),
            account_id: None,
            organization_id: None,
            device_id: None,
            machine_id: None,
            client_id: Some("client-1".to_string()),
            public_key_thumbprint: None,
            entitlements_version: None,
        };

        assert!(claims.allows_action(RelayAction::ClientConnect));
        assert!(!claims.allows_action(RelayAction::DaemonRegister));
        assert!(claims.allows_target(Some("daemon-1")));
        assert!(!claims.allows_target(Some("daemon-2")));
        assert!(!claims.allows_target(None));
    }

    #[test]
    fn scoped_verifier_fails_closed_without_tokens_or_issuers() {
        let verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::default());
        let error = verifier
            .verify(RelayAuthRequest {
                token: "scoped-token",
                action: RelayAction::ClientConnect,
                target: Some("daemon-1"),
            })
            .expect_err("scoped verifier should fail closed without issuer metadata");

        assert_eq!(error, RelayAuthError::ScopedTokensUnavailable);
    }

    #[test]
    fn scoped_verifier_checks_action_target_and_expiration() {
        let mut tokens = BTreeMap::new();
        tokens.insert(
            "client-token".to_string(),
            RelayTokenClaims {
                issuer: "issuer".to_string(),
                subject: "client-1".to_string(),
                subject_kind: RelaySubjectKind::Client,
                realm_id: "realm-1".to_string(),
                allowed_actions: vec![RelayAction::ClientConnect],
                allowed_targets: Some(vec!["daemon-1".to_string()]),
                issued_at_ms: 10,
                expires_at_ms: 20,
                token_id: "token-1".to_string(),
                account_id: None,
                organization_id: None,
                device_id: None,
                machine_id: None,
                client_id: Some("client-1".to_string()),
                public_key_thumbprint: Some("thumbprint".to_string()),
                entitlements_version: None,
            },
        );
        let verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
            tokens,
            BTreeMap::new(),
            Some(15),
        ));

        let identity = verifier
            .verify(RelayAuthRequest {
                token: "client-token",
                action: RelayAction::ClientConnect,
                target: Some("daemon-1"),
            })
            .expect("valid scoped token should verify");
        assert_eq!(identity.realm_id, "realm-1");
        assert_eq!(identity.token_id.as_deref(), Some("token-1"));
        assert_eq!(
            identity.public_key_thumbprint.as_deref(),
            Some("thumbprint")
        );

        let action_error = verifier
            .verify(RelayAuthRequest {
                token: "client-token",
                action: RelayAction::DaemonRegister,
                target: Some("daemon-1"),
            })
            .expect_err("wrong action should be rejected");
        assert_eq!(action_error, RelayAuthError::ActionNotAllowed);

        let target_error = verifier
            .verify(RelayAuthRequest {
                token: "client-token",
                action: RelayAction::ClientConnect,
                target: Some("daemon-2"),
            })
            .expect_err("wrong target should be rejected");
        assert_eq!(target_error, RelayAuthError::TargetNotAllowed);
    }

    #[test]
    fn scoped_hmac_verifier_accepts_signed_hosted_issuer_tokens() {
        let claims = RelayTokenClaims {
            issuer: "arroba-cloud-test".to_string(),
            subject: "client-1".to_string(),
            subject_kind: RelaySubjectKind::Client,
            realm_id: "realm-1".to_string(),
            allowed_actions: vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
            allowed_targets: Some(vec!["daemon-1".to_string()]),
            issued_at_ms: 10,
            expires_at_ms: 20,
            token_id: "token-1".to_string(),
            account_id: Some("account-1".to_string()),
            organization_id: None,
            device_id: Some("device-1".to_string()),
            machine_id: None,
            client_id: Some("client-1".to_string()),
            public_key_thumbprint: Some("thumbprint".to_string()),
            entitlements_version: Some("entitlements-1".to_string()),
        };
        let token =
            encode_scoped_hmac_token(&claims, "issuer-secret").expect("token should encode");
        let verifier = RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
            Some(15),
        );

        let identity = verifier
            .verify(RelayAuthRequest {
                token: &token,
                action: RelayAction::ClientConnect,
                target: Some("daemon-1"),
            })
            .expect("signed hosted issuer token should verify");

        assert_eq!(identity.realm_id, "realm-1");
        assert_eq!(identity.subject, "client-1");
        assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
        assert_eq!(identity.token_id.as_deref(), Some("token-1"));
        assert_eq!(
            identity.public_key_thumbprint.as_deref(),
            Some("thumbprint")
        );
    }

    #[test]
    fn scoped_hmac_verifier_rejects_tampered_or_unknown_issuer_tokens() {
        let claims = RelayTokenClaims {
            issuer: "arroba-cloud-test".to_string(),
            subject: "client-1".to_string(),
            subject_kind: RelaySubjectKind::Client,
            realm_id: "realm-1".to_string(),
            allowed_actions: vec![RelayAction::ClientConnect],
            allowed_targets: None,
            issued_at_ms: 10,
            expires_at_ms: 20,
            token_id: "token-1".to_string(),
            account_id: None,
            organization_id: None,
            device_id: None,
            machine_id: None,
            client_id: Some("client-1".to_string()),
            public_key_thumbprint: None,
            entitlements_version: None,
        };
        let token =
            encode_scoped_hmac_token(&claims, "issuer-secret").expect("token should encode");
        let mut parts = token.split('.').map(str::to_string).collect::<Vec<_>>();
        parts[2].push('x');
        let tampered = parts.join(".");
        let verifier = RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
            Some(15),
        );
        let unknown_issuer = RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("other-issuer".to_string(), "issuer-secret".to_string())]),
            Some(15),
        );

        assert_eq!(
            verifier
                .verify(RelayAuthRequest {
                    token: &tampered,
                    action: RelayAction::ClientConnect,
                    target: None,
                })
                .expect_err("tampered token should fail"),
            RelayAuthError::InvalidToken
        );
        assert_eq!(
            unknown_issuer
                .verify(RelayAuthRequest {
                    token: &token,
                    action: RelayAction::ClientConnect,
                    target: None,
                })
                .expect_err("unknown issuer should fail"),
            RelayAuthError::InvalidToken
        );
    }
}
