//! MP-08/MP-10/MP-11: protocol 472 Browser artifact snapshot and hash.
use super::*;

#[test]
fn mp08_mp10_mp11_browser_artifact_protocol_437_snapshot_and_hash() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    let wire = serde_json::json!({"RoomBrowserArtifact": {
        "session_id":"room-a", "attachment_id":"client-a", "tab_id":"tab-a",
        "operation":{"action":"capture","kind":"image","browser_generation":1,"guid":null,"return_image_base64":true}
    }});
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "1fe22b3d6d481ef9a0c7a50cbab08d18808c835fecf2a220f1d6f92de3c1a7a7"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"RoomBrowserArtifact": {
        "session_id":"room-a","attachment_id":"client-a","tab_id":"tab-a",
        "operation":{"action":"capture","kind":"image","browser_generation":1,"path":"/private/profile"}
    }})).is_err());
}

#[test]
fn mp08_mp10_mp11_browser_artifact_peer_82_shape_hashes() {
    use crate::transport::room_browser_controller::{
        RoomBrowserControllerCommand, RoomBrowserControllerResult,
    };
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        82
    );
    let wire = serde_json::json!({"kind":"artifact","request":{"target_id":"tab-target","document_id":"doc-a","browser_generation":1,"viewport":{"css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800,"revision":1,"last_actor_id":null},"kind":"network"}});
    let typed: RoomBrowserControllerCommand = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "2d00dc9bcd6fc91a589f9074cf10d840e8c1538ccdeca042256f8058565c3443"
    );
    let wire = serde_json::json!({"kind":"upload","execution_id":"action-a","target_id":"tab-target","document_id":"doc-a","node_ref":"node-a","files":{"artifacts":[{"display_name":"report.txt","mime_type":"text/plain","data_base64":"aGk=","sha256":"8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4","size_bytes":2}]}});
    let typed: RoomBrowserControllerCommand = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "890a1cc772ca8e6faeb7171830899a604d9c1fef19c38085548ad2b3d4adf680"
    );
    let wire = serde_json::json!({"kind":"artifact","capture":null});
    let typed: RoomBrowserControllerResult = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "1d29f49720cfdf93e3da6b3ec5fab5ca90a2e55db892882517c4e3eb59146036"
    );
    let wire = serde_json::json!({"kind":"artifact","capture":{"display_name":"browser-network.har","mime_type":"application/json","data_base64":"e30=","sha256":"44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size_bytes":2,"kind":"network","guid":null,"target_id":"tab-target","document_id":"doc-a","browser_generation":1,"browser_id":"browser-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","viewport":{"css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800,"revision":1,"last_actor_id":null},"redaction":"metadata_only","geometry":{"pageX":0,"pageY":0,"clientWidth":1280,"clientHeight":800,"scale":1}}});
    let typed: RoomBrowserControllerResult = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "ee34defc09ac9f87b9a36a96b451660c9cd26e98e0d958e795111e7bdb67820f"
    );
}

// MP-08/MP-10/MP-11: the integration contract contains Apps access, Browser
// artifact, Computer hold and public provider-run shapes together; unreleased per-lane numbers fold.
#[test]
fn mp08_mp10_mp11_apps_browser_computer_union_484_82_is_hashed() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        82
    );
    let snapshot = serde_json::json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"peer_protocol":82,"requests":[{"RoomBrowserArtifact":{"attachment_id":"client","operation":{"action":"capture","browser_generation":1,"guid":null,"kind":"image","return_image_base64":true},"session_id":"room","tab_id":"tab"}},{"SubmitRoomEnvironmentAction":{"action":{"duration_ms":750,"key":"shift+Left","kind":"keyboard_hold"},"idempotency_key":"hold","runtime_generation":2,"session_id":"room","viewport_revision":3}}],"responses":[{"KernelSudoRequested":{"agent_id":"agent"}},{"ProviderRun":{"provider_run":{"account_profile":"default","adapter_key":"codex","agent_instance_id":"agent-public","client_interface":"chariox","control_capabilities":[{"mode":"native","operation":"interrupt_turn"},{"mode":"native","operation":"cancel_prompt"},{"mode":"mcp","operation":"ack_workflow_turn"},{"mode":"mcp","operation":"validate_workflow_handoff"}],"endpoint_mode":"Managed","execution_mode":"build","id":"run-public","last_activity_at_ms":2,"model":"","owner_user_id":"local","permission_level":"yolo","provider":"codex","provider_session_id":null,"session_id":"session-public","started_at_ms":1,"state":"Starting","structured_endpoint":"http://127.0.0.1:1234","usage_tokens_total":null,"variant":null,"working_directory":null}}}]});
    for request in snapshot["requests"].as_array().unwrap() {
        let typed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *request);
    }
    for response in snapshot["responses"].as_array().unwrap() {
        let typed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *response);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "e2ecaccf5611b49605d8a4290a55c1a779b4c6fcaff7f077c9e348c8e615edfe"
    );
}
