use super::*;

#[test]
fn a_request_read_by_a_busy_peer_does_not_reply_with_that_turns_answer() {
    run_peer_test("peer-room-unrelated-answer", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        {
            let app = rooms.app.lock().await;
            let session = app.sessions().get_session(&rooms.peer_room).unwrap();
            let prompt = session.active_prompt_for_agent(rooms.peer.id()).unwrap();
            let run = app
                .providers()
                .get_run_for_agent(&rooms.peer_room, rooms.peer.id())
                .unwrap();
            app.durable_state_store()
                .agent_lifecycle(crate::durable_state::agent_lifecycle::Operation::Begin {
                    owner: session.owner_user_id().into(),
                    room: rooms.peer_room.clone(),
                    agent: rooms.peer.id().into(),
                    prompt: prompt.id().into(),
                    run: Some(run.id().into()),
                    now: crate::session::unix_epoch_ms(),
                })
                .unwrap();
        }
        let sent = rooms
            .send(serde_json::json!({
                "agent": "peer",
                "room": rooms.peer_room,
                "message": "What is the answer to topic X?",
                "reply_requested": true,
            }))
            .await
            .unwrap();
        let sequence = sent["sequence"].as_u64().unwrap();
        let runtime = &rooms.router.runtime_state;
        let read = runtime
            .read_agent_inbox(
                &rooms.peer_room,
                rooms.peer.id(),
                &serde_json::json!({"sequence": sequence}),
            )
            .unwrap();
        assert_eq!(
            read["events"][0]["payload"]["message"],
            "What is the answer to topic X?"
        );
        runtime
            .ack_agent_inbox(
                &rooms.peer_room,
                rooms.peer.id(),
                &serde_json::json!({"sequence": sequence, "handled": true}),
                crate::session::unix_epoch_ms(),
            )
            .await
            .unwrap();
        runtime
            .end_agent_turn_for_tests(&rooms.peer_room, rooms.peer.id(), false)
            .await
            .unwrap();
        let source = format!("agent-event-{}-{sequence}", rooms.peer.id());
        let inbox = rooms
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_inbox(&rooms.home, rooms.sender.id(), 0)
            .unwrap();
        let replies: Vec<_> = inbox
            .iter()
            .filter(|event| event.source_id == source)
            .collect();
        assert_eq!(replies.len(), 1, "{inbox:?}");
        assert_eq!(replies[0].kind, "source_lost", "{replies:?}");
        let answer = &replies[0].payload["public_answer"];
        assert_eq!(answer["kind"], "no_answer", "{answer}");
        assert!(answer["reason"].is_string(), "{answer}");
        assert!(answer.get("prompt_id").is_none(), "{answer}");
    });
}

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
