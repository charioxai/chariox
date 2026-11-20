// L13-P2 / RC1-RC6: an owner's room agent creates Rooms of its own and deletes only those.
// Rooms from `world`: `room` (a -> {b -> {d}, s}, peer) and `other_room` (q), one owner.
use super::subtree::{error_text, launch, room_command, world, World};
use super::*;

fn created_room(result: &crate::transport::runtime_tools::RuntimeToolResult) -> String {
    result.payload["response"]["room"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("created room in {result:?}"))
        .to_owned()
}

async fn room_of(world: &World, id: &str) -> crate::session::RuntimeSession {
    world.app.lock().await.sessions().get_session(id).unwrap()
}

#[test]
fn rc1_an_owner_agent_creates_a_room_that_lists_like_a_web_one() {
    run_large_stack_async_test("room-create-rc1", rc1);
}
async fn rc1() {
    let world = world("room-create-rc1").await;
    let created = room_command(
        &world,
        "a",
        "room create --name Bench --provider dev-stub --model room-model --account-profile drill",
    )
    .await;
    assert!(created.ok, "{created:?}");
    let id = created_room(&created);
    let room = room_of(&world, &id).await;
    let home = room_of(&world, &world.room).await;
    assert_eq!(
        room.alias(),
        Some("bench"),
        "the kernel lower-cases Room names like every other Room"
    );
    assert_eq!(room.owner_user_id(), home.owner_user_id());
    // The same Project shape as a Room made from the web: the kernel's default Project.
    assert_eq!(room.project_id(), home.project_id());
    let defaults = room.agent_defaults();
    assert_eq!(defaults.provider, "dev-stub");
    assert_eq!(defaults.model.as_deref(), Some("room-model"));
    assert_eq!(defaults.account_profile.as_deref(), Some("drill"));
    assert_eq!(room.created_by_agent_id(), Some(world.ids["a"].as_str()));
    let listed = room_command(&world, "a", "room list").await;
    let ids = listed.payload["response"]["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|room| room["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(ids.contains(&id), "{ids:?}");
}

#[test]
fn rc1_session_new_and_create_are_room_create_and_name_a_project() {
    run_large_stack_async_test("room-create-alias", alias);
}
async fn alias() {
    let world = world("room-create-alias").await;
    let project = room_of(&world, &world.room).await.project_id().to_string();
    for command in [
        format!("session new --name ViaNew --project {project} --provider dev-stub"),
        format!("session create --name ViaCreate --project {project} --provider dev-stub"),
    ] {
        let created = room_command(&world, "a", &command).await;
        assert!(created.ok, "{command}: {created:?}");
        assert_eq!(
            room_of(&world, &created_room(&created))
                .await
                .project_id()
                .to_string(),
            project
        );
    }
    let unknown = room_command(&world, "a", "room create --project no-such-project").await;
    assert!(!unknown.ok, "{unknown:?}");
}

#[test]
fn rc2_the_creator_starts_an_agent_in_the_room_it_made() {
    run_large_stack_async_test("room-create-rc2", rc2);
}
async fn rc2() {
    let world = world("room-create-rc2").await;
    let created = room_command(
        &world,
        "a",
        "room create --name Spawned --provider dev-stub",
    )
    .await;
    assert!(created.ok, "{created:?}");
    let id = created_room(&created);
    let spawned = room_command(
        &world,
        "a",
        &format!("agent spawn worker --provider dev-stub --room {id}"),
    )
    .await;
    assert!(spawned.ok, "{spawned:?}");
    let agents = world.app.lock().await.agents().get_session_agents(&id);
    assert!(
        agents.iter().any(|agent| agent.alias() == Some("worker")),
        "the new Room holds the spawned agent: {agents:?}"
    );
}

#[test]
fn rc3_a_creator_and_the_agents_above_it_delete_only_its_rooms() {
    run_large_stack_async_test("room-create-rc3", rc3);
}
async fn rc3() {
    let mut world = world("room-create-rc3").await;
    let d = world.ids["d"].clone();
    let token = launch(&world.app, &world.room, &d).await;
    world.tokens.insert("d", token);
    // b (below a) creates; a, above b, may delete; d below b and the peer may not.
    let by_b = created_room(&room_command(&world, "b", "room create --name ByB").await);
    for who in ["d", "peer", "q"] {
        let refused = room_command(&world, who, &format!("room delete {by_b}")).await;
        assert!(!refused.ok, "{who}: {refused:?}");
        assert!(error_text(&refused).contains("owner"), "{who}: {refused:?}");
    }
    assert_eq!(room_of(&world, &by_b).await.id(), by_b);
    let deleted = room_command(&world, "a", &format!("room delete {by_b}")).await;
    assert!(deleted.ok, "{deleted:?}");
    assert!(world
        .app
        .lock()
        .await
        .sessions()
        .get_session(&by_b)
        .is_err());
    // The creator deletes its own.
    let own = created_room(&room_command(&world, "a", "room create --name Own").await);
    let deleted = room_command(&world, "a", &format!("room delete {own}")).await;
    assert!(deleted.ok, "{deleted:?}");
    // Rooms nobody created, and the room the caller runs in, stay the owner's.
    for target in [world.other_room.clone(), world.room.clone()] {
        let refused = room_command(&world, "a", &format!("room delete {target}")).await;
        assert!(!refused.ok, "{target}: {refused:?}");
        assert!(
            error_text(&refused).contains("owner"),
            "{target}: {refused:?}"
        );
        assert_eq!(room_of(&world, &target).await.id(), target);
    }
}

#[test]
fn rc4_a_guest_and_a_non_owner_agent_cannot_create_a_room() {
    run_large_stack_async_test("room-create-rc4", rc4);
}
async fn rc4() {
    let mut world = world("room-create-rc4").await;
    for (alias, owner) in [("guest", "guest"), ("stranger", "someone-else")] {
        let agent = {
            let mut app = world.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(&world.room, "dev-stub")
                        .with_alias(alias)
                        .with_owner_user_id(owner),
                )
                .unwrap()
        };
        let token = launch(&world.app, &world.room, agent.id()).await;
        world.tokens.insert(alias, token);
        let before = world.app.lock().await.sessions().list_sessions().len();
        let refused = room_command(&world, alias, "room create --name Nope").await;
        assert!(!refused.ok, "{alias}: {refused:?}");
        assert!(
            error_text(&refused).contains("owner"),
            "{alias}: the refusal names who can: {refused:?}"
        );
        assert_eq!(
            world.app.lock().await.sessions().list_sessions().len(),
            before,
            "{alias} created nothing"
        );
    }
}

#[test]
fn rc6_attach_use_and_delete_of_sessions_stay_denied() {
    run_large_stack_async_test("room-create-rc6", rc6);
}
async fn rc6() {
    let world = world("room-create-rc6").await;
    for command in [
        format!("session attach {}", world.other_room),
        format!("session use {}", world.other_room),
        format!("session delete {}", world.other_room),
    ] {
        let refused = room_command(&world, "a", &command).await;
        assert!(!refused.ok, "{command}: {refused:?}");
    }
    assert_eq!(
        room_of(&world, &world.other_room).await.id(),
        world.other_room
    );
}
