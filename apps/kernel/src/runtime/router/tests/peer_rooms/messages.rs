use super::*;

#[test]
fn a_message_reaches_an_agent_of_another_room_once_with_the_sender_stamped() {
    run_peer_test("peer-room-message", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let args = serde_json::json!({
            "agent": "peer",
            "room": rooms.peer_room,
            "message": "compare notes on the parser",
            "idempotency_key": "notes-1",
        });
        let first = rooms
            .send(args.clone())
            .await
            .expect("message across rooms");
        let retry = rooms
            .send(args)
            .await
            .expect("a retry is the same delivery");
        assert_eq!(first["sequence"], retry["sequence"]);
        let delivered = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(delivered.len(), 1, "one delivery per idempotency key");
        assert_eq!(delivered[0].source_id, rooms.sender.id());
        assert!(rooms.inbox(&rooms.home, rooms.peer.id()).await.is_empty());

        let view = rooms
            .router
            .runtime_state
            .read_agent_inbox(&rooms.peer_room, rooms.peer.id(), &serde_json::json!({}))
            .expect("the peer reads its inbox");
        let event = view["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["kind"] == "message")
            .expect("the message is in the peer's inbox")
            .clone();
        let kernel_id = rooms.app.lock().await.config().daemon_id.clone();
        assert_eq!(event["source"]["id"], rooms.sender.id());
        assert_eq!(event["source"]["room"], rooms.home);
        assert_eq!(event["source"]["kernel_id"], kernel_id);
        assert_eq!(event["source"]["relation"], "peer");
        assert_eq!(event["payload"]["message"], "compare notes on the parser");

        // The text reaches the peer only as its agent-event delivery, never as a user prompt.
        let session = rooms
            .app
            .lock()
            .await
            .sessions()
            .get_session(&rooms.peer_room)
            .unwrap();
        assert!(session
            .queued_prompts()
            .iter()
            .filter(|prompt| prompt.prompt().contains("compare notes"))
            .all(|prompt| prompt.id().starts_with("agent-event-")));
    });
}

#[test]
fn hidden_sub_agents_and_other_owners_rooms_are_unavailable() {
    run_peer_test("peer-room-refusals", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        for (room, agent) in [
            (&rooms.peer_room, rooms.hidden.id()),
            (&rooms.foreign_room, rooms.foreign.id()),
            (&rooms.peer_room, "nobody"),
        ] {
            let error = rooms
                .send(serde_json::json!({"agent": agent, "room": room, "message": "hi"}))
                .await
                .expect_err("not addressable");
            assert!(
                error
                    .to_string()
                    .contains("agent is unavailable in that room"),
                "{error}"
            );
        }
        // Without `room` the target is looked up in the sender's room only.
        assert!(rooms
            .send(serde_json::json!({"agent": rooms.peer.id(), "message": "hi"}))
            .await
            .is_err());
        assert!(rooms
            .inbox(&rooms.peer_room, rooms.hidden.id())
            .await
            .is_empty());
        assert!(rooms
            .inbox(&rooms.foreign_room, rooms.foreign.id())
            .await
            .is_empty());
    });
}
