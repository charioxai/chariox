//! A room made before its kernel was enrolled keeps the owner `local`; once both kernels are
//! enrolled to the same Cloud owner, its agents reach peers on the other kernel, and are reached
//! there, as the owner's other rooms are.
use super::*;
use crate::session::DEFAULT_LOCAL_USER_ID;

/// A's worker messages, follows, cancels and spawns B's worker, with `local` owning the room on
/// the calling end, the target end, or neither.
async fn reaches_a_peer_across_kernels(
    test: &str,
    caller_room_owner: &str,
    target_room_owner: &str,
) {
    let a = kernel_with_room_of(
        &format!("{test}-a"),
        AgentPermissionLevel::Yolo,
        true,
        caller_room_owner,
    );
    let b = kernel_with_room_of(
        &format!("{test}-b"),
        AgentPermissionLevel::Yolo,
        true,
        target_room_owner,
    );
    link(&a);
    link(&b);

    a.send(serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "hello"}))
        .await
        .expect("a message reaches the peer");
    assert_eq!(b.messages(b.agent.id()).await.len(), 1);

    let task = with_task(&a).await;
    a.subscribe(
        &task,
        serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
    )
    .await
    .expect("A follows the peer");
    a.router
        .runtime_state
        .sweep_agent_lifecycle_for_test()
        .await
        .unwrap();
    assert!(
        a.peer_events().await.is_empty(),
        "a follow of the same owner's kernel is kept"
    );

    b.end_turn(false).await;
    let events = a.peer_events_after_delivery(1).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].source_id, source_of(&b));
    assert_eq!(events[0].payload["event"], "turn_ended", "{:?}", events[0]);

    let other = {
        let mut app = b.app.lock().await;
        let other = owned_agent(
            &mut app,
            &b.room,
            "other",
            target_room_owner,
            AgentPermissionLevel::Yolo,
        );
        running_turn(&mut app, &b.room, other.id());
        other
    };
    a.command(format!(
        "agent cancel other --room {} --kernel {}",
        b.room, b.name
    ))
    .await
    .expect("the peer's turn is cancelled");
    assert!(!b.turn_running(other.id()).await);

    let spawned = a
        .command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .expect("a peer spawns in that room");
    assert_eq!(spawned["response"]["spawned"]["room"], b.room.as_str());

    let foreign = create_room(&mut *b.app.lock().await, "foreign", Some("another-owner"));
    let refused = a
        .send(serde_json::json!({"agent": "worker", "room": foreign, "kernel": b.name, "message": "hello"}))
        .await
        .unwrap_err();
    assert!(refused.contains("room_unavailable"), "{refused}");
    unlink(&[&a, &b]);
}

#[test]
fn an_agent_of_a_room_made_before_enrollment_reaches_peers_on_another_kernel() {
    run_peer_test("peer-pre-enrolled-caller", || async {
        reaches_a_peer_across_kernels("peer-pre-enrolled-caller", DEFAULT_LOCAL_USER_ID, OWNER)
            .await;
    });
}

#[test]
fn a_room_made_before_enrollment_is_reached_from_another_kernel() {
    run_peer_test("peer-pre-enrolled-target", || async {
        reaches_a_peer_across_kernels("peer-pre-enrolled-target", OWNER, DEFAULT_LOCAL_USER_ID)
            .await;
    });
}

#[test]
fn an_agent_of_a_room_made_before_enrollment_reaches_the_owners_other_rooms_here() {
    run_peer_test("peer-pre-enrolled-local", || async {
        let a = kernel_with_room_of(
            "peer-pre-enrolled-local",
            AgentPermissionLevel::Yolo,
            true,
            DEFAULT_LOCAL_USER_ID,
        );
        let other = {
            let mut app = a.app.lock().await;
            let room = create_room(&mut app, "enrolled-room", Some(OWNER));
            let helper = owned_agent(&mut app, &room, "helper", OWNER, AgentPermissionLevel::Yolo);
            running_turn(&mut app, &room, helper.id());
            room
        };
        for urgent in [false, true] {
            let sent = a
                .send(serde_json::json!({"agent": "helper", "room": other, "message": "hello", "urgent": urgent}))
                .await;
            assert!(sent.is_ok(), "urgent {urgent}: {sent:?}");
        }
        let spawned = a
            .command(format!("agent spawn peer --room {other}"))
            .await
            .expect("a `local` agent creates a peer in the owner's enrolled room");
        let peer = a
            .app
            .lock()
            .await
            .agents()
            .get_session_agents(&other)
            .into_iter()
            .find(|agent| agent.alias() == Some("peer"))
            .unwrap_or_else(|| panic!("the peer lives in the enrolled room: {spawned}"));
        assert_eq!(
            peer.owner_user_id(),
            OWNER,
            "it belongs to the room's owner"
        );
    });
}

/// In a room made before enrollment the enrolled owner is the room's owner: an agent it owns and
/// a turn it started reach the owner's other rooms here and on another kernel. A member who is
/// not the owner under any identity stays a guest of that room.
#[test]
fn the_enrolled_owner_is_no_guest_of_a_room_made_before_enrollment() {
    run_peer_test("peer-pre-enrolled-guest", || async {
        let a = kernel_with_room_of(
            "peer-pre-enrolled-guest-a",
            AgentPermissionLevel::Yolo,
            true,
            DEFAULT_LOCAL_USER_ID,
        );
        let b = kernel(
            "peer-pre-enrolled-guest-b",
            AgentPermissionLevel::Yolo,
            true,
        );
        link(&a);
        link(&b);
        let (other_room, older_room, drivers) = {
            let mut app = a.app.lock().await;
            let older_room = create_room(&mut app, "older-room", Some(DEFAULT_LOCAL_USER_ID));
            let other_room = create_room(&mut app, "enrolled-room", Some(OWNER));
            let neighbour = owned_agent(
                &mut app,
                &other_room,
                "neighbour",
                OWNER,
                AgentPermissionLevel::Yolo,
            );
            running_turn(&mut app, &other_room, neighbour.id());
            let mut room = app.sessions().get_session(&a.room).unwrap();
            room.add_member(
                "guest-user",
                Some(DEFAULT_LOCAL_USER_ID.into()),
                crate::session::CollaborationLevel::Full,
            );
            app.sessions_mut().restore_session(room);
            let mut drivers = Vec::new();
            // (alias, agent owner, who starts its turn, whether it acts as the room's owner)
            for (alias, owner, starter, owner_side) in [
                ("enrolled-own", OWNER, None, true),
                ("enrolled-driven", DEFAULT_LOCAL_USER_ID, Some(OWNER), true),
                ("guest-own", "guest-user", None, false),
                (
                    "guest-driven",
                    DEFAULT_LOCAL_USER_ID,
                    Some("guest-user"),
                    false,
                ),
            ] {
                let agent =
                    owned_agent(&mut app, &a.room, alias, owner, AgentPermissionLevel::Yolo);
                let turn = match starter {
                    Some(user) => guest_turn(&mut app, &a.room, agent.id(), user),
                    None => running_turn(&mut app, &a.room, agent.id()),
                };
                drivers.push((alias, turn, owner_side));
            }
            (other_room, older_room, drivers)
        };
        for (alias, (token, prompt), owner_side) in &drivers {
            let here = a
                .send_as(
                    token,
                    prompt,
                    serde_json::json!({"agent": "neighbour", "room": other_room, "message": "hello"}),
                )
                .await;
            let across = a
                .send_as(
                    token,
                    prompt,
                    serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "hello"}),
                )
                .await;
            let spawn = a
                .command_as(
                    token,
                    format!("agent spawn helper-{alias} --room {older_room}"),
                )
                .await;
            let spawn_on_b = a
                .command_as(
                    token,
                    format!(
                        "agent spawn helper-{alias} --room {} --kernel {} --no-follow",
                        b.room, b.name
                    ),
                )
                .await;
            if *owner_side {
                assert!(here.is_ok(), "{alias}: {here:?}");
                assert!(across.is_ok(), "{alias}: {across:?}");
                assert!(spawn.is_ok(), "{alias}: {spawn:?}");
                assert!(spawn_on_b.is_ok(), "{alias}: {spawn_on_b:?}");
            } else {
                let here = here.unwrap_err();
                assert!(here.contains("unavailable"), "{alias}: {here}");
                let across = across.unwrap_err();
                assert!(across.contains("room_unavailable"), "{alias}: {across}");
                let spawn = spawn.unwrap_err();
                assert!(spawn.contains("does not exist"), "{alias}: {spawn}");
                let spawn_on_b = spawn_on_b.unwrap_err();
                assert!(
                    spawn_on_b.contains("room_unavailable"),
                    "{alias}: {spawn_on_b}"
                );
            }
        }
        let (token, _) = &drivers[0].1;
        a.command_as(token, format!("agent spawn helper --room {other_room}"))
            .await
            .expect("the enrolled owner's agent creates a peer in the owner's enrolled room");
        assert_eq!(
            b.messages(b.agent.id()).await.len(),
            2,
            "only the owner's turns reached B"
        );
        unlink(&[&a, &b]);
    });
}
