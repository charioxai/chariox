//! MP-08/MP-10/MP-11: read-only account status follows launch namespace admission.
use super::*;

#[test]
fn amfix_r13_account_read_resolves_owned_replica_without_exposing_foreign_profiles() {
    crate::test_support::isolated_env_test!();
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    let harness = LocalRouterTestHarness::with_config(config);
    let (replica, foreign) = harness.with_app_mut(|app| {
        let profiles = app.provider_account_profile_registry();
        let replica = profiles
            .create_managed("cloud-owner", "codex", "replica-visible")
            .unwrap();
        let foreign = profiles
            .create_managed("other-owner", "codex", "foreign-private")
            .unwrap();
        profiles
            .update_observation(
                "cloud-owner",
                "codex",
                &replica.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        (replica, foreign)
    });
    let mut caller = crate::runtime::command::KernelCaller::default()
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    caller.user_id = Some("cloud-owner".into());
    let read = |reference: &str| {
        harness.dispatch_with_caller(
            LocalDaemonRequest::GetProviderAccountProfile(
                crate::local::api::GetProviderAccountProfileRequest {
                    provider: "codex".into(),
                    account_profile: reference.into(),
                },
            ),
            caller.clone(),
        )
    };
    assert!(
        read(&foreign.profile_id).is_err(),
        "foreign profile must stay private"
    );
    let home = read("default").unwrap();
    let LocalDaemonResponse::ProviderAccountProfile { profile } = home else {
        panic!("expected profile");
    };
    assert_eq!(profile.owner_user_id, crate::session::DEFAULT_LOCAL_USER_ID);
    let listed = harness
        .dispatch_with_caller(
            LocalDaemonRequest::ListProviderAccountProfiles(
                crate::local::api::ListProviderAccountProfilesRequest {
                    provider: Some("codex".into()),
                },
            ),
            caller.clone(),
        )
        .unwrap();
    let LocalDaemonResponse::ProviderAccountProfilesListed { profiles } = listed else {
        panic!("expected inventory");
    };
    assert!(profiles.iter().all(|p| p.profile_id != replica.profile_id));
    let observed = read(&replica.profile_id)
        .expect("owned replica must be readable at the same exact reference accepted for launch");
    let LocalDaemonResponse::ProviderAccountProfile { profile } = observed else {
        panic!("expected profile");
    };
    assert_eq!(profile.owner_user_id, "cloud-owner");
    assert_eq!(profile.label, "replica-visible");
    assert_eq!(
        profile.auth_state,
        crate::account_profile::ProviderAccountAuthState::Authenticated
    );
}
