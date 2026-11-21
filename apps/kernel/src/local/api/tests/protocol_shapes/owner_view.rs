use crate::local::{LocalDaemonRequest, LocalDaemonResponse, LOCAL_DAEMON_PROTOCOL_VERSION};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn owner_view_protocol_shape() {
    let bytes = include_bytes!("owner-view-1014.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "bb44b7e5205b8352ce8c94d9e57de302dbe1e79dbf0c70e9d23b249876666dce"
    );
    let fixture: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *wire);
    }
    for unsupported in [
        json!({"OwnerView":{"operation":"click","x":10,"y":20}}),
        json!({"OwnerView":{"operation":"status","text":"type"}}),
        json!({"OwnerView":{"operation":"end","agent_id":"agent","owner":"someone"}}),
        json!({"OwnerView":{"operation":"publish","generation":"generation","tab_id":"tab",
            "rows":{"navigator":[],"settings":[],"session_cookie":"value"}}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(unsupported).is_err());
    }
}
