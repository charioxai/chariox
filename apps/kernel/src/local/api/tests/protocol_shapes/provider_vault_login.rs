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

// A login the new machine could not install (its Vault was not open) is named in the import
// receipt, on the relay peer wire and in the local receipt Cloud reads. A full import omits it.
const OMITTED_LOGINS_FIXTURE: &[u8] = include_bytes!("managed-context-omitted-logins-1000.json");

#[test]
fn managed_context_omitted_logins_1000_snapshot_and_hash() {
    use crate::managed_context::package::{
        ManagedContextProviderAccountNextStep, ManagedContextProviderAccountOmission,
    };
    use crate::transport::relay_peer::{
        RelayManagedContextImportReceipt, RelayManagedContextOmittedProviderAccount,
        RelayManagedDevelopmentContextImportReceipt, RelayManagedKernelContextImportReceipt,
    };
    assert_eq!(
        format!("{:x}", Sha256::digest(OMITTED_LOGINS_FIXTURE)),
        "2919924f3fb8371df640e0366e4ddea2810e5e9d658096cfcce1d2ca9c30f16a"
    );
    let fixture: serde_json::Value = serde_json::from_slice(OMITTED_LOGINS_FIXTURE).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );

    let receipt = |omitted_provider_accounts| RelayManagedContextImportReceipt {
        destination: None,
        transfer_id: "transfer".into(),
        archive_sha256: "a".repeat(64),
        plan_digest: "sha256:plan".into(),
        development: RelayManagedDevelopmentContextImportReceipt::Empty,
        kernel_context: RelayManagedKernelContextImportReceipt::Empty,
        omitted_provider_accounts,
        receipt_sha256: "b".repeat(64),
    };
    let relay = receipt(vec![RelayManagedContextOmittedProviderAccount {
        provider: "claude".into(),
        profile_id: "work".into(),
        label: "Work".into(),
        reason: ManagedContextProviderAccountOmission::VaultClosed,
        next_step: ManagedContextProviderAccountNextStep::SignInAgain,
    }]);
    assert_eq!(
        serde_json::to_value(&relay).unwrap()["omitted_provider_accounts"],
        fixture["relay_receipt_omitted_provider_accounts"]
    );
    let local =
        crate::managed_context::outbound_service::ManagedContextOutboundImportReceipt::from(relay);
    assert_eq!(
        serde_json::to_value(&local).unwrap()["omittedProviderAccounts"],
        fixture["local_receipt_omitted_provider_accounts"]
    );

    let full = serde_json::to_value(receipt(Vec::new())).unwrap();
    assert!(full.get("omitted_provider_accounts").is_none());
    let full_local = serde_json::to_value(
        crate::managed_context::outbound_service::ManagedContextOutboundImportReceipt::from(
            receipt(Vec::new()),
        ),
    )
    .unwrap();
    assert!(full_local.get("omittedProviderAccounts").is_none());
}

// A transfer into a kernel whose Vault is closed is refused with a reason class the UI words
// itself. Both strings are wire values: peers and clients match them exactly.
#[test]
fn the_closed_receiving_vault_reason_classes_are_wire_stable() {
    let refused = crate::transport::relay_peer::RelayPeerResponse::ManagedContextImportFailed {
        code: crate::managed_context::target_vault::TARGET_VAULT_LOCKED.into(),
        retryable: true,
    };
    assert_eq!(
        serde_json::to_value(&refused).unwrap(),
        serde_json::json!({
            "kind": "managed_context_import_failed",
            "code": "target_vault_locked",
            "retryable": true
        })
    );
    assert!(crate::provider_account_copy::is_public_code(
        "provider_account_copy_target_vault_locked"
    ));
    assert!(matches!(
        crate::provider_account_copy::failure("provider_account_copy_target_vault_locked"),
        crate::error::DaemonError::ProviderAccountCopy {
            code: "provider_account_copy_target_vault_locked",
            retryable: true,
            ..
        }
    ));
}
