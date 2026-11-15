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
