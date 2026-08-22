//! Authenticated relay identity checks bound to encrypted request sender keys.

use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};

use crate::runtime::terminal_pairings::public_key_thumbprint;

use super::request_errors::relay_error;

pub(super) fn validate_bound_service_sender(
    caller_identity: Option<&RelayCallerIdentity>,
    encrypted_request: &EncryptedRelayPayload,
) -> Result<(), RelayError> {
    let Some(identity) =
        caller_identity.filter(|identity| identity.subject_kind == RelaySubjectKind::Service)
    else {
        return Ok(());
    };
    validate_identity_expiry(identity, "service")?;
    let Some(expected_thumbprint) = identity.public_key_thumbprint.as_deref() else {
        return Ok(());
    };
    validate_sender_key(expected_thumbprint, encrypted_request, "service")
}

/// Validates authenticated peer identity without breaking legacy relay peers.
///
/// Legacy and shared-token relays may omit caller identity or its sender-key
/// thumbprint. Scoped identities must identify a live daemon caller, and any supplied
/// thumbprint must match the encrypted request sender key.
pub(super) fn validate_optional_daemon_sender(
    caller_identity: Option<&RelayCallerIdentity>,
    encrypted_request: &EncryptedRelayPayload,
) -> Result<(), RelayError> {
    let Some(identity) = caller_identity else {
        return Ok(());
    };
    validate_optional_daemon_identity(identity)?;
    if let Some(expected_thumbprint) = identity.public_key_thumbprint.as_deref() {
        validate_sender_key(
            expected_thumbprint,
            encrypted_request,
            daemon_identity_label(identity.subject_kind),
        )?;
    }
    Ok(())
}

/// Requires the sender-bound Kernel identity used by managed context transfer.
pub(super) fn require_bound_kernel_sender<'a>(
    caller_identity: Option<&'a RelayCallerIdentity>,
    encrypted_request: &EncryptedRelayPayload,
) -> Result<&'a RelayCallerIdentity, RelayError> {
    let Some(identity) = caller_identity else {
        return Err(unauthorized(
            "managed context transfer requires an authenticated relay kernel identity",
        ));
    };
    validate_kernel_identity(identity)?;
    let Some(expected_thumbprint) = identity.public_key_thumbprint.as_deref() else {
        return Err(unauthorized(
            "managed context transfer requires a sender-bound relay kernel identity",
        ));
    };
    validate_sender_key(expected_thumbprint, encrypted_request, "kernel")?;
    Ok(identity)
}

fn validate_kernel_identity(identity: &RelayCallerIdentity) -> Result<(), RelayError> {
    if identity.subject_kind != RelaySubjectKind::Kernel {
        return Err(unauthorized(
            "relay peer caller identity must identify a kernel",
        ));
    }
    validate_identity_expiry(identity, "kernel")
}

fn validate_optional_daemon_identity(identity: &RelayCallerIdentity) -> Result<(), RelayError> {
    if !matches!(
        identity.subject_kind,
        RelaySubjectKind::Kernel | RelaySubjectKind::Machine | RelaySubjectKind::Service
    ) {
        return Err(unauthorized(
            "relay peer caller identity must identify a daemon",
        ));
    }
    if identity.expires_at_ms == 0 && identity.token_id.is_none() {
        return Ok(());
    }
    validate_identity_expiry(identity, daemon_identity_label(identity.subject_kind))
}

fn daemon_identity_label(subject_kind: RelaySubjectKind) -> &'static str {
    match subject_kind {
        RelaySubjectKind::Kernel => "kernel",
        RelaySubjectKind::Machine => "machine",
        RelaySubjectKind::Service => "service",
        RelaySubjectKind::Client => "client",
    }
}

fn validate_identity_expiry(
    identity: &RelayCallerIdentity,
    subject: &str,
) -> Result<(), RelayError> {
    if identity.expires_at_ms <= crate::session::unix_epoch_ms() {
        return Err(unauthorized(&format!(
            "relay {subject} identity has expired"
        )));
    }
    Ok(())
}

fn validate_sender_key(
    expected_thumbprint: &str,
    encrypted_request: &EncryptedRelayPayload,
    subject: &str,
) -> Result<(), RelayError> {
    let actual_thumbprint = public_key_thumbprint(&encrypted_request.sender_public_key);
    if actual_thumbprint != expected_thumbprint {
        return Err(unauthorized(&format!(
            "relay {subject} sender key does not match its authenticated identity"
        )));
    }
    Ok(())
}

fn unauthorized(message: &str) -> RelayError {
    relay_error("unauthorized", message, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller_identity(
        subject_kind: RelaySubjectKind,
        public_key_thumbprint: Option<String>,
    ) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: "caller-1".to_string(),
            subject_kind,
            expires_at_ms: u64::MAX,
            token_id: Some("token-1".to_string()),
            user_id: Some("user-1".to_string()),
            public_key_thumbprint,
        }
    }

    fn encrypted_request(sender_public_key: &str) -> EncryptedRelayPayload {
        EncryptedRelayPayload {
            sender_public_key: sender_public_key.to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        }
    }

    #[test]
    fn service_sender_key_must_match_bound_thumbprint() {
        let request = encrypted_request("ephemeral-service-public-key");
        let identity = caller_identity(
            RelaySubjectKind::Service,
            Some(public_key_thumbprint(&request.sender_public_key)),
        );

        assert!(validate_bound_service_sender(Some(&identity), &request).is_ok());
    }

    #[test]
    fn mismatched_service_sender_key_is_rejected_before_dispatch() {
        let identity = caller_identity(
            RelaySubjectKind::Service,
            Some(public_key_thumbprint("different-public-key")),
        );
        let error = validate_bound_service_sender(
            Some(&identity),
            &encrypted_request("ephemeral-service-public-key"),
        )
        .expect_err("a stolen service token must not act as an unbound bearer token");

        assert_eq!(error.code, "unauthorized");
        assert!(!error.retryable);
    }

    #[test]
    fn expired_service_identity_is_rejected_before_dispatch() {
        let mut identity = caller_identity(RelaySubjectKind::Service, None);
        identity.expires_at_ms = 1;
        let error = validate_bound_service_sender(
            Some(&identity),
            &encrypted_request("ephemeral-service-public-key"),
        )
        .expect_err("an authenticated socket must not outlive its service token");

        assert_eq!(error.code, "unauthorized");
        assert!(!error.retryable);
    }

    #[test]
    fn ordinary_client_identity_is_not_subject_to_service_key_binding() {
        let identity = caller_identity(
            RelaySubjectKind::Client,
            Some(public_key_thumbprint("paired-client-public-key")),
        );

        assert!(validate_bound_service_sender(
            Some(&identity),
            &encrypted_request("per-request-client-public-key"),
        )
        .is_ok());
    }

    #[test]
    fn optional_peer_validation_accepts_legacy_and_unbound_kernel_senders() {
        let request = encrypted_request("legacy-kernel-public-key");
        let identity = caller_identity(RelaySubjectKind::Kernel, None);

        assert!(validate_optional_daemon_sender(None, &request).is_ok());
        assert!(validate_optional_daemon_sender(Some(&identity), &request).is_ok());
    }

    #[test]
    fn optional_peer_validation_accepts_hosted_daemon_identity_kinds() {
        let request = encrypted_request("hosted-daemon-public-key");
        for subject_kind in [
            RelaySubjectKind::Kernel,
            RelaySubjectKind::Machine,
            RelaySubjectKind::Service,
        ] {
            let identity = caller_identity(
                subject_kind,
                Some(public_key_thumbprint(&request.sender_public_key)),
            );
            assert!(validate_optional_daemon_sender(Some(&identity), &request).is_ok());
        }
    }

    #[test]
    fn optional_peer_validation_accepts_legacy_unscoped_zero_expiry() {
        let request = encrypted_request("legacy-shared-token-public-key");
        let mut legacy = caller_identity(RelaySubjectKind::Kernel, None);
        legacy.expires_at_ms = 0;
        legacy.token_id = None;
        assert!(validate_optional_daemon_sender(Some(&legacy), &request).is_ok());

        legacy.token_id = Some("scoped-token-1".to_string());
        let error = validate_optional_daemon_sender(Some(&legacy), &request)
            .expect_err("a scoped identity must never treat zero expiry as unbounded");
        assert_eq!(error.code, "unauthorized");
        assert!(!error.retryable);
    }

    #[test]
    fn optional_peer_validation_rejects_expired_or_non_daemon_identity() {
        let request = encrypted_request("kernel-public-key");
        let mut expired = caller_identity(RelaySubjectKind::Kernel, None);
        expired.expires_at_ms = 1;
        let client = caller_identity(RelaySubjectKind::Client, None);

        for identity in [&expired, &client] {
            let error = validate_optional_daemon_sender(Some(identity), &request)
                .expect_err("an authenticated peer must identify a live daemon");
            assert_eq!(error.code, "unauthorized");
            assert!(!error.retryable);
        }
    }

    #[test]
    fn optional_peer_validation_enforces_supplied_kernel_binding() {
        let request = encrypted_request("kernel-public-key");
        let matching = caller_identity(
            RelaySubjectKind::Kernel,
            Some(public_key_thumbprint(&request.sender_public_key)),
        );
        let mismatched = caller_identity(
            RelaySubjectKind::Kernel,
            Some(public_key_thumbprint("different-public-key")),
        );

        assert!(validate_optional_daemon_sender(Some(&matching), &request).is_ok());
        let error = validate_optional_daemon_sender(Some(&mismatched), &request)
            .expect_err("a scoped kernel token must stay bound to its sender key");
        assert_eq!(error.code, "unauthorized");
        assert!(!error.retryable);
    }

    #[test]
    fn managed_transfer_requires_authenticated_bound_kernel_sender() {
        let request = encrypted_request("managed-source-public-key");
        let unbound = caller_identity(RelaySubjectKind::Kernel, None);
        let bound = caller_identity(
            RelaySubjectKind::Kernel,
            Some(public_key_thumbprint(&request.sender_public_key)),
        );
        let machine = caller_identity(
            RelaySubjectKind::Machine,
            Some(public_key_thumbprint(&request.sender_public_key)),
        );

        assert!(require_bound_kernel_sender(None, &request).is_err());
        assert!(require_bound_kernel_sender(Some(&unbound), &request).is_err());
        assert!(require_bound_kernel_sender(Some(&machine), &request).is_err());
        assert_eq!(
            require_bound_kernel_sender(Some(&bound), &request)
                .expect("a live bound kernel should authorize managed transfer"),
            &bound
        );
    }
}
