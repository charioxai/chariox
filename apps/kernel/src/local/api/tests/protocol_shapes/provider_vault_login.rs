//! MP-08 / MP-09 / MP-10 / MP-11: local 1000 / relay peer 439 carry a Vault provider login to
//! a new managed machine. The wire holds ciphertext and ids; a login that cannot move says so.
use super::*;
use crate::account_profile::{ProviderAccountProfileOrigin, ProviderAccountReplicaMetadata};

#[test]
fn vault_login_1000_shapes_are_pinned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

    // A login that can move leaves the field out; one that cannot says `unavailable`.
    let available =
        serde_json::to_value(crate::account_profile::ProviderAccountContextTransfer::Available)
            .unwrap();
    assert_eq!(available, serde_json::json!("available"));
    assert_eq!(
        serde_json::to_value(crate::account_profile::ProviderAccountContextTransfer::Unavailable)
            .unwrap(),
        serde_json::json!("unavailable")
    );
    assert!(
        serde_json::from_value::<crate::account_profile::ProviderAccountContextTransfer>(
            serde_json::json!("vault_credential")
        )
        .is_err()
    );

    let source = crate::config::DaemonConfig::new("source", "source", "owner");
    let target = crate::config::DaemonConfig::new("target", "target", "owner");
    let binding = crate::provider_vault_login::VaultLoginBinding {
        context_id: "context".into(),
        source_kernel_id: "source".into(),
        target_kernel_id: "target".into(),
        provider: "claude".into(),
        profile_id: "work".into(),
    };
    let login = crate::provider_vault_login::seal(
        &source.relay_private_key,
        &target.relay_public_key,
        &binding,
        ProviderAccountReplicaMetadata {
            owner_user_id: "owner".into(),
            provider: "claude".into(),
            profile_id: "work".into(),
            label: "Work".into(),
            origin: ProviderAccountProfileOrigin::CharioxCreated,
            is_default: false,
        },
        "sk-ant-oat01-SHAPE-ONLY",
    )
    .unwrap();
    let value = serde_json::to_value(&login).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["generated_at_ms", "profile", "schema_version", "sealed"]
    );
    let mut sealed: Vec<_> = value["sealed"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    sealed.sort();
    assert_eq!(sealed, ["ciphertext", "nonce", "sender_public_key"]);
    assert!(!value.to_string().contains("SHAPE-ONLY"));
}
