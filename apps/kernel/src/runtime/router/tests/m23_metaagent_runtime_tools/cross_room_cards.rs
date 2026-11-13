//! Owner reach goes through the authenticated room command, including refusals.
use super::subtree::{launch, room_command, world, World};
use super::*;
use crate::provider::AgentPermissionLevel;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionCustomChoice,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

async fn coordinator(label: &str) -> World {
    let mut world = world(label).await;
    let q = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.other_room)[0]
        .id()
        .to_owned();
    world.ids.insert("q", q);
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["a"],
            None,
            Some(Some(AgentPermissionLevel::Yolo)),
            None,
            None,
        )
        .unwrap();
    world
}

fn card(id: &str, asker: &str, secret: bool) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        asker,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".into()),
        "Run `ls`?",
        vec![RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        secret.then(|| {
            RuntimeInteractionCustomChoice::secret("secret", "Passphrase", None, Some(1), None)
        }),
        None,
        None,
    )
}

#[test]
fn xroom_01_owner_lists_other_rooms_agents_and_turn_state() {
    run_large_stack_async_test("xroom-01", agents);
}
async fn agents() {
    let world = coordinator("xroom-01").await;
    let result = room_command(&world, "a", &format!("room agents {}", world.other_room)).await;
    assert!(result.ok, "{result:?}");
    let agents = result.payload["response"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["id"], world.ids["q"]);
    assert!(agents[0].get("state").is_some());
    assert!(agents[0].get("is_processing").is_some());
    assert!(agents[0].get("active_prompt_id").is_some());
    assert!(agents[0].get("task").is_some());
}

#[test]
fn xroom_02_owner_lists_pending_cards_with_their_choices() {
    run_large_stack_async_test("xroom-02", cards);
}
async fn cards() {
    let world = coordinator("xroom-02").await;
    let _answer = world
        .router
        .runtime_state
        .create_runtime_interaction(
            &world.other_room,
            card("xroom-perm", &world.ids["q"], false),
        )
        .await
        .unwrap();
    let result = room_command(&world, "a", &format!("room cards {}", world.other_room)).await;
    assert!(result.ok, "{result:?}");
    let cards = result.payload["response"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["id"], "xroom-perm");
    assert_eq!(cards[0]["message"], "Run `ls`?");
    assert_eq!(cards[0]["choices"][0]["id"], "allow");
    assert_eq!(cards[0]["answerable"], true);
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/cross-room-cards-protocol.json"
    ))
    .unwrap();
    let mut actual = result.payload["response"].clone();
    actual["room_id"] = "room-other".into();
    actual["cards"][0]["agent_id"] = "asker".into();
    assert_eq!(actual, snapshot["response"]);
    assert_eq!(
        snapshot["local"],
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
    );
    assert_eq!(
        snapshot["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "c2166d28b6d6f5292a7b2084df4c233f11976b9fb6e3919a9385f1548336d4b1"
    );
}

#[test]
fn xroom_03_owner_answers_once_and_the_asking_agent_receives_it() {
    run_large_stack_async_test("xroom-03", answer);
}
async fn answer() {
    let world = coordinator("xroom-03").await;
    let mut answer = world
        .router
        .runtime_state
        .create_runtime_interaction(
            &world.other_room,
            card("xroom-answer", &world.ids["q"], false),
        )
        .await
        .unwrap();
    let command = format!(
        "agent answer xroom-answer allow --room {}",
        world.other_room
    );
    let result = room_command(&world, "a", &command).await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.payload["response"]["status"], "answered");
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
    let session = world
        .router
        .runtime_state
        .session_snapshot(&world.other_room)
        .await
        .unwrap();
    assert!(session.active_interactions().is_empty());
    let late = room_command(&world, "a", &command).await;
    assert!(!late.ok, "a second answer must lose: {late:?}");
}

#[test]
fn xroom_04_secret_cards_stay_owner_only() {
    run_large_stack_async_test("xroom-04", secrets);
}
async fn secrets() {
    let world = coordinator("xroom-04").await;
    let mut answer = world
        .router
        .runtime_state
        .create_runtime_interaction(
            &world.other_room,
            card("xroom-secret", &world.ids["q"], true),
        )
        .await
        .unwrap();
    let listed = room_command(&world, "a", &format!("room cards {}", world.other_room)).await;
    assert!(listed.ok, "{listed:?}");
    assert_eq!(listed.payload["response"]["cards"][0]["answerable"], false);
    let result = room_command(
        &world,
        "a",
        &format!(
            "agent answer xroom-secret allow --room {}",
            world.other_room
        ),
    )
    .await;
    assert!(!result.ok, "{result:?}");
    assert!(
        result.payload.to_string().contains("owner-only"),
        "{result:?}"
    );
    assert!(answer.try_recv().is_err());
}

#[test]
fn xroom_05_a_guests_agent_cannot_inspect_or_answer_another_room() {
    run_large_stack_async_test("xroom-05", guest);
}
async fn guest() {
    let mut world = coordinator("xroom-05").await;
    let guest = {
        let mut app = world.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&world.room, "dev-stub")
                    .with_alias("guest")
                    .with_owner_user_id("guest"),
            )
            .unwrap()
    };
    let token = launch(&world.app, &world.room, guest.id()).await;
    world.tokens.insert("guest", token);
    for command in [
        format!("room agents {}", world.other_room),
        format!("room cards {}", world.other_room),
        format!("agent answer xroom-perm allow --room {}", world.other_room),
    ] {
        let result = room_command(&world, "guest", &command).await;
        assert!(!result.ok, "{command}: {result:?}");
        assert!(
            result.payload.to_string().contains("guest"),
            "{command}: {result:?}"
        );
    }
}

#[test]
fn owner_card_authority_is_the_same_at_home_and_in_another_room() {
    run_large_stack_async_test("xroom-authority", authority);
}
async fn authority() {
    let world = coordinator("xroom-authority").await;
    for room in [&world.room, &world.other_room] {
        let id = format!("grant-{room}");
        let owner = world
            .app
            .lock()
            .await
            .sessions()
            .get_session(room)
            .unwrap()
            .owner_user_id()
            .to_owned();
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            format!("access-grant:{id}"),
            "Grant",
            "Allow this non-secret action?",
            vec![RuntimeInteractionChoice::new(
                "allow", "Allow", "allow", None,
            )],
        );
        let mut answer = world
            .router
            .runtime_state
            .create_kernel_operation_interaction(room, &owner, interaction)
            .await
            .unwrap();
        let result = room_command(
            &world,
            "a",
            &format!("agent answer {id} allow --room {room}"),
        )
        .await;
        assert!(result.ok, "{result:?}");
        assert_eq!(
            answer.try_recv().unwrap().choice_id.as_deref(),
            Some("allow")
        );
    }
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["a"],
            None,
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    let mut pending = world
        .router
        .runtime_state
        .create_runtime_interaction(&world.other_room, card("limited", &world.ids["q"], false))
        .await
        .unwrap();
    let result = room_command(
        &world,
        "a",
        &format!("agent answer limited allow --room {}", world.other_room),
    )
    .await;
    assert!(
        !result.ok,
        "current permission downgrade was ignored: {result:?}"
    );
    assert!(pending.try_recv().is_err());
}

#[test]
fn passkey_cards_are_owner_only_even_when_the_chosen_button_is_not_secret() {
    run_large_stack_async_test("xroom-passkey", passkey);
}
async fn passkey() {
    let world = coordinator("xroom-passkey").await;
    let owner = world
        .app
        .lock()
        .await
        .sessions()
        .get_session(&world.other_room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    let interaction = RuntimeInteraction::for_kernel_operation(
        "passkey",
        "access-grant:test",
        "Grant",
        "Requires owner verification",
        vec![
            RuntimeInteractionChoice::new("allow", "Allow", "allow", None).requiring_passkey(),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
    );
    let mut pending = world
        .router
        .runtime_state
        .create_kernel_operation_interaction(&world.other_room, &owner, interaction)
        .await
        .unwrap();
    let result = room_command(
        &world,
        "a",
        &format!("agent answer passkey deny --room {}", world.other_room),
    )
    .await;
    assert!(!result.ok, "{result:?}");
    assert!(
        result.payload.to_string().contains("owner-only"),
        "{result:?}"
    );
    assert!(pending.try_recv().is_err());
}

#[test]
fn another_owners_room_and_hidden_subagents_are_not_exposed() {
    run_large_stack_async_test("xroom-visibility", visibility);
}
async fn visibility() {
    let world = coordinator("xroom-visibility").await;
    let foreign = {
        let mut app = world.app.lock().await;
        let workspace = world._env.root.join("workspace");
        let (room, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy())
                    .with_owner_user_id("another-owner"),
            )
            .unwrap();
        room.id().to_owned()
    };
    for verb in ["agents", "cards"] {
        let result = room_command(&world, "a", &format!("room {verb} {foreign}")).await;
        assert!(!result.ok, "{result:?}");
    }
    let result = room_command(&world, "q", &format!("room agents {}", world.room)).await;
    assert!(result.ok, "{result:?}");
    let agents = result.payload["response"]["agents"].as_array().unwrap();
    assert!(agents.iter().all(|agent| ["b", "s", "d"]
        .iter()
        .all(|hidden| agent["id"] != world.ids[*hidden])));
}

#[test]
fn competing_owner_answers_settle_one_shared_card_once() {
    run_large_stack_async_test("xroom-race", competing_answers);
}
async fn competing_answers() {
    let world = coordinator("xroom-race").await;
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["peer"],
            None,
            Some(Some(AgentPermissionLevel::Yolo)),
            None,
            None,
        )
        .unwrap();
    let mut answer = world
        .router
        .runtime_state
        .create_runtime_interaction(&world.other_room, card("race", &world.ids["q"], false))
        .await
        .unwrap();
    let command = format!("agent answer race allow --room {}", world.other_room);
    let (a, b) = tokio::join!(
        room_command(&world, "a", &command),
        room_command(&world, "peer", &command)
    );
    assert_ne!(a.ok, b.ok, "exactly one answer must win: {a:?}, {b:?}");
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}

#[test]
fn owner_room_answers_follow_the_kernel_owner_identity_rule() {
    run_large_stack_async_test("xroom-owner-identity", owner_identity);
}

async fn owner_identity() {
    let world = super::subtree::world_with_config("xroom-owner-identity", |config| {
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "xroom-owner".into(),
            ..Default::default()
        });
    })
    .await;
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["a"],
            None,
            Some(Some(AgentPermissionLevel::Yolo)),
            None,
            None,
        )
        .unwrap();
    let target = {
        let mut app = world.app.lock().await;
        let workspace = world._env.root.join("workspace");
        let (room, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy())
                    .with_owner_user_id("xroom-owner"),
            )
            .unwrap();
        room.id().to_owned()
    };
    let interaction = RuntimeInteraction::for_kernel_operation(
        "owner-identity-grant",
        "access-grant:owner-identity",
        "Grant",
        "Allow this action?",
        vec![RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
    );
    let mut answer = world
        .router
        .runtime_state
        .create_kernel_operation_interaction(&target, "xroom-owner", interaction)
        .await
        .unwrap();
    let result = room_command(
        &world,
        "a",
        &format!("agent answer owner-identity-grant allow --room {target}"),
    )
    .await;
    assert!(
        result.ok,
        "one owner keeps its authority across both identities: {result:?}"
    );
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}
