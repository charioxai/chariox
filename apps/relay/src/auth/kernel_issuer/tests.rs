use super::*;
use crate::auth::{RelayAuthVerifier, DEFAULT_RELAY_REALM_ID};

const NOW_MS: u64 = 1_800_000_000_000;

fn issuer_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn keys_line(entries: &[(&str, &SigningKey)]) -> String {
    entries
        .iter()
        .map(|(issuer, key)| {
            format!(
                "{issuer}={}",
                encode_kernel_issuer_public_key(&key.verifying_key())
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn verifier(line: &str) -> RelayAuthVerifier {
    RelayAuthVerifier::SelfHosted(SelfHostedVerifier::new(
        "operator-test-token".into(),
        KernelIssuerKeys::parse(line).unwrap(),
        Some(NOW_MS),
    ))
}

fn claims(actions: Vec<RelayAction>) -> KernelIssuedClientClaims {
    KernelIssuedClientClaims {
        issuer: "chariox-kernel:home".into(),
        realm_id: DEFAULT_RELAY_REALM_ID.into(),
        client_id: "cli-1".into(),
        allowed_actions: actions,
        target_daemon_id: "home".into(),
        issued_at_ms: NOW_MS - 1_000,
        expires_at_ms: NOW_MS + 60_000,
        token_id: "tok-1".into(),
        public_key_thumbprint: Some("a".repeat(64)),
    }
}

fn attach() -> Vec<RelayAction> {
    vec![RelayAction::ClientConnect, RelayAction::PacketRoute]
}

fn request<'a>(
    token: &'a str,
    action: RelayAction,
    target: Option<&'a str>,
) -> RelayAuthRequest<'a> {
    RelayAuthRequest {
        token,
        action,
        target,
    }
}

#[test]
fn pinned_kernel_issuer_token_admits_its_client_with_key_binding() {
    let key = issuer_key(1);
    let verifier = verifier(&keys_line(&[("chariox-kernel:home", &key)]));
    let token = encode_kernel_issued_client_token(&claims(attach()), &key).unwrap();
    let identity = verifier
        .verify(request(&token, RelayAction::ClientConnect, Some("home")))
        .unwrap();
    assert_eq!(identity.subject, "cli-1");
    assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    assert_eq!(identity.client_id.as_deref(), Some("cli-1"));
    assert_eq!(identity.public_key_thumbprint, Some("a".repeat(64)));
    assert_eq!(identity.expires_at_ms, NOW_MS + 60_000);
    assert_eq!(identity.realm_id, DEFAULT_RELAY_REALM_ID);
}

#[test]
fn unpinned_key_forged_signature_and_unknown_issuer_are_refused() {
    let pinned = issuer_key(1);
    let verifier = verifier(&keys_line(&[("chariox-kernel:home", &pinned)]));
    let foreign = encode_kernel_issued_client_token(&claims(attach()), &issuer_key(2)).unwrap();
    let mut other_issuer = claims(attach());
    other_issuer.issuer = "chariox-kernel:other".into();
    let other_issuer = encode_kernel_issued_client_token(&other_issuer, &pinned).unwrap();
    let genuine = encode_kernel_issued_client_token(&claims(attach()), &pinned).unwrap();
    let (signed, _) = genuine.rsplit_once('.').unwrap();
    let mut widened = claims(attach());
    widened.target_daemon_id = "other".into();
    let widened = encode_kernel_issued_client_token(&widened, &pinned).unwrap();
    let swapped = format!(
        "{}.{}.{}",
        signed.split('.').next().unwrap(),
        widened.split('.').nth(1).unwrap(),
        genuine.rsplit_once('.').unwrap().1
    );
    for token in [foreign, other_issuer, swapped, format!("{signed}.AAAA")] {
        assert_eq!(
            verifier.verify(request(&token, RelayAction::ClientConnect, Some("home"))),
            Err(RelayAuthError::InvalidToken)
        );
    }
}

#[test]
fn expired_token_is_refused() {
    let key = issuer_key(1);
    let verifier = verifier(&keys_line(&[("chariox-kernel:home", &key)]));
    let mut expired = claims(attach());
    expired.expires_at_ms = NOW_MS;
    let token = encode_kernel_issued_client_token(&expired, &key).unwrap();
    assert_eq!(
        verifier.verify(request(&token, RelayAction::ClientConnect, Some("home"))),
        Err(RelayAuthError::TokenExpired)
    );
}

#[test]
fn scope_and_target_are_enforced() {
    let key = issuer_key(1);
    let verifier = verifier(&keys_line(&[("chariox-kernel:home", &key)]));
    let attach = encode_kernel_issued_client_token(&claims(attach()), &key).unwrap();
    let metadata =
        encode_kernel_issued_client_token(&claims(vec![RelayAction::ClientMetadataRead]), &key)
            .unwrap();
    assert_eq!(
        verifier.verify(request(&attach, RelayAction::ClientConnect, Some("other"))),
        Err(RelayAuthError::TargetNotAllowed)
    );
    assert_eq!(
        verifier.verify(request(&metadata, RelayAction::ClientConnect, Some("home"))),
        Err(RelayAuthError::ActionNotAllowed)
    );
    assert!(verifier
        .verify(request(
            &metadata,
            RelayAction::ClientMetadataRead,
            Some("home")
        ))
        .is_ok());
    for daemon_action in [RelayAction::DaemonRegister, RelayAction::PeerRequest] {
        assert_eq!(
            verifier.verify(request(&attach, daemon_action, None)),
            Err(RelayAuthError::ActionNotAllowed)
        );
    }
}

#[test]
fn operator_token_still_admits_the_kernel_and_nothing_else_is_shared() {
    let key = issuer_key(1);
    let verifier = verifier(&keys_line(&[("chariox-kernel:home", &key)]));
    let identity = verifier
        .verify(request(
            "operator-test-token",
            RelayAction::DaemonRegister,
            None,
        ))
        .unwrap();
    assert_eq!(identity.subject_kind, RelaySubjectKind::Kernel);
    assert_eq!(
        verifier.verify(request(
            "operator-test-tokex",
            RelayAction::ClientConnect,
            Some("home")
        )),
        Err(RelayAuthError::InvalidToken)
    );
}

#[test]
fn staged_rotation_accepts_both_pinned_keys_for_one_issuer() {
    let (old, next) = (issuer_key(1), issuer_key(3));
    let verifier = verifier(&keys_line(&[
        ("chariox-kernel:home", &old),
        ("chariox-kernel:home", &next),
    ]));
    for key in [&old, &next] {
        let token = encode_kernel_issued_client_token(&claims(attach()), key).unwrap();
        assert!(verifier
            .verify(request(&token, RelayAction::PacketRoute, Some("home")))
            .is_ok());
    }
}

#[test]
fn issuer_key_lines_fail_loudly_when_malformed() {
    let key = encode_kernel_issuer_public_key(&issuer_key(1).verifying_key());
    for line in [
        "".to_string(),
        "no-separator".into(),
        "=AAAA".into(),
        "chariox-kernel:home=not-a-key".into(),
        "chariox-kernel:home=AAAA".into(),
        format!("free-form-issuer={key}"),
        format!("chariox-kernel:={key}"),
    ] {
        assert!(KernelIssuerKeys::parse(&line).is_err(), "{line}");
    }
}

#[test]
fn an_issuer_cannot_grant_access_to_another_pinned_kernel() {
    let (home, other) = (issuer_key(1), issuer_key(2));
    let verifier = verifier(&keys_line(&[
        ("chariox-kernel:home", &home),
        ("chariox-kernel:other", &other),
    ]));
    let mut foreign_target = claims(attach());
    foreign_target.issuer = "chariox-kernel:other".into();
    let token = encode_kernel_issued_client_token(&foreign_target, &other).unwrap();
    assert_eq!(
        verifier.verify(request(&token, RelayAction::ClientConnect, Some("home"))),
        Err(RelayAuthError::TargetNotAllowed)
    );
}
