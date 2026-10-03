//! MP-08 / MP-10 / MP-11: a General kernel can execute its authenticated leases.
use super::*;

#[test]
fn mp08_mp10_mp11_general_kernel_resolves_leased_setup_target() {
    let (mut app, agent) = leased_agent_fixture(false);
    assert_eq!(
        app.config().kernel_runtime_role,
        crate::config::KernelRuntimeRole::General
    );
    let runtime = RemoteLeaseRuntime::new(&mut app);
    let target = runtime
        .project_environment_setup_target(&agent.id, "home-session", "home-agent", None)
        .expect("ordinary and slice kernels must resolve their own leased setup target");
    assert_eq!(target.backing_session_id, agent.backing_session_id);
    assert_eq!(target.backing_agent_id, agent.backing_agent_id);

    for (session, home_agent, workspace, expected) in [
        ("foreign-session", "home-agent", None, "home setup identity"),
        ("home-session", "foreign-agent", None, "home setup identity"),
        (
            "home-session",
            "home-agent",
            Some("/foreign-worktree"),
            "home setup workspace",
        ),
    ] {
        let error = runtime
            .project_environment_setup_target(&agent.id, session, home_agent, workspace)
            .expect_err("removing the placement gate must preserve lease identity checks");
        assert!(error.to_string().contains(expected));
    }
}

#[tokio::test]
async fn mp08_mp10_mp11_general_kernel_admits_and_cancels_leased_setup() {
    let (mut app, agent) = leased_agent_fixture(false);
    let worker = app.config().host_machine_id.clone();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    // Resolve without the setup-specific wrapper so this regression independently
    // catches the second placement gate at setup admission.
    let target = RemoteLeaseRuntime::new(&mut app)
        .leased_project_target(&agent.id, "home-session", "home-agent", None)
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    for (requested_worker, requested_platform, requested_workspace, expected) in [
        ("foreign-worker", platform.as_str(), "", "worker identity"),
        (worker.as_str(), "foreign-platform", "", "platform"),
        (
            worker.as_str(),
            platform.as_str(),
            "/foreign-worktree",
            "leased worker worktree",
        ),
    ] {
        let error = runtime
            .start_leased_project_environment_setup(
                target.clone(),
                agent.id.clone(),
                "general-leased-setup".into(),
                1,
                "project-1".into(),
                requested_workspace.into(),
                requested_worker.into(),
                requested_platform.into(),
                None,
                vec!["true".into()],
            )
            .await
            .expect_err("the ordinary kernel must reject a mismatched setup target");
        assert!(error.to_string().contains(expected));
    }
    let admitted = runtime
        .start_leased_project_environment_setup(
            target.clone(),
            agent.id.clone(),
            "general-leased-setup".into(),
            1,
            "project-1".into(),
            target.workspace_id.clone(),
            worker,
            platform,
            None,
            vec!["true".into()],
        )
        .await
        .expect("an ordinary kernel must admit setup for its authenticated lease");
    assert_eq!(
        admitted.status.phase,
        crate::local::ProjectEnvironmentSetupPhase::Requested
    );
    let observed = runtime
        .get_leased_project_environment_setup_status(
            target.clone(),
            &agent.id,
            "general-leased-setup",
        )
        .await
        .unwrap();
    assert_eq!(observed.status, admitted.status);
    let mut foreign = target.clone();
    foreign.home_agent_id = "foreign-agent".into();
    let error = runtime
        .get_leased_project_environment_setup_status(foreign, &agent.id, "general-leased-setup")
        .await
        .expect_err("setup status remains bound to the leased home agent");
    assert!(error.to_string().contains("leased home agent"));
    let cancelled = runtime
        .cancel_leased_project_environment_setup(target, &agent.id, "general-leased-setup")
        .await
        .unwrap();
    assert_eq!(
        cancelled.status.phase,
        crate::local::ProjectEnvironmentSetupPhase::Cancelled
    );
}
