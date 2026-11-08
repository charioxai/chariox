// MP-08 / MP-09 / MP-10 / MP-11 A3: `agent answer` and `agent pass` through the room command
// entry point. Tree from `world`: a -> {b -> {d}, s}; d raises, b holds, a does not.
use super::subtree::{launch, room_command, world};
use super::*;
use crate::provider::AgentPermissionLevel;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

#[test]
fn the_holder_answers_through_the_room_command_and_nobody_else() {
    run_large_stack_async_test("decision-commands", commands);
}

async fn commands() {
    let world = world("decision-commands").await;
    for agent in ["a", "b"] {
        world
            .app
            .lock()
            .await
            .agents()
            .update_agent_config(
                &world.ids[agent],
                None,
                Some(Some(AgentPermissionLevel::Yolo)),
                None,
                None,
            )
            .unwrap();
    }
    launch(&world.app, &world.room, &world.ids["d"]).await;
    let asked = RuntimeInteraction::new(
        "perm-cmd",
        &world.ids["d"],
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".into()),
        "Run `ls`?",
        vec![
            RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        None,
        None,
    );
    let mut answer = world
        .router
        .runtime_state
        .create_runtime_interaction(&world.room, asked)
        .await
        .unwrap();
    let refused = room_command(&world, "a", "agent answer perm-cmd allow").await;
    assert!(!refused.ok);
    assert_eq!(
        refused.payload["response"]["status"], "not_holder",
        "{refused:?}"
    );
    assert!(refused.payload.get("error").is_none(), "typed, no text");
    assert!(answer.try_recv().is_err());
    let done = room_command(&world, "b", "agent answer perm-cmd allow").await;
    assert!(done.ok, "{done:?}");
    assert_eq!(done.payload["response"]["status"], "answered");
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
    let late = room_command(&world, "b", "agent answer perm-cmd deny").await;
    assert_eq!(late.payload["response"]["status"], "settled_by_user");
}

#[test]
fn a_decision_is_never_answered_or_passed_for_another_room_or_kernel() {
    run_large_stack_async_test("decision-commands-peer", peer_flags_and_unknown_ids);
}

async fn peer_flags_and_unknown_ids() {
    let world = world("decision-commands-peer").await;
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["b"],
            None,
            Some(Some(AgentPermissionLevel::Yolo)),
            None,
            None,
        )
        .unwrap();
    launch(&world.app, &world.room, &world.ids["d"]).await;
    let asked = RuntimeInteraction::new(
        "perm-peer",
        &world.ids["d"],
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".into()),
        "Run `ls`?",
        vec![RuntimeInteractionChoice::new("allow", "Allow", "allow", None)],
        None,
        None,
        None,
    );
    let mut answer = world
        .router
        .runtime_state
        .create_runtime_interaction(&world.room, asked)
        .await
        .unwrap();
    for command in [
        format!("agent answer perm-peer allow --room {}", world.room),
        "agent answer perm-peer allow --kernel elsewhere".to_string(),
        format!("agent pass perm-peer --room {} --kernel elsewhere", world.room),
        "agent pass perm-peer --machine elsewhere".to_string(),
    ] {
        let refused = room_command(&world, "b", &command).await;
        assert!(!refused.ok, "{command}: {refused:?}");
        assert!(
            refused.payload.to_string().contains("peer_decision_refused"),
            "{command}: {refused:?}"
        );
    }
    assert!(answer.try_recv().is_err(), "no flag settled the card");
    let unknown = room_command(&world, "b", "agent answer no-such-card allow").await;
    assert!(!unknown.ok);
    assert_eq!(
        unknown.payload["response"]["status"], "unknown_interaction",
        "{unknown:?}"
    );
    let unknown = room_command(&world, "b", "agent pass no-such-card").await;
    assert_eq!(unknown.payload["response"]["status"], "unknown_interaction");
    let done = room_command(&world, "b", "agent answer perm-peer allow").await;
    assert!(done.ok, "own-room answering is unchanged: {done:?}");
}
