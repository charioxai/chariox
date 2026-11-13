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

mod sibling_worktrees;

#[test]
fn a_parent_cannot_move_a_sub_agent_out_of_its_own_folders_by_config() {
    run_large_stack_async_test("authority-folders-config", folders_by_config);
}

async fn folders_by_config() {
    let world = world("authority-folders-config").await;
    let inside = world._env.root.join("workspace").join("inside");
    let outside = world._env.root.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let a = world
        .app
        .lock()
        .await
        .agents()
        .get_agent(&world.ids["a"])
        .unwrap();
    let configure = |workspace: Option<&std::path::Path>, worktree: Option<&std::path::Path>| {
        let request =
            LocalDaemonRequest::UpdateAgentConfig(crate::local::UpdateAgentConfigRequest {
                session_id: world.room.clone(),
                agent_id: world.ids["s"].clone(),
                execution_mode: None,
                clear_execution_mode: false,
                permission_level: None,
                clear_permission_level: false,
                workspace_id: workspace.map(|path| path.display().to_string()),
                clear_workspace_id: false,
                worktree_id: worktree.map(|path| path.display().to_string()),
                clear_worktree_id: false,
            });
        let command = KernelCommand::from_local_request_with_caller(
            "authority-folders-config",
            KernelCommandSource::DaemonBackground,
            KernelCaller {
                caller_id: format!("metaagent:{}", a.id()),
                caller_kind: KernelCallerKind::Metaagent,
                user_id: Some(a.owner_user_id().to_string()),
                client_id: None,
                machine_id: None,
                realm_id: None,
                public_key_thumbprint: None,
                metaagent_id: Some(a.id().to_string()),
                connection_class: Some(crate::local::KernelConnectionClass::KernelAgent),
            },
            None,
            None,
            &request,
        );
        world.router.dispatch(command, request)
    };
    assert!(
        configure(None, Some(&inside)).await.is_ok(),
        "a folder under the parent's workspace is allowed"
    );
    for (workspace, worktree) in [
        (Some(outside.as_path()), None),
        (None, Some(outside.as_path())),
    ] {
        let moved = configure(workspace, worktree).await;
        assert!(
            moved.is_err(),
            "a sub-agent was configured outside its parent's folders: {moved:?}"
        );
    }
}

#[test]
fn a_deadline_is_never_later_than_the_parents_and_follows_it_down() {
    run_large_stack_async_test("authority-deadline", deadline);
}

async fn deadline() {
    let world = world("authority-deadline").await;
    let now = crate::session::unix_epoch_ms();
    let set = |agent: &str, after_ms: u64| {
        world.router.runtime_state.set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: world.room.clone(),
                agent_id: world.ids[agent].clone(),
                deadline_ms: Some(now + after_ms),
            },
        )
    };
    set("a", 10 * 60_000).await.unwrap();
    let later = room_command(&world, "a", "agent deadline b 2h").await;
    assert!(
        !later.ok && error_text(&later).contains("parent"),
        "{later:?}"
    );
    let sooner = room_command(&world, "a", "agent deadline b 5m").await;
    assert!(sooner.ok, "{sooner:?}");
    // The user shortens the parent: the child and the grandchild end with it.
    set("a", 60_000).await.unwrap();
    for agent in ["b", "d"] {
        let got = room_command(&world, "a", &format!("agent deadline get {agent}")).await;
        let deadline_ms = got.payload["response"]["deadline"]["deadline_ms"]
            .as_u64()
            .unwrap_or(u64::MAX);
        assert!(deadline_ms <= now + 60_000, "{agent}: {deadline_ms}");
    }
}

/// G and its child P start at Build/Yolo; P keeps those preferences after its tree limit drops.
async fn bold_pair(world: &mut World, g: &'static str, p: &'static str) {
    use crate::provider::AgentExecutionMode::Build;
    let parent = {
        let mut app = world.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&world.room, "dev-stub")
                    .with_alias(g)
                    .with_permission_level_override(AgentPermissionLevel::Yolo)
                    .with_execution_mode_override(Build),
            )
            .unwrap()
    };
    let token = launch(&world.app, &world.room, parent.id()).await;
    world.tokens.insert(g, token);
    world.ids.insert(g, parent.id().to_owned());
    let spawned = room_command(world, g, &format!("agent spawn {p} --provider dev-stub")).await;
    assert!(spawned.ok, "{spawned:?}");
    let child = spawned.payload["response"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = launch(&world.app, &world.room, &child).await;
    world.tokens.insert(p, token);
    world.ids.insert(p, child);
    let child = agent_by_alias(world, p).await;
    assert_eq!(
        (
            child.execution_mode_override(),
            child.permission_level_override()
        ),
        (Some(Build), Some(AgentPermissionLevel::Yolo))
    );
}

async fn agent_by_alias(world: &World, alias: &str) -> crate::agent::AgentInstance {
    world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room)
        .into_iter()
        .find(|agent| agent.alias() == Some(alias))
        .unwrap()
}

/// An unqualified spawn from P starts at P's current tree limit, and an explicit request above
/// that limit is still refused.
async fn assert_spawn_takes_the_tree_limit(world: &World, p: &str, helper: &str) {
    use crate::provider::AgentExecutionMode::Plan;
    let spawned = room_command(
        world,
        p,
        &format!("agent spawn {helper} --provider dev-stub"),
    )
    .await;
    assert!(spawned.ok, "no elevation was requested: {spawned:?}");
    let child = agent_by_alias(world, helper).await;
    assert_eq!(
        (
            child.execution_mode_override(),
            child.permission_level_override()
        ),
        (Some(Plan), Some(AgentPermissionLevel::Required)),
        "the child takes its parent's effective limit, not its stored preferences"
    );
    let bold = room_command(
        world,
        p,
        &format!("agent spawn {helper}-bold --provider dev-stub --permission yolo"),
    )
    .await;
    assert!(
        !bold.ok && error_text(&bold).contains("more authority than its parent"),
        "{bold:?}"
    );
    assert!(!alias_exists(world, &format!("{helper}-bold")).await);
}

#[test]
fn a_nested_spawn_after_the_ancestor_was_lowered_takes_the_tree_limit() {
    run_large_stack_async_test("authority-lowered-ancestor", lowered_ancestor);
}

async fn lowered_ancestor() {
    use crate::provider::AgentExecutionMode::Plan;
    let mut world = world("authority-lowered-ancestor").await;
    bold_pair(&mut world, "g", "p").await;
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["g"],
            Some(Some(Plan)),
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    assert_spawn_takes_the_tree_limit(&world, "p", "helper").await;
}

#[test]
fn a_nested_spawn_after_a_move_under_a_lower_root_takes_the_tree_limit() {
    run_large_stack_async_test("authority-reparented", reparented);
}

async fn reparented() {
    use crate::provider::AgentExecutionMode::Plan;
    let mut world = world("authority-reparented").await;
    bold_pair(&mut world, "g", "p").await;
    let root = {
        let mut app = world.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&world.room, "dev-stub")
                    .with_alias("low-root")
                    .with_permission_level_override(AgentPermissionLevel::Required)
                    .with_execution_mode_override(Plan),
            )
            .unwrap()
    };
    let request = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.room.clone(),
        agent_id: world.ids["p"].clone(),
        parent_agent_id: Some(root.id().to_owned()),
    });
    let mut command = KernelCommand::from_local_request("user-move-p", None, None, &request);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let moved = world.router.dispatch(command, request).await.unwrap();
    assert!(matches!(
        moved,
        LocalDaemonResponse::AgentParentChanged { .. }
    ));
    assert_spawn_takes_the_tree_limit(&world, "p", "helper").await;
}

#[test]
fn a_nested_spawn_in_a_bolder_room_takes_the_tree_limit_not_the_room_defaults() {
    run_large_stack_async_test("authority-room-defaults", room_defaults);
}

async fn room_defaults() {
    use crate::provider::AgentExecutionMode::{Build, Plan};
    let mut world = world("authority-room-defaults").await;
    let workspace = world._env.root.join("workspace");
    let (room, root) = {
        let mut app = world.app.lock().await;
        let mut sessions = crate::app::KernelSessionService::new(&mut app);
        let (room, _) = sessions
            .create_session(
                CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy())
                    .with_agent_defaults(
                        crate::session::SessionAgentDefaults::new("dev-stub")
                            .with_execution_mode(Build)
                            .with_permission_level(AgentPermissionLevel::Yolo),
                    ),
            )
            .unwrap();
        let root = sessions
            .spawn_agent(
                CreateAgentRequest::new(room.id(), "dev-stub")
                    .with_alias("low-root")
                    .with_permission_level_override(AgentPermissionLevel::Required)
                    .with_execution_mode_override(Plan),
            )
            .unwrap();
        (room.id().to_owned(), root)
    };
    world.room = room;
    let token = launch(&world.app, &world.room, root.id()).await;
    world.tokens.insert("low-root", token);
    let spawned = room_command(&world, "low-root", "agent spawn p --provider dev-stub").await;
    assert!(spawned.ok, "{spawned:?}");
    let child = spawned.payload["response"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // No settings of its own: it runs at the lower of the room defaults and its parent.
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(&child, Some(None), Some(None), None, None)
        .unwrap();
    let p = agent_by_alias(&world, "p").await;
    assert_eq!(
        (p.execution_mode_override(), p.permission_level_override()),
        (None, None)
    );
    let token = launch(&world.app, &world.room, &child).await;
    world.tokens.insert("p", token);
    assert_spawn_takes_the_tree_limit(&world, "p", "helper").await;
}

#[test]
fn children_that_finished_do_not_count_against_the_sixteen_a_parent_runs() {
    run_large_stack_async_test("authority-finished-children", finished_children);
}

async fn finished_children() {
    let world = world("authority-finished-children").await;
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    let mut world = world;
    world.tokens.insert("s", token);
    let mut children = Vec::new();
    for index in 0..16 {
        children.push(spawn_under(&world, &s, &format!("done-{index}")).await);
    }
    // A child that was never prompted is a prepared agent, not a finished one.
    let waiting = room_command(&world, "s", "agent spawn waiting --provider dev-stub").await;
    assert!(
        !waiting.ok && error_text(&waiting).contains("at most 16"),
        "{waiting:?}"
    );
    // Once each child has had its turn and is idle again, none of them is running.
    {
        let app = world.app.lock().await;
        for child in &children {
            app.agents().note_prompt_sent_at(child, 1).unwrap();
        }
    }
    let next = room_command(&world, "s", "agent spawn next --provider dev-stub").await;
    assert!(
        next.ok,
        "finished children must not block a spawn: {next:?}"
    );
    // Children that are working do count.
    {
        let app = world.app.lock().await;
        for child in &children {
            app.agents()
                .set_agent_state(child, crate::agent::AgentState::Working)
                .unwrap();
        }
    }
    let over = room_command(&world, "s", "agent spawn over --provider dev-stub").await;
    assert!(
        !over.ok && error_text(&over).contains("at most 16"),
        "sixteen working children are the cap: {over:?}"
    );
}

#[test]
fn children_with_a_prompt_through_real_admission_keep_their_places() {
    run_large_stack_async_test("authority-prompted-children", prompted_children);
}

async fn prompted_children() {
    let mut world = world("authority-prompted-children").await;
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    world.tokens.insert("s", token);
    for index in 0..16 {
        spawn_under(&world, &s, &format!("busy-{index}")).await;
        let prompted =
            room_command(&world, "s", &format!("prompt busy-{index} \"keep going\"")).await;
        assert!(prompted.ok, "{prompted:?}");
    }
    // The prompt left the stored state alone, but each child is running its turn.
    {
        let app = world.app.lock().await;
        let child = app
            .agents()
            .get_session_agents(&world.room)
            .into_iter()
            .find(|agent| agent.alias() == Some("busy-0"))
            .unwrap();
        assert!(child.last_prompt_sent_at_ms().is_some());
        assert_ne!(child.state(), crate::agent::AgentState::Working);
    }
    let over = room_command(&world, "s", "agent spawn over --provider dev-stub").await;
    assert!(
        !over.ok && error_text(&over).contains("at most 16"),
        "sixteen children running a prompt are the cap: {over:?}"
    );
}

fn git(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_refused_spawn_leaves_no_checkout_or_branch_behind() {
    run_large_stack_async_test("authority-spawn-atomic", refused_spawn_is_atomic);
}

async fn refused_spawn_is_atomic() {
    let mut world = world("authority-spawn-atomic").await;
    let workspace = world._env.root.join("workspace");
    git(&workspace, &["init", "-q"]);
    git(&workspace, &["commit", "-q", "--allow-empty", "-m", "x"]);
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    world.tokens.insert("s", token);
    // An alias that is taken refuses the spawn after the checkout would have been made.
    let target = world._env.root.join("wt-alias");
    let refused = room_command(
        &world,
        "a",
        &format!(
            "agent spawn s --provider dev-stub --worktree {} --branch alias-branch",
            target.display()
        ),
    )
    .await;
    assert!(!refused.ok, "{refused:?}");
    assert!(
        !target.exists(),
        "alias refusal left a checkout: {refused:?}"
    );
    assert!(
        git(&workspace, &["branch", "--list", "alias-branch"])
            .trim()
            .is_empty(),
        "alias refusal left a branch"
    );
    // The cap refuses it the same way.
    for index in 0..16 {
        spawn_under(&world, &s, &format!("fill-{index}")).await;
    }
    let target = world._env.root.join("wt-cap");
    let refused = room_command(
        &world,
        "s",
        &format!(
            "agent spawn capped --provider dev-stub --worktree {} --branch cap-branch",
            target.display()
        ),
    )
    .await;
    assert!(
        !refused.ok && error_text(&refused).contains("at most 16"),
        "{refused:?}"
    );
    assert!(!target.exists(), "cap refusal left a checkout");
    assert!(
        git(&workspace, &["branch", "--list", "cap-branch"])
            .trim()
            .is_empty(),
        "cap refusal left a branch"
    );
    assert_eq!(
        git(&workspace, &["worktree", "list"]).lines().count(),
        1,
        "git still lists a worktree for the refused spawn"
    );
    // A branch that existed before the spawn is not the spawn's to delete.
    git(&workspace, &["branch", "kept-branch"]);
    let target = world._env.root.join("wt-kept");
    let refused = room_command(
        &world,
        "s",
        &format!(
            "agent spawn kept --provider dev-stub --worktree {} --branch kept-branch",
            target.display()
        ),
    )
    .await;
    assert!(!refused.ok, "{refused:?}");
    assert!(!target.exists());
    assert!(
        !git(&workspace, &["branch", "--list", "kept-branch"])
            .trim()
            .is_empty(),
        "a refused spawn deleted a branch it did not create"
    );
}

fn fail_events_of_kind(world: &World, kind: &str, message: &str) {
    let path = world
        .app
        .try_lock()
        .expect("app is idle")
        .durable_state_store()
        .path()
        .to_owned();
    rusqlite::Connection::open(path)
        .expect("failure injection connection should open")
        .execute_batch(&format!(
            "CREATE TRIGGER fail_{kind_name} BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = '{kind}'
             BEGIN SELECT RAISE(FAIL, '{message}'); END;",
            kind_name = kind.replace('.', "_"),
        ))
        .expect("append failure should install");
}

fn checkout_and_branch_exist(
    workspace: &std::path::Path,
    target: &std::path::Path,
    branch: &str,
) -> bool {
    target.exists()
        && !git(workspace, &["branch", "--list", branch])
            .trim()
            .is_empty()
}

#[test]
fn a_single_agent_registered_before_the_failure_keeps_its_checkout() {
    run_large_stack_async_test("authority-spawn-retained", single_agent_keeps_checkout);
}

async fn single_agent_keeps_checkout() {
    let mut world = world("authority-spawn-retained").await;
    let workspace = world._env.root.join("workspace");
    git(&workspace, &["init", "-q"]);
    git(&workspace, &["commit", "-q", "--allow-empty", "-m", "x"]);
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    world.tokens.insert("s", token);
    // The agent is registered before its creation event is written, so the spawn fails with the
    // agent in place.
    fail_events_of_kind(&world, "agent.created", "injected agent.created failure");
    let target = workspace.join("wt-single");
    let failed = room_command(
        &world,
        "s",
        &format!(
            "agent spawn single --provider dev-stub --worktree {} --branch single-branch",
            target.display()
        ),
    )
    .await;
    assert!(
        !failed.ok && error_text(&failed).contains("injected agent.created failure"),
        "{failed:?}"
    );
    assert!(
        alias_exists(&world, "single").await,
        "the agent stays registered"
    );
    assert!(
        checkout_and_branch_exist(&workspace, &target, "single-branch"),
        "a registered agent lost its checkout"
    );
}

#[test]
fn a_batch_registered_before_the_failure_keeps_its_checkouts() {
    run_large_stack_async_test("authority-batch-retained", batch_keeps_checkouts);
}

async fn batch_keeps_checkouts() {
    let world = world("authority-batch-retained").await;
    let workspace = world._env.root.join("workspace");
    git(&workspace, &["init", "-q"]);
    git(&workspace, &["commit", "-q", "--allow-empty", "-m", "x"]);
    // Every agent is registered when the batch event fails.
    fail_events_of_kind(&world, "agents.created", "injected agents.created failure");
    let first = workspace.join("wt-batch-1");
    let second = workspace.join("wt-batch-2");
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "SpawnAgents": {"session_id": world.room, "agents": [
            {"provider": "dev-stub", "alias": "batch-1", "worktree_placement":
                {"target_directory": first, "branch": "batch-1", "from_ref": null}},
            {"provider": "dev-stub", "alias": "batch-2", "worktree_placement":
                {"target_directory": second, "branch": "batch-2", "from_ref": null}},
        ]}
    }))
    .expect("public request");
    let mut command = KernelCommand::from_local_request("batch", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    let failed = world
        .router
        .dispatch(command, request)
        .await
        .expect_err("the batch event fails");
    assert!(
        failed
            .to_string()
            .contains("injected agents.created failure"),
        "{failed}"
    );
    for (alias, target) in [("batch-1", &first), ("batch-2", &second)] {
        assert!(
            alias_exists(&world, alias).await,
            "{alias} stays registered"
        );
        assert!(
            checkout_and_branch_exist(&workspace, target, alias),
            "{alias} lost its checkout"
        );
    }
}
