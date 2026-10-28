//! MP-08 / MP-10 / MP-11: fail-first security and filesystem deadline regressions.
use super::*;
pub(super) fn context(root: &std::path::Path) -> EnvironmentCheckContext {
    EnvironmentCheckContext {
        config: crate::config::DaemonConfig::for_tests(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.join("apps.db")).unwrap(),
    }
}
#[test]
fn public_aliases_cannot_probe_or_save_protected_environment_sources() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    // This is a public canary, never an actual credential value.
    std::env::set_var("CHARIOX_REVIEW_PUBLIC_CANARY", "review-public-canary");
    let (root, store, env) = environment();
    let ctx = context(root.path());
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    for source in [
        "HOME",
        "CHARIOX_REVIEW_PUBLIC_CANARY",
        "CHARIOX_REVIEW_ABSENT_CANARY",
    ] {
        let mut r = requirement("public-setting");
        r.spec = RequirementSpec::Variables {
            name: "APP_ROOT".into(),
            locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
                name: source.into(),
            },
        };
        let mut baseline = Default::default();
        let observation = super::super::check::check_requirement(
            &ctx,
            &env,
            &r,
            &target,
            "protected",
            &mut baseline,
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        );
        assert_eq!(
            observation.reason_code, "protected_binding",
            "source={source}"
        );
        assert_eq!(observation.measured_facts.present, None);
        assert_eq!(observation.measured_facts.content_digest, None);
        assert!(baseline.is_empty());
        let mut draft = environment_draft(&env);
        draft.project_requirements = vec![r];
        assert!(
            store
                .save_revision_locked(
                    &env,
                    &crate::local::SaveProjectEnvironmentRevisionRequest {
                        project_id: env.local_project_id.clone(),
                        expected_revision: env.revision,
                        expected_content_digest: env.content_digest.clone(),
                        draft,
                        accepted_proposal_ids: vec![],
                        excluded_proposal_ids: vec![],
                    },
                    "owner"
                )
                .is_err(),
            "Save must reject alias for {source}"
        );
    }
    std::env::remove_var("CHARIOX_REVIEW_PUBLIC_CANARY");
}

#[test]
fn workspace_metadata_stall_settles_check_and_releases_admission() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let (root, store, _) = environment();
    let project = crate::session::RuntimeProject::new(
        "metadata",
        "owner",
        root.path().to_str().unwrap(),
        "Metadata",
        crate::session::RuntimeProjectKind::Named,
    );
    let env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    draft.project_requirements = vec![requirement("node")];
    store
        .save_revision_locked(
            &env,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                expected_content_digest: env.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap();
    let env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let request = |id: &str| crate::local::CheckProjectEnvironmentRequest {
        project_id: project.id().into(),
        revision_digest: env.content_digest.clone(),
        operation_id: id.into(),
        selected_items: vec![],
        target: target.clone(),
    };
    let CheckStart::New {
        environment,
        operation,
        ..
    } = store
        .start_check(request("metadata-stall"), "owner", target.clone(), || {
            Ok(project.clone())
        })
        .unwrap()
    else {
        panic!("new Check expected")
    };
    let ctx = context(root.path());
    let marker = root.path().join("metadata-stall-pid");
    std::env::set_var("CHARIOX_WORKSPACE_CHECK_STALL_MARKER", &marker);
    let began = std::time::Instant::now();
    let result = store.run_check_until(
        ctx,
        environment,
        operation,
        || Ok(project.clone()),
        began + std::time::Duration::from_millis(300),
    );
    std::env::remove_var("CHARIOX_WORKSPACE_CHECK_STALL_MARKER");
    result.unwrap();
    assert!(marker.exists(), "metadata preflight was reached");
    assert!(
        began.elapsed() < std::time::Duration::from_millis(1000),
        "workspace metadata exceeded operation budget: {:?}",
        began.elapsed()
    );
    let checks = store.load_checks(project.id()).unwrap();
    assert_eq!(checks.observations[0].reason_code, "check_budget_exceeded");
    assert_eq!(
        checks.operations.last().unwrap().recovery_state,
        EnvironmentRecoveryState::Settled
    );
    assert!(matches!(
        store
            .start_check(request("metadata-next"), "owner", target, || Ok(
                project.clone()
            ))
            .unwrap(),
        CheckStart::New { .. }
    ));
}
