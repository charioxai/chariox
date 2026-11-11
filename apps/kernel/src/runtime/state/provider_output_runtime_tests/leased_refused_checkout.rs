// MP-08 / MP-11: a leased spawn that is refused after the worker made its checkout removes the
// checkout and the branch it created, like a local spawn does.
use crate::project_environment::{
    ProjectEnvironmentRecord, ProjectEnvironmentStore, PROJECT_ENVIRONMENT_SCHEMA,
};

fn git(directory: &std::path::Path, args: &[&str]) -> String {
    let output = crate::process_spawn::command("git")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test]
async fn a_refused_leased_spawn_leaves_no_checkout_or_branch_on_the_worker() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-refused-checkout");
    let main = root.path().join("main");
    let target = root.path().join("placed");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(
        &main,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Initial",
        ],
    );
    // A project file too large to copy into a new checkout refuses the spawn after the checkout
    // exists.
    let too_large = std::fs::File::create(main.join("big.local")).unwrap();
    too_large.set_len(65 * 1024 * 1024).unwrap();
    let config = crate::config::DaemonConfig::for_tests();
    let mut app = crate::test_support::bootstrap_authenticated_app(config.clone()).unwrap();
    app.session_state_store()
        .restore_projects(vec![crate::session::RuntimeProject::new(
            "main",
            "owner",
            main.display().to_string(),
            "Main",
            crate::session::RuntimeProjectKind::Named,
        )]);
    ProjectEnvironmentStore::new(&config.private_runtime_state_root())
        .save(
            "main",
            0,
            ProjectEnvironmentRecord {
                schema: PROJECT_ENVIRONMENT_SCHEMA,
                files: vec!["big.local".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "home-session", "home-agent", false, "owner")
        .unwrap();
    let refused = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent_from_base_directory(
            &main,
            &lease.id,
            "dev-stub",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(crate::agent::GitWorktreePlacement {
                target_directory: Some(target.display().to_string()),
                branch: Some("placed".into()),
                from_ref: Some("HEAD".into()),
            }),
        );
    let error = refused
        .expect_err("the oversized file refuses the spawn")
        .to_string();
    assert!(error.contains("too large"), "{error}");
    assert!(
        !target.exists(),
        "the refused spawn left a checkout on the worker"
    );
    assert!(
        git(&main, &["branch", "--list", "placed"])
            .trim()
            .is_empty(),
        "the refused spawn left its branch"
    );
    assert_eq!(git(&main, &["worktree", "list"]).lines().count(), 1);
}
