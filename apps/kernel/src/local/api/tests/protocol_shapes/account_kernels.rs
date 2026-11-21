use super::*;

#[test]
fn account_kernel_directory_protocol_shapes_and_hash() {
    let bytes = include_bytes!("account-kernels-1013.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ec6a58c2f9c3dd0e02a97699befb23c228e0b665524de4fb4cb80ae6012d58bd"
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
