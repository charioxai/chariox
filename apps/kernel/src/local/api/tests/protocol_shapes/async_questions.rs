//! MP-08 / MP-09 / MP-10 / MP-11 AM-ASK-06: additive question and inbox contract.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn async_popup_returns_protocol_954_403_snapshot_and_hash() {
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/async-questions-protocol.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::json!(LOCAL_DAEMON_PROTOCOL_VERSION),
        snapshot["local"]
    );
    assert_eq!(
        serde_json::json!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
        snapshot["relayPeer"]
    );
    let args: crate::transport::runtime_tools::RequestPopupArgs =
        serde_json::from_value(snapshot["arguments"].clone()).unwrap();
    assert_eq!(serde_json::to_value(args).unwrap(), snapshot["arguments"]);
    // asking_task_id is additive; RespondToInteraction remains the answer request.
    let interaction: crate::session::RuntimeInteraction =
        serde_json::from_value(snapshot["interaction"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&interaction).unwrap(),
        snapshot["interaction"]
    );
    let response: LocalDaemonRequest = serde_json::from_value(snapshot["answer"].clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), snapshot["answer"]);
    let inbox: crate::durable_state::agent_lifecycle::InboxEvent =
        serde_json::from_value(snapshot["event"].clone()).unwrap();
    assert_eq!(serde_json::to_value(inbox).unwrap(), snapshot["event"]);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "e0bc160414ec6cc6e161ccd07266360fa5b459cd38b7f4dce9ba921238e1042f"
    );
    // An old client ignores the additive ownership field; its answer uses
    // exactly the pre-existing request shape.
    let mut legacy = snapshot["interaction"].clone();
    legacy.as_object_mut().unwrap().remove("asking_task_id");
    let legacy: crate::session::RuntimeInteraction = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.choices(), interaction.choices());
}
