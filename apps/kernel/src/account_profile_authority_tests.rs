//! MP-08 / MP-10 / MP-11: exact account IDs retain their runtime owner namespace.
use super::*;

#[test]
fn cloud_lease_account_owner_preserves_exact_profile_and_rejects_other_owners() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("cloud-lease-account-owner");
    let registry = ProviderAccountProfileRegistry::open(root.path().join("profiles.json")).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    let mut profiles = Vec::new();
    for owner in ["local", "cloud-owner", "other-owner"] {
        let profile = registry
            .migrate_effective_defaults(owner, &root.path().join(owner))
            .unwrap()
            .into_iter()
            .find(|profile| profile.provider == "opencode")
            .unwrap();
        registry
            .update_observation(
                owner,
                "opencode",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        profiles.push(profile);
    }
    let mut sessions = crate::session::SessionService::new(&config);
    let session = sessions
        .create_session(
            crate::session::CreateSessionRequest::new("workspace", root.path().to_string_lossy())
                .with_owner_user_id("cloud-owner"),
        )
        .unwrap();
    let mut agents = crate::agent::AgentService::new();
    let mut request = crate::agent::CreateAgentRequest::new(session.id(), "opencode")
        .with_owner_user_id("cloud-owner")
        .with_account_profile(profiles[1].profile_id.clone());
    let agent = agents.create_agent(request.clone(), &mut sessions).unwrap();
    registry
        .require_agent_authenticated(
            &owner(&config, &registry, &agent),
            &agent,
            "MP-08/MP-10/MP-11 lease account",
        )
        .unwrap();
    request = request.with_account_profile(profiles[0].profile_id.clone());
    let local = agents.create_agent(request.clone(), &mut sessions).unwrap();
    registry
        .require_agent_authenticated(
            &owner(&config, &registry, &local),
            &local,
            "MP-08/MP-10/MP-11 home alias",
        )
        .unwrap();
    request = request.with_account_profile(profiles[2].profile_id.clone());
    let foreign = agents.create_agent(request, &mut sessions).unwrap();
    assert!(registry
        .require_agent_authenticated(
            &owner(&config, &registry, &foreign),
            &foreign,
            "MP-08/MP-10/MP-11 foreign denial"
        )
        .is_err());
    registry
        .update_observation(
            "cloud-owner",
            "opencode",
            &profiles[1].profile_id,
            ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert!(registry
        .require_agent_authenticated(
            &owner(&config, &registry, &agent),
            &agent,
            "MP-08/MP-10/MP-11 expired replica"
        )
        .is_err());
    let alias = agents
        .create_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_owner_user_id("cloud-owner")
                .with_account_profile("default"),
            &mut sessions,
        )
        .unwrap();
    registry
        .require_agent_authenticated(
            &owner(&config, &registry, &alias),
            &alias,
            "MP-08/MP-10/MP-11 alias must stay local",
        )
        .unwrap();
}

fn owner(
    config: &crate::config::DaemonConfig,
    registry: &ProviderAccountProfileRegistry,
    agent: &crate::agent::AgentInstance,
) -> String {
    provider_account_authority_owner_for_profile(
        config,
        registry,
        agent.owner_user_id(),
        "opencode",
        agent.provider_account_profile(),
    )
    .unwrap()
}

// MP-08/MP-10/MP-11: the local account remains selected after its runtime owner enrolls.
#[test]
fn enrolled_runtime_owner_keeps_its_resolved_local_provider_account() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("enrollment-resolved-account");
    let registry = ProviderAccountProfileRegistry::open(root.path().join("profiles.json")).unwrap();
    let profile = registry
        .migrate_effective_defaults("local", &root.path().join("native"))
        .unwrap()
        .into_iter()
        .find(|p| p.provider == "codex")
        .unwrap();
    let mut request = crate::provider::LaunchProviderRequest::new(
        "session",
        "codex",
        "codex",
        "default",
        "gpt-6-luna",
    );
    request.account_profile = profile.profile_id.clone();
    bind_provider_account_authority(&mut request, "local".into(), profile.profile_id.clone());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "enrollment-metadata".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let mut config = crate::config::DaemonConfig::for_tests();
    assert_eq!(
        provider_account_authority_for_run(&config, &registry, &run).unwrap(),
        "local"
    );
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "enrolled-owner".into(),
        ..Default::default()
    });
    run.enroll_local_owner("enrolled-owner").unwrap();
    assert_eq!(run.owner_user_id(), "enrolled-owner");
    assert_eq!(
        provider_account_authority_for_run(&config, &registry, &run).unwrap(),
        "local"
    );
    assert_eq!(provider_account_selection_for_run(&run), profile.profile_id);
    assert!(run.enroll_local_owner("another-owner").is_err());
    assert_eq!(run.owner_user_id(), "enrolled-owner");
    config.cloud_relay.as_mut().unwrap().user_id = "another-owner".into();
    assert!(
        provider_account_authority_for_run(&config, &registry, &run).is_err(),
        "another enrolled person must not inherit the selected local account"
    );
}
