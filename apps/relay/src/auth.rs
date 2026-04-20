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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySubjectKind {
    Client,
    Kernel,
    Machine,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAction {
    DaemonRegister,
    DaemonHeartbeat,
    ClientMetadataRead,
    ClientConnect,
    PacketRoute,
    PeerRequest,
    PeerEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct ScopedTokenVerifier;

impl ScopedTokenVerifier {
    pub fn verify(
        &self,
        _request: RelayAuthRequest<'_>,
    ) -> Result<VerifiedRelayIdentity, RelayAuthError> {
        Err(RelayAuthError::ScopedTokensUnavailable)
    }
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
    fn scoped_verifier_is_a_non_accepting_skeleton_until_issuer_support_lands() {
        let verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier);
        let error = verifier
            .verify(RelayAuthRequest {
                token: "scoped-token",
                action: RelayAction::ClientConnect,
                target: Some("daemon-1"),
            })
            .expect_err("scoped verifier should fail closed until implemented");

        assert_eq!(error, RelayAuthError::ScopedTokensUnavailable);
    }
}
