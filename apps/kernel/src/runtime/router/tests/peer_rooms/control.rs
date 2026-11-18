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
fn a_message_after_a_peer_cancel_runs_on_the_cancelled_agent() {
    run_peer_test("peer-room-cancel-then-message", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let cancel = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel peer --room {}", rooms.peer_room),
            )
            .await;
        assert!(cancel.ok, "{}", cancel.payload);
        rooms.settle_cancelled_turn().await;
        rooms
            .send(serde_json::json!({
                "agent": "peer",
                "room": rooms.peer_room,
                "message": "take over parser.rs",
                "urgent": true,
            }))
            .await
            .expect("a peer may message");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, rooms.peer.id())
                .await,
            "a cancel ends the current task, not the agent: the later message runs"
        );
    });
}

#[test]
fn an_owner_stop_in_flight_is_not_turned_into_an_agent_cancel() {
    run_peer_test("peer-room-owner-stop-then-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        rooms.owner_stop(&rooms.peer_room, rooms.peer.id()).await;
        let cancel = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel peer --room {}", rooms.peer_room),
            )
            .await;
        assert!(cancel.ok, "{}", cancel.payload);
        rooms
            .router
            .runtime_state
            .end_agent_turn_for_tests(&rooms.peer_room, rooms.peer.id(), true)
            .await
            .expect("the stopped turn settles");
        assert_eq!(
            rooms
                .last_task(&rooms.peer_room, rooms.peer.id())
                .await
                .reason,
            "Stopped by you"
        );
        rooms
            .send(serde_json::json!({
                "agent": "peer",
                "room": rooms.peer_room,
                "message": "take over parser.rs",
            }))
            .await
            .expect("a peer may message");
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, rooms.peer.id())
                .await,
            "the owner's Stop wins: the message stays held until the owner prompts the agent"
        );
    });
}

#[test]
fn a_message_after_a_peer_cancel_of_a_parked_task_runs() {
    run_peer_test("peer-room-cancel-parked-then-message", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let parked = {
            let mut app = rooms.app.lock().await;
            let agent = spawn_with_level(
                &mut app,
                &rooms.peer_room,
                "parked",
                AgentPermissionLevel::Yolo,
            );
            running_turn(&mut app, &rooms.peer_room, agent.id());
            agent
        };
        rooms.park_task(&rooms.peer_room, parked.id()).await;
        let cancel = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent cancel parked --room {}", rooms.peer_room),
            )
            .await;
        assert!(cancel.ok, "{}", cancel.payload);
        assert_eq!(
            rooms.last_task(&rooms.peer_room, parked.id()).await.reason,
            "Stopped by an agent"
        );
        rooms
            .send(serde_json::json!({
                "agent": "parked",
                "room": rooms.peer_room,
                "message": "take over parser.rs",
                "urgent": true,
            }))
            .await
            .expect("a peer may message");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, parked.id())
                .await,
            "a cancel ends the parked task, not the agent: the later message runs"
        );
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

#[test]
fn the_message_and_steer_room_commands_reach_a_peer_like_the_tool() {
    run_peer_test("peer-room-message-steer", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let message = rooms
            .room_command(
                &rooms.sender_token,
                format!(
                    "agent message peer \"parser.rs is mine\" --room {} --reply-requested",
                    rooms.peer_room
                ),
            )
            .await;
        assert!(message.ok, "{}", message.payload);
        let steer = rooms
            .room_command(
                &rooms.sender_token,
                format!(
                    "agent steer peer \"stop editing\" --room {}",
                    rooms.peer_room
                ),
            )
            .await;
        assert!(steer.ok, "{}", steer.payload);
        let inbox = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(inbox.len(), 2, "{inbox:?}");
        assert!(!inbox[0].urgent && inbox[0].reply_requested);
        assert!(inbox[1].urgent && !inbox[1].reply_requested);
        for event in &inbox {
            assert_eq!(event.source_id, rooms.sender.id());
            assert_eq!(event.payload["sender_room_id"], rooms.home.as_str());
        }
        assert!(
            rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await,
            "a steer is an inbox message, never a user prompt"
        );
    });
}

#[test]
fn a_steer_can_ask_for_the_reply_like_a_message() {
    run_peer_test("peer-room-steer-reply", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let steer = rooms
            .room_command(
                &rooms.sender_token,
                format!(
                    "agent steer peer \"stop editing\" --room {} --reply-requested",
                    rooms.peer_room
                ),
            )
            .await;
        assert!(steer.ok, "{}", steer.payload);
        let inbox = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(inbox.len(), 1, "{inbox:?}");
        assert!(inbox[0].urgent && inbox[0].reply_requested, "{inbox:?}");
    });
}

#[test]
fn the_steer_room_command_keeps_the_authority_limit_and_the_message_one_does_not_have_it() {
    run_peer_test("peer-room-steer-clamp", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Required, AgentPermissionLevel::Yolo);
        let steer = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent steer peer stop --room {}", rooms.peer_room),
            )
            .await;
        assert!(!steer.ok);
        let text = steer.payload.to_string();
        assert!(
            text.contains("more authority") && text.contains("send it a message instead"),
            "{text}"
        );
        let foreign = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent message foreign hi --room {}", rooms.foreign_room),
            )
            .await;
        assert!(!foreign.ok);
        assert!(
            foreign.payload.to_string().contains("unavailable"),
            "{}",
            foreign.payload
        );
        let message = rooms
            .room_command(
                &rooms.sender_token,
                format!("agent message peer fyi --room {}", rooms.peer_room),
            )
            .await;
        assert!(message.ok, "{}", message.payload);
        assert_eq!(
            rooms.inbox(&rooms.peer_room, rooms.peer.id()).await.len(),
            1
        );
        for bad in ["agent steer peer", "agent message", "agent steer peer a b"] {
            let usage = rooms
                .room_command(&rooms.sender_token, bad.to_string())
                .await;
            assert!(
                !usage.ok && usage.payload.to_string().contains("usage:"),
                "{bad}: {}",
                usage.payload
            );
            assert!(
                usage
                    .payload
                    .to_string()
                    .contains("the text is one quoted argument"),
                "{bad}: {}",
                usage.payload
            );
        }
    });
}

#[test]
fn kwait_cancelling_an_idle_peer_is_a_successful_noop() {
    run_peer_test("kwait-idle-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        rooms
            .router
            .runtime_state
            .end_agent_turn_for_tests(&rooms.peer_room, rooms.peer.id(), true)
            .await
            .unwrap();
        assert!(
            !rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await,
            "the target must be idle before cancellation"
        );
        for _ in 0..2 {
            let result = rooms
                .room_command(
                    &rooms.sender_token,
                    format!("agent cancel peer --room {}", rooms.peer_room),
                )
                .await;
            assert!(result.ok, "{}", result.payload);
            assert_eq!(
                result.payload["response"]["already_idle"], true,
                "{}",
                result.payload
            );
            assert!(!rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        }
    });
}
