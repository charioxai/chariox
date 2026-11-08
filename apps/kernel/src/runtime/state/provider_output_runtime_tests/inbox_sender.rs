//! The inbox says what the sender of each event is, from the agent tree: an instruction from the
//! reader's parent, a report from its sub-agent, information from a peer.
use super::child_finished::family;
use crate::durable_state::agent_lifecycle::{occurrence, Operation};

fn send(team: &super::child_finished::Family, to: &str, from: &str, id: &str) {
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(occurrence(
            &team.room,
            to,
            from,
            id,
            "message",
            serde_json::json!({"message": "write DELTA into steer.txt"}),
        )))
        .unwrap();
}

fn read(team: &super::child_finished::Family, reader: &str) -> serde_json::Value {
    team.runtime
        .read_agent_inbox(&team.room, reader, &serde_json::json!({}))
        .unwrap()
}

#[tokio::test]
async fn a_message_from_the_parent_is_an_instruction_and_never_claims_the_users_authority() {
    let team = family("inbox-sender-parent", 2, None, true).await;
    let (first, second) = (&team.kids[0].child, &team.kids[1].child);
    send(&team, first, &team.parent, "from-parent");
    send(&team, first, second, "from-sibling");
    send(&team, &team.parent, first, "from-child");
    let child_inbox = read(&team, first);
    let events = child_inbox["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "{child_inbox}");
    let parent_event = &events[0];
    assert_eq!(parent_event["source"]["relation"], "ancestor");
    let text = parent_event["guidance"].as_str().unwrap();
    assert!(
        text.contains("your parent agent")
            && text.contains("Act on it like any request from the room agent"),
        "{text}"
    );
    let sibling_event = &events[1];
    assert_eq!(sibling_event["source"]["relation"], "peer");
    let text = sibling_event["guidance"].as_str().unwrap();
    assert!(
        text.contains("neither above nor below you") && text.contains("at your own judgment"),
        "{text}"
    );
    for event in events {
        let text = event["guidance"].as_str().unwrap().to_lowercase();
        assert!(!text.contains("user") && !text.contains("error"), "{text}");
    }
    assert_eq!(
        child_inbox["untrusted"], true,
        "the sibling's event is information"
    );

    let parent_inbox = read(&team, &team.parent);
    let report = &parent_inbox["events"][0];
    assert_eq!(report["source"]["relation"], "descendant");
    assert!(
        report["guidance"].as_str().unwrap().contains("a report"),
        "{report}"
    );
}

#[tokio::test]
async fn an_inbox_of_instructions_only_is_not_marked_untrusted() {
    let team = family("inbox-sender-only-parent", 1, None, true).await;
    let child = &team.kids[0].child;
    send(&team, child, &team.parent, "from-parent");
    let inbox = read(&team, child);
    assert_eq!(inbox["untrusted"], false, "{inbox}");
}

#[tokio::test]
async fn a_message_from_an_agent_on_another_kernel_is_a_peer_with_the_kernel_that_stamped_it() {
    let team = family("inbox-sender-remote", 1, None, true).await;
    let reader = &team.kids[0].child;
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(occurrence(
            &team.room,
            reader,
            "peer:kernel-b:agent-x",
            "kernel-b:request-1",
            "message",
            serde_json::json!({"message": "parser.rs is mine", "sender_agent_id": "agent-x", "sender_room_id": "room-x", "sender_kernel_id": "kernel-b", "no_reply": true}),
        )))
        .unwrap();
    let event = &read(&team, reader)["events"][0];
    assert_eq!(event["source"]["kind"], "agent", "{event}");
    assert_eq!(event["source"]["relation"], "peer", "{event}");
    assert_eq!(event["source"]["kernel_id"], "kernel-b", "{event}");
    assert_eq!(event["source"]["room"], "room-x", "{event}");
}
