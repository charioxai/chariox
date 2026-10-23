use chariox_relay::auth::{
    KernelIssuerKeys, RelayAuthError, RelayAuthRequest, RelayAuthVerifier, SelfHostedVerifier,
};

use super::*;
use crate::local::RelayIssuerStatusRequest;
use crate::runtime::command::KernelCommandSource;
use crate::runtime::self_host_terminal_grants;
use crate::runtime::terminal_pairings::public_key_thumbprint;

fn self_hosted_config() -> DaemonConfig {
    let root = std::env::temp_dir().join(format!(
        "chariox-relay-issuer-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.relay_url = Some("ws://relay.test".into());
    config.relay_token = Some("operator-test-token".into());
    config.cloud_relay = None;
    config
}

fn local() -> KernelCaller {
    KernelCaller::for_source(&KernelCommandSource::LocalIpc)
}

fn pair(config: &DaemonConfig, client_id: &str, key: &str) -> PersistedClientPairing {
    let link = format!("test-link-{client_id}");
    self_host_terminal_grants::register(
        config,
        &link,
        client_id,
        crate::session::unix_epoch_ms() + 60_000,
        "cli",
        None,
    )
    .unwrap();
    self_host_terminal_grants::redeem(
        config,
        &link,
        PersistedClientPairing {
            client_id: client_id.into(),
            public_key_thumbprint: public_key_thumbprint(key),
            ..Default::default()
        },
    )
    .unwrap()
}

fn enable(config: &DaemonConfig, step: RelayIssuerStep) -> RelayIssuerRecord {
    match execute_relay_issuer_request(
        config,
        &local(),
        LocalDaemonRequest::ConfigureRelayIssuer(ConfigureRelayIssuerRequest { step }),
    )
    .unwrap()
    {
        LocalDaemonResponse::RelayIssuer {
            issuer: Some(issuer),
        } => issuer,
        other => panic!("unexpected response {other:?}"),
    }
}

fn issue(
    config: &DaemonConfig,
    caller: &KernelCaller,
    client_id: &str,
    key: &str,
    scope: RelayClientTokenScope,
    lifetime_ms: Option<u64>,
) -> Result<RelayClientToken, String> {
    execute_relay_issuer_request(
        config,
        caller,
        LocalDaemonRequest::IssueRelayClientToken(IssueRelayClientTokenRequest {
            client_id: client_id.into(),
            public_key_thumbprint: public_key_thumbprint(key),
            scope,
            lifetime_ms,
        }),
    )
    .map(|response| match response {
        LocalDaemonResponse::RelayClientTokenIssued { token } => token,
        other => panic!("unexpected response {other:?}"),
    })
    .map_err(|error| error.to_string())
}

fn relay(keys: &str, now_ms: Option<u64>) -> RelayAuthVerifier {
    RelayAuthVerifier::SelfHosted(SelfHostedVerifier::new(
        "operator-test-token".into(),
        KernelIssuerKeys::parse(keys).unwrap(),
        now_ms,
    ))
}

fn connect<'a>(token: &'a str, target: &'a str) -> RelayAuthRequest<'a> {
    RelayAuthRequest {
        token,
        action: RelayAction::ClientConnect,
        target: Some(target),
    }
}

#[test]
fn issued_token_is_admitted_by_a_relay_pinned_with_the_issuer_line() {
    let config = self_hosted_config();
    let issuer = enable(&config, RelayIssuerStep::Enable);
    assert_eq!(
        issuer.issuer_id,
        format!("chariox-kernel:{}", config.daemon_id)
    );
    assert!(!issuer.relay_issuer_keys.contains("operator-test-token"));
    pair(&config, "cli-1", "cli-key");
    let token = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    )
    .unwrap();
    assert_eq!(token.target_daemon_id, config.daemon_id);
    assert_eq!(token.relay_url, "ws://relay.test");
    let identity = relay(&issuer.relay_issuer_keys, None)
        .verify(connect(&token.relay_token, &config.daemon_id))
        .unwrap();
    assert_eq!(identity.client_id.as_deref(), Some("cli-1"));
    assert_eq!(
        identity.public_key_thumbprint,
        Some(public_key_thumbprint("cli-key"))
    );
    assert_eq!(identity.expires_at_ms, token.expires_at_ms);
    let lifetime = token.expires_at_ms - crate::session::unix_epoch_ms();
    assert!(lifetime <= DEFAULT_CLIENT_TOKEN_LIFETIME_MS && lifetime > 14 * 60 * 1000);
}

#[test]
fn wrong_thumbprint_unpaired_and_revoked_clients_are_refused() {
    let config = self_hosted_config();
    enable(&config, RelayIssuerStep::Enable);
    pair(&config, "cli-1", "cli-key");
    let wrong = issue(
        &config,
        &local(),
        "cli-1",
        "other-key",
        RelayClientTokenScope::Attach,
        None,
    );
    assert!(wrong.unwrap_err().contains("key thumbprint does not match"));
    let unpaired = issue(
        &config,
        &local(),
        "cli-2",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    );
    assert!(unpaired.unwrap_err().contains("not paired or was revoked"));
    self_host_terminal_grants::revoke(&config, "cli-1").unwrap();
    let revoked = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    );
    assert!(revoked.unwrap_err().contains("not paired or was revoked"));
}

#[test]
fn a_paired_terminal_renews_only_its_own_identity_and_key() {
    let config = self_hosted_config();
    enable(&config, RelayIssuerStep::Enable);
    let own = pair(&config, "cli-1", "cli-key");
    pair(&config, "cli-2", "other-key");
    let caller = KernelCaller::from_paired_terminal(own);
    assert!(issue(
        &config,
        &caller,
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None
    )
    .is_ok());
    let other = issue(
        &config,
        &caller,
        "cli-2",
        "other-key",
        RelayClientTokenScope::Attach,
        None,
    );
    assert!(other.unwrap_err().contains("only renew its own"));
    let unverified = KernelCaller::for_source(&KernelCommandSource::RelayClient);
    assert!(issue(
        &config,
        &unverified,
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None
    )
    .is_err());
    for request in [
        LocalDaemonRequest::RelayIssuerStatus(RelayIssuerStatusRequest),
        LocalDaemonRequest::ConfigureRelayIssuer(ConfigureRelayIssuerRequest {
            step: RelayIssuerStep::Disable,
        }),
    ] {
        assert!(execute_relay_issuer_request(&config, &caller, request).is_err());
    }
}

#[test]
fn scope_target_and_expiry_are_enforced_by_the_relay() {
    let config = self_hosted_config();
    let issuer = enable(&config, RelayIssuerStep::Enable);
    pair(&config, "cli-1", "cli-key");
    let metadata = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Metadata,
        None,
    )
    .unwrap();
    let attach = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        Some(60_000),
    )
    .unwrap();
    let relay_now = relay(&issuer.relay_issuer_keys, None);
    assert_eq!(
        relay_now.verify(connect(&metadata.relay_token, &config.daemon_id)),
        Err(RelayAuthError::ActionNotAllowed)
    );
    assert_eq!(
        relay_now.verify(connect(&attach.relay_token, "another-kernel")),
        Err(RelayAuthError::TargetNotAllowed)
    );
    let relay_later = relay(&issuer.relay_issuer_keys, Some(attach.expires_at_ms));
    assert_eq!(
        relay_later.verify(connect(&attach.relay_token, &config.daemon_id)),
        Err(RelayAuthError::TokenExpired)
    );
    for lifetime in [59_999, MAX_CLIENT_TOKEN_LIFETIME_MS + 1] {
        let refused = issue(
            &config,
            &local(),
            "cli-1",
            "cli-key",
            RelayClientTokenScope::Attach,
            Some(lifetime),
        );
        assert!(refused.unwrap_err().contains("lifetime_ms"));
    }
}

#[test]
fn issuing_needs_the_enabled_issuer_and_a_cloud_free_relay() {
    let config = self_hosted_config();
    pair(&config, "cli-1", "cli-key");
    let disabled = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    );
    assert!(disabled.unwrap_err().contains("not enabled"));
    assert!(pairing_link_token(&config, "cli-9", u64::MAX)
        .unwrap()
        .is_none());
    enable(&config, RelayIssuerStep::Enable);
    let mut no_relay = config.clone();
    no_relay.relay_url = None;
    assert!(issue(
        &no_relay,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None
    )
    .is_err());
}

#[test]
fn staged_rotation_keeps_old_tokens_valid_until_activation() {
    let config = self_hosted_config();
    let first = enable(&config, RelayIssuerStep::Enable);
    pair(&config, "cli-1", "cli-key");
    let staged = enable(&config, RelayIssuerStep::StageRotation);
    assert_eq!(staged.active_key, first.active_key);
    let next = staged.next_key.clone().unwrap();
    assert!(staged.relay_issuer_keys.contains(&next.public_key));
    let before = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    )
    .unwrap();
    assert!(relay(&first.relay_issuer_keys, None)
        .verify(connect(&before.relay_token, &config.daemon_id))
        .is_ok());
    let active = enable(&config, RelayIssuerStep::ActivateRotation);
    assert_eq!(active.active_key, next);
    assert!(active.next_key.is_none());
    let after = issue(
        &config,
        &local(),
        "cli-1",
        "cli-key",
        RelayClientTokenScope::Attach,
        None,
    )
    .unwrap();
    assert_eq!(
        relay(&first.relay_issuer_keys, None)
            .verify(connect(&after.relay_token, &config.daemon_id)),
        Err(RelayAuthError::InvalidToken)
    );
    assert!(relay(&staged.relay_issuer_keys, None)
        .verify(connect(&after.relay_token, &config.daemon_id))
        .is_ok());
    assert!(execute_relay_issuer_request(
        &config,
        &local(),
        LocalDaemonRequest::ConfigureRelayIssuer(ConfigureRelayIssuerRequest {
            step: RelayIssuerStep::ActivateRotation,
        }),
    )
    .is_err());
}

#[test]
fn pairing_link_token_reaches_only_this_kernel_and_is_not_key_bound() {
    let config = self_hosted_config();
    let issuer = enable(&config, RelayIssuerStep::Enable);
    let expires = crate::session::unix_epoch_ms() + 5 * 60 * 1000;
    let token = pairing_link_token(&config, "cli-link", expires)
        .unwrap()
        .unwrap();
    let identity = relay(&issuer.relay_issuer_keys, None)
        .verify(connect(&token.relay_token, &config.daemon_id))
        .unwrap();
    assert_eq!(identity.client_id.as_deref(), Some("cli-link"));
    assert_eq!(identity.public_key_thumbprint, None);
    assert!(identity.expires_at_ms <= expires);
    assert_ne!(token.relay_token, "operator-test-token");
}
