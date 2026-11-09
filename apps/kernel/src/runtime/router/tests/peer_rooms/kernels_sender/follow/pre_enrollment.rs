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

    running_turn(&mut *b.app.lock().await, &b.room, b.agent.id());
    assert!(b.turn_running(b.agent.id()).await);
    a.command(format!(
        "agent cancel worker --room {} --kernel {}",
        b.room, b.name
    ))
    .await
    .expect("the peer's turn is cancelled");
    assert!(!b.turn_running(b.agent.id()).await);

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
    });
}
