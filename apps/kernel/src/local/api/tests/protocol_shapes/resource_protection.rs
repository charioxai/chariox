//! Machine-local resource switches are shared with every kernel client.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn resource_protection_protocol_1017_455_snapshot_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("resource-protection-1017.json")).unwrap();
    let mut config = crate::config::CharioxUserConfig::default();
    let defaults = serde_json::to_value(config.kernel.resource_protection).unwrap();
    config.kernel.resource_protection.memory = false;
    config.kernel.resource_protection.cpu = false;
    let response = serde_json::to_value(LocalDaemonResponse::UserConfig {
        path: "/kernel/config.toml".into(),
        config,
    })
    .unwrap();
    let requests = expected["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|wire| {
            let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
            serde_json::to_value(request).unwrap()
        })
        .collect::<Vec<_>>();
    let actual = serde_json::json!({"defaults": defaults, "mixed": response["UserConfig"]["config"]["kernel"]["resource_protection"], "requests": requests});
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "e4f498048e528135c01437375ce0c974abc0355d5f33189861f2b312c3ff35c1"
    );
}
