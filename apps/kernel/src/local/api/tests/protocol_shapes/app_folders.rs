use super::*;

#[test]
fn app_folder_grants_protocol_1053_shapes_hash_and_owner_boundary() {
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("app-folder-grants-1053.json")).unwrap();
    assert_eq!(snapshot["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        snapshot["relayPeer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for request in snapshot["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *request);
        let mut forged = request.clone();
        forged.as_object_mut().unwrap().values_mut().next().unwrap()["owner_id"] =
            serde_json::json!("another-owner");
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
    for response in snapshot["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *response);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "ac8e8f85752699a4f11a5a233cc7da3b340d99b1ea12657815fb4b42fc18e1eb"
    );
}
