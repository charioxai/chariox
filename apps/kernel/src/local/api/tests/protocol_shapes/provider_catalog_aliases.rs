//! Local 1012: a provider catalog's model lists `aliases`, the other names the provider's own tool
//! accepts for it. The field is absent when a model has none.
use super::*;
use crate::transport::kernel_protocol::KernelEvent;
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn provider_catalog_aliases_1012_snapshot_and_hash() {
    let bytes = include_bytes!("provider-catalog-aliases-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ce6ffee574418abb46cbeff388343e1f44198bb64c23f3eb849a53eef4699280"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["relayPeer"], RELAY_PEER_PROTOCOL_VERSION);
    let response = &fixture["responses"][0];
    let parsed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
    let LocalDaemonResponse::ProviderCatalog { catalog } = &parsed else {
        panic!("unexpected response");
    };
    let models = &catalog.all[0].models;
    assert_eq!(models["claude-haiku-5-5"].aliases, ["haiku"]);
    assert!(models["claude-sonnet-5-5"].aliases.is_empty());
    assert_eq!(serde_json::to_value(parsed).unwrap(), *response);
    let event: KernelEvent = serde_json::from_value(fixture["events"][0].clone()).unwrap();
    assert!(matches!(event, KernelEvent::ProviderCatalogChanged { .. }));
    assert_eq!(serde_json::to_value(event).unwrap(), fixture["events"][0]);
}
