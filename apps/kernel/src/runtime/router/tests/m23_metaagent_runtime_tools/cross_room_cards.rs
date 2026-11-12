//! Owner reach goes through the authenticated room command, including refusals.
use super::subtree::{launch, room_command, world, World};
use super::*;
use crate::provider::AgentPermissionLevel;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel};

async fn coordinator(label: &str) -> World {
    let mut world = world(label).await;
    let q = world.app.lock().await.agents().get_session_agents(&world.other_room)[0].id().to_owned();
    world.ids.insert("q", q);
    world.app.lock().await.agents().update_agent_config(&world.ids["a"], None, Some(Some(AgentPermissionLevel::Yolo)), None, None).unwrap();
    world
}

fn card(id: &str, asker: &str, secret: bool) -> RuntimeInteraction {
    RuntimeInteraction::new(id, asker, RuntimeInteractionKind::Choice, RuntimeInteractionLevel::Warning,
        Some("Allow command?".into()), "Run `ls`?", vec![RuntimeInteractionChoice::new("allow", "Allow", "allow", None)],
        secret.then(|| RuntimeInteractionCustomChoice::secret("secret", "Passphrase", None, Some(1), None)), None, None)
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
    let _answer = world.router.runtime_state.create_runtime_interaction(&world.other_room, card("xroom-perm", &world.ids["q"], false)).await.unwrap();
    let result = room_command(&world, "a", &format!("room cards {}", world.other_room)).await;
    assert!(result.ok, "{result:?}");
    let cards = result.payload["response"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["id"], "xroom-perm");
    assert_eq!(cards[0]["message"], "Run `ls`?");
    assert_eq!(cards[0]["choices"][0]["id"], "allow");
    assert_eq!(cards[0]["answerable"], true);
}

#[test]
fn xroom_03_owner_answers_once_and_the_asking_agent_receives_it() {
    run_large_stack_async_test("xroom-03", answer);
}
async fn answer() {
    let world = coordinator("xroom-03").await;
    let mut answer = world.router.runtime_state.create_runtime_interaction(&world.other_room, card("xroom-answer", &world.ids["q"], false)).await.unwrap();
    let command = format!("agent answer xroom-answer allow --room {}", world.other_room);
    let result = room_command(&world, "a", &command).await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.payload["response"]["status"], "answered");
    assert_eq!(answer.try_recv().unwrap().choice_id.as_deref(), Some("allow"));
    let session = world.router.runtime_state.session_snapshot(&world.other_room).await.unwrap();
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
    let mut answer = world.router.runtime_state.create_runtime_interaction(&world.other_room, card("xroom-secret", &world.ids["q"], true)).await.unwrap();
    let listed = room_command(&world, "a", &format!("room cards {}", world.other_room)).await;
    assert!(listed.ok, "{listed:?}");
    assert_eq!(listed.payload["response"]["cards"][0]["answerable"], false);
    let result = room_command(&world, "a", &format!("agent answer xroom-secret allow --room {}", world.other_room)).await;
    assert!(!result.ok, "{result:?}");
    assert!(result.payload.to_string().contains("owner-only"), "{result:?}");
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
        crate::app::KernelSessionService::new(&mut app).spawn_agent(
            CreateAgentRequest::new(&world.room, "dev-stub").with_alias("guest").with_owner_user_id("guest")
        ).unwrap()
    };
    let token = launch(&world.app, &world.room, guest.id()).await;
    world.tokens.insert("guest", token);
    for command in [format!("room agents {}", world.other_room), format!("room cards {}", world.other_room), format!("agent answer xroom-perm allow --room {}", world.other_room)] {
        let result = room_command(&world, "guest", &command).await;
        assert!(!result.ok, "{command}: {result:?}");
        assert!(result.payload.to_string().contains("guest"), "{command}: {result:?}");
    }
}
