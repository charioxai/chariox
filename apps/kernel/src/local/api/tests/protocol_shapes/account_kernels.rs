use super::*;

#[test]
fn account_kernel_directory_protocol_shapes_and_hash() {
    let bytes = include_bytes!("account-kernels-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "235a6386130e2feb5a4131865c49ff04828c930b1dce0093aca23c8fb7f5c0af"
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
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "CloudAccountKernels": {"action":"unlink","target_id":"row","agent_id":"forged"}
        }))
        .is_err()
    );
}
