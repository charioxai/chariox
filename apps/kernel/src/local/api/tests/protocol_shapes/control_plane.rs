use super::*;
use sha2::{Digest, Sha256};
#[test]
fn mp08_mp11_control_plane_959_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let snapshot = serde_json::json!({"requests":[{"ControlPlane":{"action":"capabilities"}},{"ControlPlane":{"action":"acquire","capability_revision":"revision","request_id":"one"}},{"ControlPlane":{"action":"list"}},{"ControlPlane":{"action":"get","allocation_ref":"resource"}},{"ControlPlane":{"action":"release","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"start","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"stop","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"start_saved_state","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1,"saved_state_ref":"saved-record"}},{"ControlPlane":{"action":"delete_saved_state","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1,"saved_state_ref":"saved-record"}}],"responses":[{"ControlPlane":{"result":{"kind":"capabilities","machine_allocation":null}}},{"ControlPlane":{"result":{"kind":"capabilities","machine_allocation":{"revision":"revision","actions":["acquire","list"],"offer":{"machine_type":"Standard","vcpu":2,"ram_gb":4.0,"disk_gb":40,"location":"Europe","price":{"amount":"0.01","currency":"EUR","period":"hour"}}}}}},{"ControlPlane":{"result":{"kind":"machines","machines":[{"allocation_ref":"resource","name":"Computer","state":"ready","revision":1,"since":"2026-11-02T00:00:00Z","kernel_id":"kernel","machine_id":"machine","available_actions":["stop","release"]}],"saved_states":[]}}},{"ControlPlane":{"result":{"kind":"machine","allocation_ref":"resource","revision":1,"available_actions":["stop","release"],"state":"ready","access_state":"available","access_reason":null,"kernel_target":{"kernel_id":"kernel","machine_id":"machine","relay_realm_id":"realm","key_thumbprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}]});
    for shape in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *shape);
    }
    for shape in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *shape);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "782babe50f8bfc4e131c5cb753511a61df8fd22eeab4690f346573190a873e20"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ControlPlane":{"action":"capabilities","token":"must-not-travel"}})
    )
    .is_err());
}
