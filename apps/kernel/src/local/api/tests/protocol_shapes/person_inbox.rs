//! Local 1012: the person's Inbox (docs/NOTIFICATION_CENTER_PLAN.md section 4): its cards, read, snooze and mute.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn person_inbox_1012_snapshot_and_hash() {
    let bytes = include_bytes!("person-inbox-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "3735550b27c62e24305346d2f465180d27a907aaac114751b6abf6f25a9cad89"
    );
}

#[test]
fn person_inbox_round_trips_registered_cards_and_rejects_retired_card_bodies() {
    let bytes = include_bytes!("person-inbox-1012.json");
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
    // Workflow results and subscription health still belong to their owning views,
    // but their retired Inbox bodies must never re-enter the wire card list.
    for body in [
        json!({"workflow":{"workflow_id":"wf-1","workflow_name":"nightly_deploy","run_id":"run-7","run_number":7,"outcome":{"kind":"failed","reason":"workflow_run_output_validation_failed","detail":"ticket is missing"},"at_ms":11}}),
        json!({"flag":{"subscription_id":"sub-1","subscriber_name":"Reviewer","source_name":null,"health":"source_gone","at_ms":null}}),
    ] {
        let retired = json!({"PersonInbox":{"cards":[{"card_id":"retired","session_id":"room-1","snoozed_until_ms":null,"body":body}],"mutes":[]}});
        assert!(serde_json::from_value::<LocalDaemonResponse>(retired).is_err());
    }
    // A card's state is the kernel's: a request names cards and a change, nothing else.
    for forged in [
        json!({"SetInboxCardState":{"card_ids":["n"],"change":{"read":{"read":true}},"owner":"someone-else"}}),
        json!({"SetInboxCardState":{"card_ids":["n"],"change":{"mute":{}}}}),
        json!({"SetInboxMute":{"origin":{"session_id":"room-1"},"muted":true}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
