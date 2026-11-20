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
fn an_agent_can_spawn_eighty_idle_helpers() {
    run_large_stack_async_test("authority-wide-team", wide_team);
}

async fn wide_team() {
    let mut world = world("authority-wide-team").await;
    let s = world.ids["s"].clone();
    let token = launch(&world.app, &world.room, &s).await;
    world.tokens.insert("s", token);
    for index in 0..80 {
        let spawned = room_command(
            &world,
            "s",
            &format!("agent spawn wide-{index} --provider dev-stub"),
        )
        .await;
        assert!(spawned.ok, "helper {index} must be created: {spawned:?}");
    }
}

#[test]
fn an_agent_can_spawn_helpers_six_levels_deep() {
    run_large_stack_async_test("authority-deep-team", deep_team);
}

async fn deep_team() {
    let mut world = world("authority-deep-team").await;
    let mut parent_id = world.ids["d"].clone(); // Already two levels below the room agent.
    for depth in 3..=6 {
        let token = launch(&world.app, &world.room, &parent_id).await;
        world.tokens.insert("parent", token);
        let spawned = room_command(
            &world,
            "parent",
            &format!("agent spawn depth-{depth} --provider dev-stub"),
        )
        .await;
        assert!(
            spawned.ok,
            "helper at depth {depth} must be created: {spawned:?}"
        );
        let child = &spawned.payload["response"]["agent"];
        assert_eq!(child["parent_agent_id"], parent_id);
        parent_id = child["id"].as_str().unwrap().to_owned();
    }
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
                max_context_tokens: None,
                clear_max_context_tokens: false,
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
                reason: None,
                usage_limit: None,
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
fn a_limit_bounds_only_the_agent_that_carries_it() {
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
    // TL4: the parent's limit caps no child. A child may be given a later limit, or none.
    let later = room_command(&world, "a", "agent deadline b 2h").await;
    assert!(later.ok, "{later:?}");
    let sooner = room_command(&world, "a", "agent deadline b 5m").await;
    assert!(sooner.ok, "{sooner:?}");
    // Shortening the parent moves nothing below it.
    let before = room_command(&world, "a", "agent deadline get b").await;
    set("a", 60_000).await.unwrap();
    let after = room_command(&world, "a", "agent deadline get b").await;
    assert_eq!(
        before.payload["response"]["deadline"], after.payload["response"]["deadline"],
        "the child keeps its own limit"
    );
    let grandchild = room_command(&world, "a", "agent deadline get d").await;
    assert!(
        grandchild.payload["response"]["deadline"].is_null(),
        "a limit set above a child is not copied onto it: {grandchild:?}"
    );
    let spawned = room_command(&world, "a", "agent spawn kid --provider dev-stub").await;
    assert!(spawned.ok, "{spawned:?}");
    let kid = room_command(&world, "a", "agent deadline get kid").await;
    assert!(
        kid.payload["response"]["deadline"].is_null(),
        "a new child under a limited parent starts with no limit: {kid:?}"
    );
}

#[test]
fn a_finished_child_gets_a_new_deadline_from_its_parent_and_runs_again() {
    run_large_stack_async_test("authority-finished-deadline", finished_deadline);
}

/// Ends `agent`'s deadline the way the scheduler does, so the agent is finished.
async fn finish(world: &World, agent: &str) {
    let store = world.app.lock().await.durable_state_store().clone();
    let deadline = store
        .agent_deadline(&world.room, &world.ids[agent])
        .unwrap()
        .unwrap()
        .deadline_ms;
    for stopped in [false, true] {
        store
            .agent_lifecycle(
                crate::durable_state::agent_lifecycle::Operation::ReachDeadline {
                    room: world.room.clone(),
                    agent: world.ids[agent].clone(),
                    now: deadline,
                    stopped,
                },
            )
            .unwrap();
    }
}

async fn runs(world: &World, agent: &str) -> bool {
    world
        .app
        .lock()
        .await
        .durable_state_store()
        .ensure_agent_before_deadline(&world.room, &world.ids[agent])
        .is_ok()
}

#[test]
fn editing_a_middle_agent_does_not_reapply_an_ended_ancestors_limit_to_the_revived_agent_below() {
    run_large_stack_async_test("authority-revived-below-middle", revived_below_middle);
}

/// a -> b -> d: a's limit ends the whole team; the owner revives d, then edits b while a stays ended.
async fn revived_below_middle() {
    let world = world("authority-revived-below-middle").await;
    let now = crate::session::unix_epoch_ms();
    let set = |agent: &str, deadline_ms: Option<u64>| {
        world.router.runtime_state.set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: world.room.clone(),
                agent_id: world.ids[agent].clone(),
                deadline_ms,
            },
        )
    };
    // a's limit passes for real: the intent it keeps is a time in the past.
    set("a", Some(now + 300)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    for agent in ["a", "b", "d"] {
        finish(&world, agent).await;
    }
    assert!(!runs(&world, "d").await, "a's ended limit ended d too");

    set("d", None).await.unwrap();
    assert!(runs(&world, "d").await, "the owner revived d");

    // Extending b gives d b's limit, not a's ended one.
    set("b", Some(now + 8 * 60_000)).await.unwrap();
    assert!(
        runs(&world, "d").await,
        "extending b wrote a's ended limit onto the revived d"
    );
    // Clearing b leaves d without a limit; a's retained limit is gone from the team.
    set("b", None).await.unwrap();
    assert!(
        runs(&world, "d").await,
        "clearing b wrote a's ended limit onto the revived d"
    );
}

async fn finished_deadline() {
    let world = world("authority-finished-deadline").await;
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
    set("b", 5 * 60_000).await.unwrap();
    finish(&world, "b").await;
    assert!(
        !runs(&world, "b").await,
        "the reached deadline ends b's run"
    );

    // The parent's limit does not bound b: the parent may give a later time, or none.
    let later = room_command(&world, "a", "agent deadline b 2h").await;
    assert!(later.ok, "{later:?}");
    assert!(runs(&world, "b").await, "the new limit revived b");
    finish(&world, "b").await;
    assert!(!runs(&world, "b").await);
    // D2: a refusal names who can change the limit.
    let refused = room_command(&world, "peer", "agent deadline b 1h").await;
    assert!(!refused.ok, "{refused:?}");
    assert!(!runs(&world, "b").await, "a refused edit changes nothing");

    // D1: the parent gives the finished child a new deadline, and b runs again.
    let extended = room_command(&world, "a", "agent deadline b 8m").await;
    assert!(extended.ok, "{extended:?}");
    assert!(runs(&world, "b").await);
    let got = room_command(&world, "a", "agent deadline get b").await;
    assert_eq!(
        got.payload["response"]["deadline"]["state"], "armed",
        "{got:?}"
    );

    // D1: the parent's own deadline ended too: the owner clears or extends the child.
    finish(&world, "b").await;
    set("a", 60_000).await.unwrap();
    finish(&world, "a").await;
    for _ in 0..2 {
        assert!(!runs(&world, "b").await);
        let reopened = world
            .router
            .runtime_state
            .set_agent_deadline(
                crate::session::DEFAULT_LOCAL_USER_ID,
                crate::local::SetAgentDeadlineRequest {
                    session_id: world.room.clone(),
                    agent_id: world.ids["b"].clone(),
                    deadline_ms: None,
                },
            )
            .await;
        assert!(reopened.is_ok(), "{reopened:?}");
        assert!(runs(&world, "b").await);
        set("b", 60_000).await.unwrap();
        finish(&world, "b").await;
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
    // The authority limit refuses it the same way.
    set_level(&world, "s", AgentPermissionLevel::Required).await;
    let target = world._env.root.join("wt-cap");
    let refused = room_command(
        &world,
        "s",
        &format!(
            "agent spawn capped --provider dev-stub --permission yolo --worktree {} --branch cap-branch",
            target.display()
        ),
    )
    .await;
    assert!(
        !refused.ok && error_text(&refused).contains("more authority"),
        "{refused:?}"
    );
    assert!(!target.exists(), "authority refusal left a checkout");
    assert!(
        git(&workspace, &["branch", "--list", "cap-branch"])
            .trim()
            .is_empty(),
        "authority refusal left a branch"
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
            "agent spawn kept --provider dev-stub --permission yolo --worktree {} --branch kept-branch",
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
