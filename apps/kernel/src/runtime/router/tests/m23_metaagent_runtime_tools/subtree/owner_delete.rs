// Row 314: an owner's agent deletes agents of its Room that are not its own children, as the owner can:
// the agent and everything below it go, running or not, and the result lists them.

use super::*;
use crate::agent::AgentState;

fn names(list: &serde_json::Value) -> Vec<String> {
    let mut found = list
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            entry["alias"]
                .as_str()
                .or_else(|| entry["agent"]["alias"].as_str())
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

async fn exists(world: &World, who: &str) -> bool {
    world
        .app
        .lock()
        .await
        .agents()
        .get_agent(&world.ids[who])
        .is_ok()
}

async fn working(world: &World, who: &str) {
    world
        .app
        .lock()
        .await
        .agents()
        .set_agent_state(&world.ids[who], AgentState::Working)
        .unwrap();
}

/// The world launches a provider turn for a, b and peer, so they run. An ended subtree is made of
/// agents that never started one: top -> {f1, f2 -> {g}}.
async fn ended_subtree(world: &mut World) {
    let mut app = world.app.lock().await;
    let mut ids: Vec<(&'static str, String)> = Vec::new();
    for (alias, parent) in [
        ("top", None),
        ("f1", Some("top")),
        ("f2", Some("top")),
        ("g", Some("f2")),
    ] {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&world.room, "dev-stub").with_alias(alias),
            )
            .unwrap();
        if let Some(parent) = parent {
            let parent = ids
                .iter()
                .find(|(name, _)| *name == parent)
                .unwrap()
                .1
                .clone();
            app.agents()
                .set_parent_agent(agent.id(), Some(crate::agent::AgentRef::local(parent)))
                .unwrap();
        }
        ids.push((alias, agent.id().to_owned()));
    }
    drop(app);
    for (alias, id) in ids {
        world.ids.insert(alias, id);
    }
}

#[test]
fn a_peer_deletes_an_ended_agent_that_is_not_its_child_with_everything_below_it() {
    run_large_stack_async_test("owner-delete-peer", peer_deletes_ended_subtree);
}

async fn peer_deletes_ended_subtree() {
    let mut world = world("owner-delete-peer").await;
    ended_subtree(&mut world).await;
    // peer is not above top: before this, only a parent could delete an agent.
    let result = room_command(
        &world,
        "peer",
        &format!("agent delete {}", world.ids["top"]),
    )
    .await;
    assert!(result.ok, "{result:?}");
    let response = &result.payload["response"];
    assert_eq!(response["type"], "AgentDestroyed", "{result:?}");
    assert_eq!(
        names(&response["deleted"]),
        ["f1", "f2", "g", "top"],
        "{result:?}"
    );
    assert!(
        response["deleted"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["running"] == false),
        "none of them runs: {result:?}"
    );
    for gone in ["top", "f1", "f2", "g"] {
        assert!(!exists(&world, gone).await, "{gone} is deleted");
    }
    for stays in ["a", "b", "d", "s", "peer"] {
        assert!(exists(&world, stays).await, "{stays} stays");
    }
}

#[test]
fn an_agent_never_deletes_itself_or_an_agent_of_another_room() {
    run_large_stack_async_test("owner-delete-scope", scope_is_the_callers_room);
}

async fn scope_is_the_callers_room() {
    let world = world("owner-delete-scope").await;
    let own = room_command(
        &world,
        "peer",
        &format!("agent delete {}", world.ids["peer"]),
    )
    .await;
    assert!(!own.ok, "{own:?}");
    assert!(error_text(&own).contains("cannot delete itself"), "{own:?}");
    let other_room = room_command(&world, "q", &format!("agent delete {}", world.ids["b"])).await;
    assert!(!other_room.ok, "{other_room:?}");
    for stays in ["a", "b", "d", "s", "peer"] {
        assert!(exists(&world, stays).await, "{stays} stays");
    }
}

#[test]
fn an_agent_deletes_a_running_agent_it_did_not_create_with_its_subtree() {
    run_large_stack_async_test("owner-delete-parity", parity_delete_is_the_owners_delete);
}

async fn parity_delete_is_the_owners_delete() {
    let world = world("owner-delete-parity").await;
    working(&world, "b").await;
    working(&world, "d").await;
    // peer did not create b and b runs: the owner's own delete takes it with everything below it.
    let result = room_command(&world, "peer", &format!("agent delete {}", world.ids["b"])).await;
    assert!(result.ok, "{result:?}");
    let response = &result.payload["response"];
    assert_eq!(response["type"], "AgentDestroyed", "{result:?}");
    let listed = response["deleted"].as_array().unwrap();
    assert_eq!(names(&response["deleted"]), ["b", "d"], "{result:?}");
    assert!(
        listed.iter().all(|entry| entry["running"] == true),
        "the result names the running agents it stops: {result:?}"
    );
    for gone in ["b", "d"] {
        assert!(!exists(&world, gone).await, "{gone} is deleted");
    }
    for stays in ["a", "s", "peer"] {
        assert!(exists(&world, stays).await, "{stays} stays");
    }
}

#[test]
fn an_agent_never_deletes_an_agent_above_it() {
    run_large_stack_async_test("owner-delete-above", never_deletes_above);
}

async fn never_deletes_above() {
    let world = world("owner-delete-above").await;
    let above = room_command(&world, "b", &format!("agent delete {}", world.ids["a"])).await;
    assert!(!above.ok, "{above:?}");
    assert!(error_text(&above).contains("above you"), "{above:?}");
    for stays in ["a", "b", "d", "s", "peer"] {
        assert!(exists(&world, stays).await, "{stays} stays");
    }
}

#[test]
fn an_agent_of_another_owner_is_never_deleted() {
    run_large_stack_async_test("owner-delete-foreign-owner", foreign_owner_refused);
}

async fn foreign_owner_refused() {
    let world = world("owner-delete-foreign-owner").await;
    let foreign = {
        let mut app = world.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&world.room, "dev-stub")
                    .with_alias("guest-agent")
                    .with_owner_user_id("another-owner"),
            )
            .unwrap()
            .id()
            .to_owned()
    };
    let refused = room_command(&world, "peer", &format!("agent delete {foreign}")).await;
    assert!(!refused.ok, "{refused:?}");
    assert!(error_text(&refused).contains("own owner"), "{refused:?}");
    assert!(world.app.lock().await.agents().get_agent(&foreign).is_ok());
}
