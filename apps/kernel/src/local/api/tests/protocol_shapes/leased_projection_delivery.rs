// MP-07 / MP-08 / MP-10 / MP-11: extension of the allocated531/139 contract.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RelayPeerEvent};
use sha2::{Digest, Sha256};
#[test]
fn mp08_projection_delivery_531_139_snapshot_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 531);
    assert_eq!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION, 139);
    let mut snapshot: serde_json::Value = serde_json::from_str(r###"{"event":{"completions":[],"delivery":{"execution_lease_id":"lease","leased_agent_id":"leased","sequence":1},"home_agent_id":"agent","home_session_id":"session","kind":"leased_runtime_projection","notices":[],"output_chunks":[],"provider_run_id":"run"},"local_protocol":531,"peer_protocol":139,"request":{"delivery":{"execution_lease_id":"lease","leased_agent_id":"leased","sequence":1},"kind":"acknowledge_leased_runtime_projection","leased_agent_id":"leased","provider_run_id":"run"},"response":{"accepted":true,"kind":"leased_runtime_projection_acknowledged"}}"###).unwrap();
    // MP-08: field ownership is part of the unmerged 531/139 tool payload.
    snapshot["event"]["output_chunks"] = serde_json::json!([{
        "kind": "provider_tool", "merge_key": "tool",
        "bytes": serde_json::to_vec(&serde_json::json!({
            "stdout": "tail", "stderr": "", "chariox_delta": true,
            "chariox_delta_offset_bytes": 50, "chariox_delta_field": "stdout",
            "chariox_field_bytes": {"stdout": 30000},
        })).unwrap(),
    }]);
    let request: RelayPeerRequest = serde_json::from_value(snapshot["request"].clone()).unwrap();
    let response: RelayPeerResponse = serde_json::from_value(snapshot["response"].clone()).unwrap();
    let event: RelayPeerEvent = serde_json::from_value(snapshot["event"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), snapshot["request"]);
    assert_eq!(serde_json::to_value(response).unwrap(), snapshot["response"]);
    assert_eq!(serde_json::to_value(event).unwrap(), snapshot["event"]);
    assert_eq!(format!("{:x}", Sha256::digest(serde_json::to_vec(&snapshot).unwrap())), "746373ac47415df37e53dfcc8dcfbceda84ec2814707b9f2c0b6d2f79b06fd26");
    for missing in ["delivery", "provider_run_id", "leased_agent_id"] {
        let mut old = snapshot["request"].clone(); old.as_object_mut().unwrap().remove(missing);
        assert!(serde_json::from_value::<RelayPeerRequest>(old).is_err());
    }
    for missing in ["execution_lease_id", "leased_agent_id", "sequence"] {
        let mut old = snapshot["request"].clone(); old["delivery"].as_object_mut().unwrap().remove(missing);
        assert!(serde_json::from_value::<RelayPeerRequest>(old).is_err());
    }
}
