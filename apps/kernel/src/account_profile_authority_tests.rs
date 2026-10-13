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
        .require_agent_authenticated(&config, &agent, "MP-08/MP-10/MP-11 lease account")
        .unwrap();
    request = request.with_account_profile(profiles[0].profile_id.clone());
    let local = agents.create_agent(request.clone(), &mut sessions).unwrap();
    registry
        .require_agent_authenticated(&config, &local, "MP-08/MP-10/MP-11 home alias")
        .unwrap();
    request = request.with_account_profile(profiles[2].profile_id.clone());
    let foreign = agents.create_agent(request, &mut sessions).unwrap();
    assert!(registry
        .require_agent_authenticated(&config, &foreign, "MP-08/MP-10/MP-11 foreign denial")
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
        .require_agent_authenticated(&config, &agent, "MP-08/MP-10/MP-11 expired replica")
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
        .require_agent_authenticated(&config, &alias, "MP-08/MP-10/MP-11 alias must stay local")
        .unwrap();
}
