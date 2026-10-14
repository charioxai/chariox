//! MP-08 / MP-10 / MP-11: coordinator-allocated 518/126 catalog contract.
use super::*;
#[test]
fn client_catalog_revisions_518_126_are_versioned_and_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 518);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        126
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_str!("../../types/provider_control.rs").as_bytes())
        ),
        "a9beb2a0337fa1db9c4e3dc0e7bcd249f70e92703b08fa3fd4aa47c754d0b7a1"
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
