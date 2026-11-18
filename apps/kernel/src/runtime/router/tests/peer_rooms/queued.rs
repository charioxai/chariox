//! A message that is not urgent waits in its receiver's queue and is delivered in the receiver's
//! next turn; only an urgent one steers a running turn or starts one. The sender can promote its
//! queued message, and the person cancels it or sends it from the receiver's queue.
use super::*;

const QUEUED_NOTE: &str =
    "queued for idle's next turn; send with urgent: true to start or steer now";

async fn spawn_idle(rooms: &Rooms) -> crate::agent::AgentInstance {
    let mut app = rooms.app.lock().await;
    spawn_with_level(
        &mut app,
        &rooms.peer_room,
        "idle",
        AgentPermissionLevel::Yolo,
    )
}

async fn send_to(rooms: &Rooms, agent: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut args = serde_json::json!({
        "agent": agent,
        "room": rooms.peer_room,
        "message": "compare notes on the parser",
    });
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    rooms.send(args).await.expect("message across rooms")
}

async fn stripe(rooms: &Rooms) -> Vec<crate::session::QueuedAgentMessage> {
    rooms
        .router
        .runtime_state
        .session_snapshot(&rooms.peer_room)
        .await
        .expect("the room snapshot")
        .queued_agent_messages()
        .to_vec()
}

async fn states(rooms: &Rooms, agent: &str) -> Vec<String> {
    rooms
        .inbox(&rooms.peer_room, agent)
        .await
        .into_iter()
        .map(|event| event.state)
        .collect()
}

#[test]
fn a_message_to_an_idle_agent_queues_and_starts_no_turn() {
    run_peer_test("queued-idle", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        assert_eq!(sent["status"], "queued");
        assert_eq!(sent["note"], QUEUED_NOTE);
        assert!(sent["message_id"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(
            !rooms.turn_running(&rooms.peer_room, idle.id()).await,
            "a queued message starts nothing"
        );
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        let stripe = stripe(&rooms).await;
        assert_eq!(stripe.len(), 1);
        assert_eq!(stripe[0].agent_id, idle.id());
        assert_eq!(stripe[0].sender, "sender");
        assert_eq!(stripe[0].prompt, "compare notes on the parser");
        assert_eq!(states(&rooms, idle.id()).await, vec!["pending"]);
    });
}

#[test]
fn a_message_to_a_running_agent_queues_and_does_not_steer_it() {
    run_peer_test("queued-running", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let sent = send_to(&rooms, "peer", serde_json::json!({})).await;
        assert_eq!(sent["status"], "queued");
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, rooms.peer.id())
                .await
        );
        assert_eq!(states(&rooms, rooms.peer.id()).await, vec!["pending"]);
        assert_eq!(stripe(&rooms).await.len(), 1);
    });
}

#[test]
fn an_urgent_message_to_an_idle_agent_starts_its_turn() {
    run_peer_test("queued-urgent-idle", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({"urgent": true})).await;
        assert_eq!(sent["status"], "durable");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty());
    });
}

#[test]
fn the_person_cancels_a_queued_message_from_the_queue() {
    run_peer_test("queued-person-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        let cancelled = rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await
            .expect("the person cancels it");
        assert_eq!(cancelled.prompt.id(), id);
        assert!(stripe(&rooms).await.is_empty());
        assert_eq!(states(&rooms, idle.id()).await, vec!["expired"]);
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        let again = rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await;
        assert!(again.is_err(), "it is gone: a second cancel says so");
    });
}

#[test]
fn the_person_sends_a_queued_message_now_and_the_turn_starts() {
    run_peer_test("queued-person-send", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        rooms
            .router
            .runtime_state
            .steer_queued_prompt_with_external_authority(
                &rooms.peer_room,
                idle.id(),
                "client",
                &id,
                None,
            )
            .await
            .expect("the person sends it now");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty());
    });
}

#[test]
fn the_sender_promotes_its_queued_message_in_place_and_it_is_delivered_once() {
    run_peer_test("queued-promote", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let promoted = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(promoted["status"], "promoted");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty(), "the stripe is gone");
        assert_eq!(
            rooms.inbox(&rooms.peer_room, idle.id()).await.len(),
            1,
            "the same message, not a second one"
        );
        let again = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(again["status"], "already_delivered");
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 1);
    });
}

#[test]
fn sending_the_same_key_again_as_urgent_promotes_the_queued_message() {
    run_peer_test("queued-resend-urgent", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let key = serde_json::json!({"idempotency_key": "notes-1"});
        let queued = send_to(&rooms, "idle", key.clone()).await;
        let plain = send_to(&rooms, "idle", key).await;
        assert_eq!(plain, queued, "a repeat is answered as the first send was");
        assert_eq!(plain["sequence"], queued["sequence"]);
        let promoted = send_to(
            &rooms,
            "idle",
            serde_json::json!({"idempotency_key": "notes-1", "urgent": true}),
        )
        .await;
        assert_eq!(promoted["status"], "promoted");
        assert_eq!(promoted["sequence"], queued["sequence"]);
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 1);
    });
}

#[test]
fn only_the_sender_promotes_a_queued_message() {
    run_peer_test("queued-promote-sender-only", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let (stranger_token, _) = {
            let mut app = rooms.app.lock().await;
            let stranger = spawn_with_level(
                &mut app,
                &rooms.home,
                "stranger",
                AgentPermissionLevel::Yolo,
            );
            running_turn(&mut app, &rooms.home, stranger.id())
        };
        let refused = rooms
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &stranger_token,
                crate::transport::runtime_tools::PROMOTE_AGENT_MESSAGE_TOOL,
                serde_json::json!({"message_id": sent["message_id"]}),
            )
            .await;
        assert!(
            refused.is_err() || !refused.unwrap().ok,
            "another agent cannot promote it"
        );
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        assert_eq!(stripe(&rooms).await.len(), 1);
    });
}

#[test]
fn a_promotion_after_the_person_cancelled_says_so() {
    run_peer_test("queued-promote-after-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await
            .expect("the person cancels it");
        let promoted = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(promoted["status"], "withdrawn");
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
    });
}
