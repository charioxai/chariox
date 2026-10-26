//! MP-08 / MP-10 / MP-11: a worker's leased profile keeps the lease owner's id, never the local alias.
use super::*;

#[test]
fn leased_profile_of_the_cloud_owner_has_observer_roots_on_its_worker() {
    crate::test_support::isolated_env_test!();
    let root = TestWorktree::new("leased-profile-observer-roots");
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    let harness = crate::local::test_support::LocalRouterTestHarness::with_config(config);
    let runtime_state = harness.runtime_state();
    let registry = runtime_state.provider_account_profile_registry();
    let profile = registry
        .migrate_effective_defaults("cloud-owner", &root.path().join("cloud-owner"))
        .unwrap()
        .into_iter()
        .find(|profile| profile.provider == "opencode")
        .unwrap();
    registry
        .update_observation(
            "cloud-owner",
            "opencode",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let roots = registered_external_provider_profile_roots_for(
        &runtime_state,
        "cloud-owner",
        "opencode",
        &profile.profile_id,
    );

    assert!(
        !roots.is_empty(),
        "the worker must read its leased profile's transcripts"
    );
}
