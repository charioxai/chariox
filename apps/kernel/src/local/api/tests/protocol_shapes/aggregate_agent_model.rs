//! MP-08/MP-10/MP-11: main 520/128 and agent-model 513/121 share one wire contract.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_aggregate_agent_model_protocol_snapshot_and_hash() {
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/aggregate-agent-model-protocol.json"
    ))
    .unwrap();
    assert_eq!(
        snapshot["local"],
        serde_json::json!(LOCAL_DAEMON_PROTOCOL_VERSION)
    );
    assert_eq!(
        snapshot["relayPeer"],
        serde_json::json!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION)
    );
    // This fixture is deliberately not generated from the runtime constants.
    // The hash binds the allocation and both parent contracts together.
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "b0f4461a6864156348e987e1f976286d8ac78770a1f4819cb064825ef906cfd9"
    );
    for wire in snapshot["requests"].as_array().unwrap() {
        let typed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *wire);
    }
    for wire in snapshot["responses"].as_array().unwrap() {
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *wire);
    }
    for wire in snapshot["peerRequests"].as_array().unwrap() {
        let typed: crate::transport::relay_peer::RelayPeerRequest =
            serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *wire);
    }
    for wire in snapshot["events"].as_array().unwrap() {
        let typed: crate::transport::kernel_protocol::KernelEvent =
            serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *wire);
    }
    for wire in snapshot["replyOutcomes"].as_array().unwrap() {
        let typed: crate::durable_state::agent_lifecycle::ReplyOutcome =
            serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *wire);
    }
}
