use super::*;

fn init_repo(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["commit", "-q", "--allow-empty", "-m", "fixture"]);
}

#[test]
fn a_sub_agent_can_use_a_sibling_worktree_of_its_parents_repository() {
    run_large_stack_async_test("authority-sibling-worktree", existing_sibling);
}

async fn existing_sibling() {
    let world = world("authority-sibling-worktree").await;
    let workspace = world._env.root.join("workspace");
    init_repo(&workspace);
    let sibling = world._env.root.join("sibling");
    git(
        &workspace,
        &[
            "worktree",
            "add",
            "-b",
            "sibling",
            sibling.to_str().unwrap(),
        ],
    );
    let result = room_command(
        &world,
        "a",
        &format!(
            "agent spawn wt --provider dev-stub --dir {}",
            sibling.display()
        ),
    )
    .await;
    assert!(result.ok, "{result:?}");
    let agents = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room);
    let child = agents.iter().find(|a| a.alias() == Some("wt")).unwrap();
    assert_eq!(child.parent_agent_id(), Some(world.ids["a"].as_str()));
    assert_eq!(child.worktree_id(), sibling.to_str());

    // A linked worktree can itself be a parent, and its child may use a subfolder
    // of another checkout of that repository.
    let token = launch(&world.app, &world.room, child.id()).await;
    let third = world._env.root.join("third");
    git(
        &workspace,
        &["worktree", "add", "-b", "third", third.to_str().unwrap()],
    );
    let subfolder = third.join("src");
    std::fs::create_dir_all(&subfolder).unwrap();
    let result = world
        .router
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.room.run_command",
            serde_json::json!({"command": format!(
                "agent spawn grandchild --provider dev-stub --dir {}", subfolder.display()
            )}),
        )
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
}

#[test]
fn a_sub_agent_can_create_a_sibling_worktree_of_its_parents_repository() {
    run_large_stack_async_test("authority-create-sibling-worktree", create_sibling);
}

async fn create_sibling() {
    let world = world("authority-create-sibling-worktree").await;
    let workspace = world._env.root.join("workspace");
    init_repo(&workspace);
    let sibling = world._env.root.join("workers").join("sibling");
    let result = room_command(
        &world,
        "a",
        &format!(
            "agent spawn wt --provider dev-stub --worktree {} --branch sibling",
            sibling.display()
        ),
    )
    .await;
    assert!(result.ok, "{result:?}");
    assert_eq!(
        git(
            &workspace,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        ),
        git(
            &sibling,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        )
    );
    assert_eq!(
        git(&sibling, &["branch", "--show-current"]).trim(),
        "sibling"
    );
}

#[test]
fn a_sub_agent_still_refuses_unrelated_directories_and_repositories() {
    run_large_stack_async_test("authority-unrelated-worktree", unrelated);
}

#[test]
fn slice_worktree_refuses_unrelated_directories_with_a_next_step() {
    run_large_stack_async_test("slice-unrelated-worktree", slice_refuses_unrelated);
}

async fn slice_refuses_unrelated() {
    let world = world("slice-unrelated-worktree").await;
    let workspace = world._env.root.join("workspace");
    init_repo(&workspace);
    let unrelated = world._env.root.join("unrelated");
    std::fs::create_dir_all(&unrelated).unwrap();
    let unrelated_repo = world._env.root.join("other-repo");
    init_repo(&unrelated_repo);
    let stale = world._env.root.join("replaced-worktree");
    git(
        &workspace,
        &["worktree", "add", "--detach", stale.to_str().unwrap()],
    );
    std::fs::rename(&stale, world._env.root.join("moved-worktree")).unwrap();
    init_repo(&stale);
    for directory in [&unrelated, &unrelated_repo, &stale] {
        let result = room_command(
            &world,
            "a",
            &format!(
                "agent spawn worker --slice new:headless --dir {}",
                directory.display()
            ),
        )
        .await;
        let message = error_text(&result);
        assert!(!result.ok, "{result:?}");
        assert!(
            message.starts_with(&format!("`{}` ", directory.display())),
            "{message}"
        );
        assert!(
            message.contains("is not a Git worktree of Workspace"),
            "{message}"
        );
        assert!(message.contains("Run `git worktree list`"), "{message}");
        assert!(message.contains(workspace.to_str().unwrap()), "{message}");
    }
    assert!(!alias_exists(&world, "worker").await);
    assert!(world.app.lock().await.slices().list().is_empty());
}

#[test]
fn slice_worktree_agent_deletion_cleans_up_only_the_unused_agent_slice() {
    run_large_stack_async_test("slice-agent-deletion", delete_slice_agent);
}

async fn delete_slice_agent() {
    use crate::slice::{CreateSliceInput, SliceBackendKind, SliceDisplayMode};

    let world = world("slice-agent-deletion").await;
    let (temporary, reusable) = {
        let app = world.app.lock().await;
        let store = app.slices();
        let create = |name: &str, delete_when_unused| {
            store
                .create_with_agent_lifetime(
                    &app.config().daemon_id,
                    &app.config().host_machine_id,
                    CreateSliceInput {
                        source_slice_ref: None,
                        name: name.into(),
                        // Exercise the kernel deletion path without invoking Docker.
                        backend: SliceBackendKind::SshDocker,
                        os: "linux".into(),
                        display_mode: SliceDisplayMode::Headless,
                        display_backend: Default::default(),
                        workspace_id: None,
                        worktree_id: None,
                        workspace_mount: None,
                        development: None,
                        worker_kernel_ref: None,
                        display_url: None,
                        provider_auth: Vec::new(),
                        from_saved_state: None,
                        now_ms: 0,
                    },
                    delete_when_unused,
                )
                .unwrap()
        };
        let temporary = create("temporary", true);
        let reusable = create("reusable", false);
        for (slice, alias) in [(&temporary, "b"), (&temporary, "s"), (&reusable, "s")] {
            store
                .attach_agent(&slice.id, &world.room, &world.ids[alias], 1)
                .unwrap();
        }
        (temporary, reusable)
    };
    let deleted = room_command(&world, "a", "agent delete b").await;
    assert!(deleted.ok, "{deleted:?}");
    assert!(world
        .app
        .lock()
        .await
        .slices()
        .resolve(&temporary.id)
        .is_ok());

    let deleted = room_command(&world, "a", "agent delete s").await;
    assert!(deleted.ok, "{deleted:?}");
    let app = world.app.lock().await;
    assert!(app.slices().resolve(&temporary.id).is_err());
    assert!(app.slices().resolve(&reusable.id).is_ok());
}

async fn unrelated() {
    let world = world("authority-unrelated-worktree").await;
    let workspace = world._env.root.join("workspace");
    init_repo(&workspace);
    let unrelated = world._env.root.join("unrelated");
    std::fs::create_dir_all(&unrelated).unwrap();
    let other_repo = world._env.root.join("other-repo");
    init_repo(&other_repo);
    for (alias, path) in [("plain", &unrelated), ("other", &other_repo)] {
        let result = room_command(
            &world,
            "a",
            &format!(
                "agent spawn {alias} --provider dev-stub --dir {}",
                path.display()
            ),
        )
        .await;
        assert!(!result.ok, "{result:?}");
        assert!(
            error_text(&result).contains("same Git repository"),
            "{result:?}"
        );
        assert!(!alias_exists(&world, alias).await);
    }
    #[cfg(unix)]
    {
        let alias = workspace.join("escape");
        std::os::unix::fs::symlink(&other_repo, &alias).unwrap();
        let result = room_command(
            &world,
            "a",
            &format!(
                "agent spawn escape --provider dev-stub --dir {}",
                alias.display()
            ),
        )
        .await;
        assert!(!result.ok, "{result:?}");
    }
    // A nested repository is within the ordinary folder grant, but cannot be
    // used as the source of a new checkout outside that grant.
    let nested = workspace.join("nested-repo");
    init_repo(&nested);
    let target = world._env.root.join("wrong-source");
    let result = world
        .router
        .runtime_state
        .spawn_agent(
            CreateAgentRequest::new(&world.room, "dev-stub")
                .with_parent_agent_id(&world.ids["a"])
                .with_worktree(nested.to_str().unwrap())
                .with_worktree_placement(crate::agent::GitWorktreePlacement {
                    target_directory: Some(target.display().to_string()),
                    branch: Some("wrong".into()),
                    from_ref: None,
                }),
        )
        .await;
    assert!(result.is_err() && !target.exists(), "{result:?}");
}
