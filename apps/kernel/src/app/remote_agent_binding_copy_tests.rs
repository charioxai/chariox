//! Recorded managed-context default mappings must survive leased-agent creation.
use super::*;
use crate::account_profile::*;

#[test]
fn review_p2_imported_default_mapping_launches_the_receiving_account() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("copy-default-spawn");
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let source = ProviderAccountProfileRegistry::open(root.path().join("source.json"))
        .unwrap()
        .with_machine_identity("source-machine", "source-kernel");
    let source_profile = source
        .create_managed(owner, "codex", "Source default")
        .unwrap();
    source
        .set_default(owner, "codex", &source_profile.profile_id)
        .unwrap();
    let source_environment = source
        .resolve_environment(owner, "codex", "default")
        .unwrap();
    std::fs::write(
        std::path::Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
        br#"{"tokens":{"refresh_token":"synthetic-copy-login"}}"#,
    )
    .unwrap();
    let materialization = source
        .export_managed_context_materialization(owner, "codex", "default")
        .unwrap();
    let expected = ProviderAccountCopyExpectation::from_materialization(&materialization).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut worker = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
    let receiver = worker.provider_account_profile_registry();
    let target_default = receiver.get(owner, "codex", "default").unwrap();
    assert_ne!(target_default.profile_id, source_profile.profile_id);
    let receipt = receiver
        .materialize_managed_context_replica(
            owner,
            "remapped-copy",
            &"a".repeat(64),
            &materialization,
        )
        .unwrap();
    receiver
        .record_received_account_copy(
            owner,
            &materialization,
            &receipt.profile_id,
            ProviderAccountMaterializationTargetKind::Worker,
        )
        .unwrap();
    let installed = receiver
        .update_observation(
            owner,
            "codex",
            &receipt.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let status = installed
        .materializations
        .iter()
        .find(|status| status.target_ref == config.daemon_id)
        .unwrap()
        .clone();
    source
        .record_confirmed_account_copy(
            owner,
            &expected,
            ProviderAccountMaterializationTargetKind::Worker,
            &config.host_machine_id,
            &config.daemon_id,
            &receipt.profile_id,
            status,
        )
        .unwrap();
    let source_profile = source
        .get(owner, "codex", &source_profile.profile_id)
        .unwrap();
    let metadata = installed_remote_account_metadata(
        &source_profile,
        ProviderAccountMaterializationTargetKind::Worker,
        &config.daemon_id,
        owner,
    )
    .unwrap();
    let account = worker_account_profile_for_spawn("default", Some(&metadata));
    let lease = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            "source-kernel",
            "source-session",
            "source-agent",
            false,
            owner,
        )
        .unwrap();
    let leased = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .create_leased_agent_from_base_directory(
            root.path(),
            &lease.id,
            "codex",
            &account,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let environment = receiver
        .resolve_environment(owner, "codex", &leased.account_profile)
        .expect(
            "spawned lease must select the registered receiving login, not the source default ID",
        );
    assert_eq!(leased.account_profile, receipt.profile_id);
    assert_eq!(
        environment,
        receiver
            .resolve_environment(owner, "codex", &target_default.profile_id)
            .unwrap()
    );
    assert_eq!(
        worker
            .agents()
            .get_agent(&leased.backing_agent_id)
            .unwrap()
            .provider_account_profile(),
        receipt.profile_id
    );
    assert_eq!(
        source_profile.profile_id,
        materialization.profile.profile_id
    );
}
