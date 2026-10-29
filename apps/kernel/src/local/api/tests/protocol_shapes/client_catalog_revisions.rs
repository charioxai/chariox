//! MP-08 / MP-10 / MP-11: coordinator-allocated 534/142 catalog contract.
use super::*;
#[test]
fn client_catalog_revisions_534_142_are_versioned_and_hashed() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_str!("../../types/provider_control.rs").as_bytes())
        ),
        "f5b727d947480d46c8e4194c27e8ececa66ebe1fef5b2bffd24e078f9c89962a"
    );
    let request =
        LocalDaemonRequest::GetClientCatalogRevisions(crate::local::GetProviderCatalogRequest {
            provider: Some("codex".into()),
            account_profiles: [("codex".into(), "work".into())].into(),
            execution_location: crate::local::api::ProviderCatalogExecutionLocation::Local,
        });
    let response = LocalDaemonResponse::ClientCatalogRevisions {
        revisions: crate::local::ClientCatalogRevisions {
            provider_accounts: "d".repeat(64),
            provider_catalog: "a".repeat(64),
            provider_commands: "b".repeat(64),
            terminal_commands: "c".repeat(64),
        },
    };
    let value = serde_json::json!({"request": request, "response": response});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
        "092327e096789e41f60feae6547e394a119604c8ad21a417b51b94cbab19cb42"
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value["request"].clone()).unwrap(),
        request
    );
}

#[test]
fn client_catalog_snapshot_and_change_event_534_142_are_versioned_and_hashed() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({"GetClientCatalogSnapshot": {"scope": {"provider": null, "account_profiles": {}, "execution_location": {"kind": "local"}}, "known": null}})).unwrap();
    let response = LocalDaemonResponse::ClientCatalogSnapshot {
        snapshot: crate::local::ClientCatalogSnapshot {
            revisions: crate::local::ClientCatalogRevisions {
                provider_accounts: "a".repeat(64),
                provider_catalog: "b".repeat(64),
                provider_commands: "c".repeat(64),
                terminal_commands: format!("sha256:{}", "d".repeat(64)),
            },
            provider_accounts: None,
            provider_catalog: None,
            provider_commands: None,
            terminal_commands: None,
        },
    };
    let event =
        crate::transport::kernel_protocol::KernelEvent::ClientCatalogChanged { revision: 7 };
    let value = serde_json::json!({"request": request, "response": response, "event": event});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
        "5428acbd98f1edab6cb0b2a4fc56d871adba63f1917477c0ee2599e90e792eab"
    );
}
