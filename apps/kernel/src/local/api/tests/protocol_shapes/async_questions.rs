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
        "2c88fbabfa78d25a206305a9dbcd6df5f3eeba4a3a716d525428451c058c361e"
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
        "faf4270129475210f2ff98e6c291938368a031d3b4dc62ea647da6596827a6c8"
    );
}
