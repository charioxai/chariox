//! MP-08 / MP-10 / MP-11: exact account IDs retain their runtime owner namespace.
use super::*;

#[test]
fn opencode_launch_defers_authentication_to_the_native_harness() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("opencode-native-admission");
    let registry = ProviderAccountProfileRegistry::open(root.path().join("profiles.json")).unwrap();
    let profile = registry
        .create_managed("owner", "opencode", "empty")
        .unwrap();
    for state in [
        ProviderAccountAuthState::Unknown,
        ProviderAccountAuthState::NotConfigured,
        ProviderAccountAuthState::Expired,
        ProviderAccountAuthState::Error,
    ] {
        registry
            .update_observation(
                "owner",
                "opencode",
                &profile.profile_id,
                state,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        for model in [
            "opencode/deepseek-v4.1-flash-free",
            "opencode/glm-5.3-flash",
        ] {
            registry
                .admit_launch(
                    "owner",
                    "opencode",
                    &profile.profile_id,
                    Some(model),
                    "submit prompt",
                )
                .expect("the native harness decides whether the selected model needs sign-in");
        }
        assert!(
            registry
                .require_authenticated(
                    "owner",
                    "opencode",
                    &profile.profile_id,
                    None,
                    "ensure remote provider account"
                )
                .is_err(),
            "launch admission must not verify a copied credential"
        );
        assert_eq!(
            registry
                .get("owner", "opencode", &profile.profile_id)
                .unwrap()
                .auth_state,
            state,
            "launch admission must not invent an authenticated account"
        );
        assert!(registry
            .admit_launch(
                "other-owner",
                "opencode",
                &profile.profile_id,
                None,
                "submit prompt",
            )
            .is_err());
    }
}

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
    registry
        .require_agent_authenticated(
            &owner(&config, &registry, &agent),
            &agent,
            "MP-08/MP-10/MP-11 expired replica",
        )
        .unwrap();
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
