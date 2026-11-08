use super::*;

#[test]
fn a_peer_in_another_room_steers_and_cancels_with_equal_authority() {
    run_peer_test("peer-room-control", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        rooms
            .send(serde_json::json!({
                "agent": "peer",
                "room": rooms.peer_room,
                "message": "stop editing parser.rs, I am on it",
                "urgent": true,
            }))
            .await
            .expect("a peer may steer");
        let steer = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(steer.len(), 1);
        assert!(steer[0].urgent);
        assert_eq!(steer[0].source_id, rooms.sender.id());

        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        let cancel = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel peer --room {}", rooms.peer_room),
            )
            .await;
        assert!(cancel.ok, "{}", cancel.payload);
        assert!(!rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
    });
}

#[test]
fn a_peer_with_less_authority_cannot_steer_or_cancel() {
    run_peer_test("peer-room-control-clamp", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Required, AgentPermissionLevel::Yolo);
        let error = rooms
            .send(serde_json::json!({
                "agent": "peer",
                "room": rooms.peer_room,
                "message": "stop",
                "urgent": true,
            }))
            .await
            .expect_err("a Required agent cannot steer a Yolo peer");
        assert!(error.to_string().contains("more authority"), "{error}");
        rooms
            .send(serde_json::json!({"agent": "peer", "room": rooms.peer_room, "message": "fyi"}))
            .await
            .expect("an ordinary message needs no authority");
        let cancel = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel peer --room {}", rooms.peer_room),
            )
            .await;
        assert!(!cancel.ok);
        assert!(
            cancel.payload.to_string().contains("more authority"),
            "{}",
            cancel.payload
        );
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        let foreign = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel foreign --room {}", rooms.foreign_room),
            )
            .await;
        assert!(!foreign.ok);
        assert!(
            foreign
                .payload
                .to_string()
                .contains("agent is unavailable in that room"),
            "{}",
            foreign.payload
        );
    });
}

#[test]
fn a_same_room_peer_may_cancel_but_a_sub_agent_never_cancels_its_parent() {
    run_peer_test("peer-room-control-tree", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let (neighbour, child, child_token) = {
            let mut app = rooms.app.lock().await;
            let neighbour = spawn_with_level(
                &mut app,
                &rooms.home,
                "neighbour",
                AgentPermissionLevel::Yolo,
            );
            running_turn(&mut app, &rooms.home, neighbour.id());
            let child = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(&rooms.home, "dev-stub")
                        .with_alias("child")
                        .with_parent_agent_id(rooms.sender.id()),
                )
                .expect("child should spawn");
            let (child_token, _) = running_turn(&mut app, &rooms.home, child.id());
            (neighbour, child, child_token)
        };
        let up = rooms
            .room_command(&child_token, "agent cancel sender".to_string())
            .await;
        assert!(
            !up.ok,
            "a sub-agent never controls its parent: {}",
            up.payload
        );
        assert!(rooms.turn_running(&rooms.home, rooms.sender.id()).await);
        let across = rooms
            .room_command(&rooms.sender_token, "agent cancel neighbour".to_string())
            .await;
        assert!(across.ok, "{}", across.payload);
        assert!(!rooms.turn_running(&rooms.home, neighbour.id()).await);
        assert!(rooms.turn_running(&rooms.home, child.id()).await);
    });
}
