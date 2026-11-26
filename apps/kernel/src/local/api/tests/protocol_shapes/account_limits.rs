use super::*;

/// Local 1017: the account's machine limits over the control plane and the machine-acquisition request.
#[test]
fn account_limits_protocol_shapes_and_hash() {
    let bytes = include_bytes!("account-limits-1017.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "aa54d0ededfe92244e69d3a209c815c1ce91ddfc14fa2b95bb263d809c9a13fc"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    // A request carries nothing the kernel does not name: no forged field, no unknown approval kind.
    for forged in [
        serde_json::json!({"ControlPlane":{"action":"limits","token":"must-not-travel"}}),
        serde_json::json!({"ControlPlane":{"action":"set_limits","expected_version":1,"machines_at_once":2,"spend_per_day_micros":1,
            "approval":{"kind":"owner","ref":"x","from":{"machines_at_once":1,"spend_per_day_micros":1},"to":{"machines_at_once":2,"spend_per_day_micros":1}}}}),
        serde_json::json!({"MachineAcquisition":{"action":"set_limits","scope":"everyone","machines_at_once":2}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
