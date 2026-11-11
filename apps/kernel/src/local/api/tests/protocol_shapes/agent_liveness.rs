//! What an agent waits on and the agents the health report flags are part of the client wire.
use crate::runtime::projection::{AgentRuntimeActivity, AgentRuntimeProjectionHealthSnapshot};
use sha2::{Digest, Sha256};

#[test]
fn populated_agent_liveness_and_silent_agents_protocol_snapshot_and_hash() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("agent-liveness.json")).unwrap();
    let activity: AgentRuntimeActivity =
        serde_json::from_value(expected["activity"].clone()).unwrap();
    let health: AgentRuntimeProjectionHealthSnapshot =
        serde_json::from_value(expected["health"].clone()).unwrap();
    let actual = serde_json::json!({"activity": activity, "health": health});
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "779368333283c6ba95ed8d8859d90c17ef81c0925a2ec27a1e6e8553bf0ca9e9"
    );
}
