//! MP-08/MP-10/MP-11: a refused peer cannot poison the next fresh Copy.
use super::*;
use crate::managed_context::owner_managed::*;

#[tokio::test(flavor = "current_thread")]
async fn mp08_mp10_mp11_terminal_owner_attempt_allows_fresh_peer_binding() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-owner-rebinding-{:032x}",
        rand::random::<u128>()
    ));
    create_private_directory(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config_path = root.join("config.toml");
    config.user_config.history.operational.path =
        Some(root.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config = config.with_session_history_root(root.join("sessions"));
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        account_id: "account".into(),
        user_id: "owner".into(),
        realm_id: "realm".into(),
        machine_id: Some(config.host_machine_id.clone()),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-enrollment".into()),
        kernel_public_key_thumbprint: Some(public_key_thumbprint(&config.relay_public_key)),
        ..Default::default()
    });
    let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let selection = OwnerManagedTransfer {
        target: ManagedContextTransferTarget {
            relay_realm_id: "realm".into(),
            machine_id: "target-machine".into(),
            kernel_id: "target-kernel".into(),
            relay_public_key: config.relay_public_key.clone(),
            key_thumbprint: public_key_thumbprint(&config.relay_public_key),
        },
        context_selection: OwnerManagedContextSelection {
            kernel_context: OwnerManagedKernelSelection::Empty,
            development_setup: OwnerManagedDevelopmentSelection::Empty,
        },
    };

    let private = crate::transport::relay_crypto::generate_private_key_base64();
    let public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&private).unwrap();
    let mut corrected = selection.clone();
    corrected.target.relay_public_key = public.clone();
    corrected.target.key_thumbprint = public_key_thumbprint(&public);
    let invalid_root = root.join("invalid-candidate");
    let invalid_store = ManagedContextOutboundOperationStore::open(invalid_root.clone()).unwrap();
    let mut invalid = selection.clone();
    invalid.target.key_thumbprint = "0".repeat(64);
    let error = invalid_store
        .prepare_owner_ticket(&config, &runtime, invalid)
        .expect_err("an invalid peer pin must be refused before persisting an owner binding");
    assert!(error.to_string().contains("target identity is invalid"));
    assert!(!invalid_root.join(".operations").exists());
    invalid_store
        .prepare_owner_ticket(&config, &runtime, selection.clone())
        .expect("a refused malformed peer must not poison the next valid selection");
    for (index, (phase, retryable, replace)) in [
        (
            ManagedContextOutboundOperationPhase::Preparing,
            false,
            false,
        ),
        (
            ManagedContextOutboundOperationPhase::Uploading,
            false,
            false,
        ),
        (
            ManagedContextOutboundOperationPhase::Importing,
            false,
            false,
        ),
        (ManagedContextOutboundOperationPhase::Failed, true, false),
        (ManagedContextOutboundOperationPhase::Failed, false, true),
        (ManagedContextOutboundOperationPhase::Completed, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let store_root = root.join(format!("outbound-{index}"));
        let store = ManagedContextOutboundOperationStore::open(store_root.clone()).unwrap();
        let old = store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap();
        let old_plan = old.context_plan.package_binding();
        let candidate = crate::managed_context::owner_managed::prepare_ticket(
            &config,
            &runtime,
            corrected.clone(),
        )
        .unwrap();
        assert_eq!(
            old_plan.plan_digest,
            candidate.context_plan.package_binding().plan_digest,
            "peer keys change independently of the selected Project plan"
        );
        let (_, permit) = store
            .start(&old_plan.context_id, &old_plan.plan_digest)
            .unwrap();
        assert!(store.update(&old_plan.context_id, |status| {
            status.phase = phase;
            status.retryable = retryable;
        }));
        drop(permit);
        store.finish(&old_plan.context_id);
        let result = store.prepare_owner_ticket(&config, &runtime, corrected.clone());
        if replace {
            let next = result.expect(
                "a settled refused/completed attempt must permit a fresh Copy to the current peer",
            );
            assert_ne!(
                next.context_plan.context_id(),
                old.context_plan.context_id()
            );
            assert_eq!(next.target, corrected.target);
            let next_plan = next.context_plan.package_binding();
            let (_, next_permit) = store
                .start(&next_plan.context_id, &next_plan.plan_digest)
                .unwrap();
            drop(next_permit);
            store.finish(&next_plan.context_id);
            assert_eq!(
                store.get(&old_plan.context_id).unwrap().phase,
                phase,
                "old status/authority must remain available under its own context ID"
            );
            assert_eq!(
                serde_json::from_slice::<PersistedOwnerTicket>(
                    &fs::read(
                        store_root
                            .join(".operations")
                            .join(format!("{}-owner.json", old_plan.context_id))
                    )
                    .unwrap()
                )
                .unwrap()
                .ticket
                .target,
                old.target
            );
            drop(store);
            let reopened = ManagedContextOutboundOperationStore::open(store_root.clone()).unwrap();
            let retry = reopened
                .prepare_owner_ticket(&config, &runtime, corrected.clone())
                .unwrap();
            assert_eq!(
                retry.context_plan.context_id(),
                next.context_plan.context_id()
            );
            assert_eq!(
                serde_json::from_slice::<PersistedOwnerTicket>(
                    &fs::read(
                        store_root
                            .join(".operations")
                            .join(format!("{}-owner.json", old_plan.context_id))
                    )
                    .unwrap()
                )
                .unwrap()
                .ticket
                .target,
                old.target
            );
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("destination changed"),
                "unfinished/retryable attempts must retain their exact peer binding"
            );
            let same = store
                .prepare_owner_ticket(&config, &runtime, selection.clone())
                .unwrap();
            assert_eq!(
                same.context_plan.context_id(),
                old.context_plan.context_id()
            );
        }
    }
    for (index, attempted) in [Some(false), Some(true), None].into_iter().enumerate() {
        let store_root = root.join(format!("missing-status-{index}"));
        let store = ManagedContextOutboundOperationStore::open(store_root.clone()).unwrap();
        let old = store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap();
        let plan = old.context_plan.package_binding();
        let parent = store_root.join(".operations");
        let index = parent.join(format!(
            "owner-{}.json",
            plan.plan_digest.trim_start_matches("sha256:")
        ));
        let mut saved: PersistedOwnerTicket =
            serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
        saved.consumption_attempted = attempted;
        let bytes = serde_json::to_vec(&saved).unwrap();
        crate::config::write_private_file(&index, &bytes).unwrap();
        crate::config::write_private_file(
            &parent.join(format!("{}-owner.json", plan.context_id)),
            &bytes,
        )
        .unwrap();
        assert!(store.get(&plan.context_id).is_none());
        let error = store
            .prepare_owner_ticket(&config, &runtime, corrected.clone())
            .unwrap_err();
        assert!(
            error.to_string().contains("destination changed"),
            "missing status is not proof that an unconsumed, consumed or legacy attempt settled"
        );
        assert_eq!(
            serde_json::from_slice::<PersistedOwnerTicket>(&fs::read(&index).unwrap())
                .unwrap()
                .ticket,
            old
        );
    }
    drop(router);
}
