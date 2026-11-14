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
        "ad28263a191bdf67afbc0582b83115338507653e76d528410cd03f04a080f4e2"
    );
    // An old client ignores the additive ownership field; its answer uses
    // exactly the pre-existing request shape.
    let mut legacy = snapshot["interaction"].clone();
    legacy.as_object_mut().unwrap().remove("asking_task_id");
    let legacy: crate::session::RuntimeInteraction = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.choices(), interaction.choices());
}

// Person cards: a card follows a wake source or carries one pre-bound request.
#[test]
fn person_card_arguments_and_interaction_keep_their_shape() {
    let arguments = serde_json::json!({
        "wait": false,
        "message": "Approve the deploy key",
        "title": "Deploy key",
        "choices": [
            {
                "id": "done",
                "label": "Done",
                "reply": "done"
            }
        ],
        "until": {
            "wake_id": "wake-1"
        },
        "action": {
            "request": {
                "AliasSession": {
                    "session_id": "s-1",
                    "alias": "nightly"
                }
            }
        },
        "view": {
            "browser_region": {
                "tab_id": "tab-1",
                "ref": "backend:7"
            }
        }
    });
    let args: crate::transport::runtime_tools::RequestPopupArgs =
        serde_json::from_value(arguments.clone()).unwrap();
    assert_eq!(serde_json::to_value(args).unwrap(), arguments);
    let unknown = serde_json::json!({"wake_id": "wake-1", "also": true});
    assert!(
        serde_json::from_value::<crate::transport::runtime_tools::RequestPopupUntilArgs>(unknown)
            .is_err()
    );
    let interaction = serde_json::json!({
        "closes_with_wake": "wake-1",
        "id": "interaction-1",
        "agent_id": "agent-1",
        "kind": "choice",
        "level": "info",
        "message": "Finish the sign-in",
        "choices": [
            {
                "id": "done",
                "label": "Done",
                "reply": "done"
            }
        ],
        "requested_at_ms": 1,
        "view": {
            "browser_region": {
                "tab_id": "tab-1",
                "ref": "backend:7",
                "generation": 1,
                "document_id": "document-1",
                "origin": "https://www.wikipedia.org",
                "rect": {
                    "x": 16,
                    "y": 24,
                    "width": 320,
                    "height": 80
                },
                "viewport_width": 1280,
                "viewport_height": 800
            }
        }
    });
    let card: crate::session::RuntimeInteraction =
        serde_json::from_value(interaction.clone()).unwrap();
    assert_eq!(card.closes_with_wake(), Some("wake-1"));
    assert_eq!(serde_json::to_value(&card).unwrap(), interaction);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&serde_json::json!([arguments, interaction])).unwrap()
            )
        ),
        "cb56dc70bbfc839aace0d66435a827cdc080b1b64cc30756c7d091724c32196d"
    );
}

// Person cards (protocol 1006): a sub-agent's card names the panel that shows it and its asker.
#[test]
fn a_sub_agent_card_carries_its_panel_and_asker_name() {
    let interaction = serde_json::json!({
        "shown_in_agent_id": "agent-room",
        "asker_name": "worker-1",
        "id": "interaction-2",
        "agent_id": "agent-child",
        "kind": "choice",
        "level": "info",
        "message": "Child asks: which fruit",
        "choices": [
            {
                "id": "apple",
                "label": "Apple",
                "reply": "apple"
            }
        ],
        "requested_at_ms": 1
    });
    let card: crate::session::RuntimeInteraction =
        serde_json::from_value(interaction.clone()).unwrap();
    assert_eq!(card.asker_name(), Some("worker-1"));
    assert_eq!(card.shown_in_agent_id(), Some("agent-room"));
    assert_eq!(serde_json::to_value(&card).unwrap(), interaction);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&interaction).unwrap())
        ),
        "445199b2803142f0936fb2a5d18d929e14995d9969c23f8f674395426106af24"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: verification metadata never carries page values.
#[test]
fn browser_verification_protocol_992_431_snapshot_and_hash() {
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("browser-verification-992.json")).unwrap();
    assert_eq!(snapshot["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        snapshot["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let args: crate::transport::runtime_tools::RequestPopupArgs =
        serde_json::from_value(snapshot["arguments"].clone()).unwrap();
    assert_eq!(serde_json::to_value(args).unwrap(), snapshot["arguments"]);
    let region: crate::session::RuntimeInteractionBrowserRegion =
        serde_json::from_value(snapshot["region"].clone()).unwrap();
    assert_eq!(serde_json::to_value(region).unwrap(), snapshot["region"]);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "d366aec4f219705cc0de61304bf12f7a7dc0ec559a045308493edc66f80e5b77"
    );
}
