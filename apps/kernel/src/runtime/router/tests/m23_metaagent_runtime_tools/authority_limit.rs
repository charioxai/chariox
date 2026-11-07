// MP-08 / MP-09 / MP-10 / MP-11 A5: the authority limit is checked where the child is created,
// against the parent's current settings. Tree from `world`: a -> {b -> {d}, s}.
use super::subtree::{error_text, launch, room_command, world, World};
use super::*;
use crate::provider::AgentPermissionLevel;

async fn set_level(world: &World, agent: &str, level: AgentPermissionLevel) {
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(&world.ids[agent], None, Some(Some(level)), None, None)
        .unwrap();
}

async fn alias_exists(world: &World, alias: &str) -> bool {
    world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room)
        .iter()
        .any(|agent| agent.alias() == Some(alias))
}

#[test]
fn lowering_the_parent_between_the_check_and_the_effect_refuses_the_spawn() {
    run_large_stack_async_test("authority-race", race);
}

async fn race() {
    let world = world("authority-race").await;
    let store = world.app.lock().await.agents().clone();
    let parent = world.ids["a"].clone();
    crate::agent::authority_limit::set_hook_before_spawn_commit(&world.room, move || {
        store
            .update_agent_config(
                &parent,
                None,
                Some(Some(AgentPermissionLevel::Required)),
                None,
                None,
            )
            .unwrap();
    });
    let raced = room_command(
        &world,
        "a",
        "agent spawn racer --provider dev-stub --permission yolo",
    )
    .await;
    assert!(
        !raced.ok && error_text(&raced).contains("more authority than its parent"),
        "{raced:?}"
    );
    assert!(!alias_exists(&world, "racer").await, "no child was created");
}

#[test]
fn a_child_runs_at_most_where_its_parent_does_for_every_level_and_mode() {
    run_large_stack_async_test("authority-matrix", matrix);
}

async fn matrix() {
    use crate::provider::AgentExecutionMode::{Build, Plan};
    use AgentPermissionLevel::{Required, Yolo};
    let mut world = world("authority-matrix").await;
    for (parent_level, parent_mode) in [
        (Required, Plan),
        (Required, Build),
        (Yolo, Plan),
        (Yolo, Build),
    ] {
        let alias = format!("p-{parent_level}-{parent_mode}");
        let parent = {
            let mut app = world.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(&world.room, "dev-stub")
                        .with_alias(&alias)
                        .with_permission_level_override(parent_level)
                        .with_execution_mode_override(parent_mode),
                )
                .unwrap()
        };
        let token = launch(&world.app, &world.room, parent.id()).await;
        world
            .tokens
            .insert(Box::leak(alias.clone().into_boxed_str()), token);
        for child_level in [Required, Yolo] {
            let result = room_command(
                &world,
                &alias,
                &format!("agent spawn c-{child_level}-{alias} --provider dev-stub --permission {child_level}"),
            )
            .await;
            assert_eq!(
                result.ok,
                parent_level.permits(child_level),
                "{parent_level}/{parent_mode} -> {child_level}: {result:?}"
            );
            if result.ok {
                let child_alias = format!("c-{child_level}-{alias}");
                let agents = world
                    .app
                    .lock()
                    .await
                    .agents()
                    .get_session_agents(&world.room);
                let child = agents
                    .iter()
                    .find(|agent| agent.alias() == Some(child_alias.as_str()))
                    .unwrap();
                assert_eq!(
                    child.execution_mode_override(),
                    Some(parent_mode),
                    "the child starts in its parent's mode"
                );
            }
        }
        // The mode is checked where the child is created, whoever asks.
        for child_mode in [Plan, Build] {
            let created = {
                let mut app = world.app.lock().await;
                crate::app::KernelSessionService::new(&mut app).spawn_agent(
                    CreateAgentRequest::new(&world.room, "dev-stub")
                        .with_parent_agent_id(parent.id())
                        .with_execution_mode_override(child_mode),
                )
            };
            assert_eq!(
                created.is_ok(),
                parent_mode.permits(child_mode),
                "{parent_mode} -> {child_mode}"
            );
        }
    }
}

#[test]
fn a_child_the_user_raised_above_its_parent_is_still_controlled_by_it() {
    run_large_stack_async_test("authority-raised", raised);
}

async fn raised() {
    let world = world("authority-raised").await;
    set_level(&world, "a", AgentPermissionLevel::Required).await;
    set_level(&world, "b", AgentPermissionLevel::Yolo).await;
    let controlled = room_command(&world, "a", "agent deadline get b").await;
    assert!(controlled.ok, "membership, not comparison: {controlled:?}");
    let cancelled = room_command(&world, "a", "agent cancel b").await;
    assert!(cancelled.ok, "{cancelled:?}");
    let agents = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room);
    let session = world
        .app
        .lock()
        .await
        .sessions()
        .get_session(&world.room)
        .unwrap();
    let b = agents.iter().find(|a| a.id() == world.ids["b"]).unwrap();
    assert_eq!(
        crate::session::effective_agent_execution_config_in_tree(&session, b, &agents)
            .permission_level,
        AgentPermissionLevel::Required,
        "raised above its parent, it still runs at the parent's limit"
    );
}

#[test]
fn the_caps_are_depth_four_sixteen_per_parent_and_sixty_four_per_team() {
    run_large_stack_async_test("authority-caps", caps);
}

async fn spawn_under(world: &World, parent: &str, alias: &str) -> String {
    let mut app = world.app.lock().await;
    crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&world.room, "dev-stub")
                .with_alias(alias)
                .with_parent_agent_id(parent),
        )
        .unwrap()
        .id()
        .to_owned()
}

async fn caps() {
    let mut world = world("authority-caps").await;
    // Depth: a -> b -> d is two levels; two more are allowed, the fifth is not.
    let d = world.ids["d"].clone();
    launch(&world.app, &world.room, &d).await;
    let e = spawn_under(&world, &d, "depth3").await;
    let token_e = launch(&world.app, &world.room, &e).await;
    let f = spawn_under(&world, &e, "depth4").await;
    let token = launch(&world.app, &world.room, &f).await;
    world.tokens.insert("f", token);
    let too_deep = room_command(&world, "f", "agent spawn depth5 --provider dev-stub").await;
    assert!(
        !too_deep.ok && error_text(&too_deep).contains("levels below"),
        "{too_deep:?}"
    );
    // Per parent: s takes sixteen, the seventeenth is refused.
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    world.tokens.insert("s", token);
    for index in 0..16 {
        spawn_under(&world, &s, &format!("s-child-{index}")).await;
    }
    let seventeenth = room_command(&world, "s", "agent spawn s-17 --provider dev-stub").await;
    assert!(
        !seventeenth.ok && error_text(&seventeenth).contains("at most 16"),
        "{seventeenth:?}"
    );
    // Per team: fill below a up to sixty-four, no parent over sixteen, then one more is refused.
    let b = world.ids["b"].clone();
    for index in 0..15 {
        spawn_under(&world, &b, &format!("b-fill-{index}")).await;
        spawn_under(&world, &d, &format!("d-fill-{index}")).await;
    }
    let mut index = 0;
    while team_size(&world).await < 64 {
        spawn_under(&world, &e, &format!("e-fill-{index}")).await;
        index += 1;
    }
    assert_eq!(team_size(&world).await, 64);
    world.tokens.insert("e", token_e);
    let over = room_command(&world, "e", "agent spawn over --provider dev-stub").await;
    assert!(
        !over.ok && error_text(&over).contains("at most 64"),
        "{over:?}"
    );
}

async fn team_size(world: &World) -> usize {
    let agents = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room);
    crate::agent::authority_limit::descendants(&world.ids["a"], &agents).len()
}

#[test]
fn a_sub_agent_works_inside_its_parents_folders() {
    run_large_stack_async_test("authority-folders", folders);
}

async fn folders() {
    let world = world("authority-folders").await;
    let inside = world._env.root.join("workspace").join("inside");
    let outside = world._env.root.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let ok = room_command(
        &world,
        "a",
        &format!(
            "agent spawn in --provider dev-stub --dir {}",
            inside.display()
        ),
    )
    .await;
    assert!(ok.ok, "{ok:?}");
    let refused = room_command(
        &world,
        "a",
        &format!(
            "agent spawn out --provider dev-stub --dir {}",
            outside.display()
        ),
    )
    .await;
    assert!(
        !refused.ok && error_text(&refused).contains("outside"),
        "{refused:?}"
    );
    assert!(!alias_exists(&world, "out").await);
    // A path that climbs out of the folder is outside too.
    let climbing = room_command(
        &world,
        "a",
        &format!(
            "agent spawn climb --provider dev-stub --dir {}/../../outside",
            inside.display()
        ),
    )
    .await;
    assert!(!climbing.ok, "{climbing:?}");
}

#[test]
fn a_deadline_is_never_later_than_the_parents_and_follows_it_down() {
    run_large_stack_async_test("authority-deadline", deadline);
}

async fn deadline() {
    let world = world("authority-deadline").await;
    let now = crate::session::unix_epoch_ms();
    let set = |agent: &str, after_ms: u64| {
        world
            .router
            .runtime_state
            .set_agent_deadline(crate::local::SetAgentDeadlineRequest {
                session_id: world.room.clone(),
                agent_id: world.ids[agent].clone(),
                deadline_ms: now + after_ms,
            })
    };
    set("a", 10 * 60_000).unwrap();
    let later = room_command(&world, "a", "agent deadline set b 2h").await;
    assert!(
        !later.ok && error_text(&later).contains("parent"),
        "{later:?}"
    );
    let sooner = room_command(&world, "a", "agent deadline set b 5m").await;
    assert!(sooner.ok, "{sooner:?}");
    // The user shortens the parent: the child and the grandchild end with it.
    set("a", 60_000).unwrap();
    for agent in ["b", "d"] {
        let got = room_command(&world, "a", &format!("agent deadline get {agent}")).await;
        let deadline_ms = got.payload["response"]["deadline"]["deadline_ms"]
            .as_u64()
            .unwrap_or(u64::MAX);
        assert!(deadline_ms <= now + 60_000, "{agent}: {deadline_ms}");
    }
}
