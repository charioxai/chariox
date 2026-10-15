//! MP-08 / MP-10 / MP-11: bounded typed I/O and operation-scoped workspace work.
use super::*;
use std::time::{Duration, Instant};
fn run(
    requirements: Vec<Requirement>,
    marker: &str,
    budget: Duration,
) -> (Vec<MachineObservation>, usize, Duration) {
    let (root, store, _) = environment();
    let project = crate::session::RuntimeProject::new(
        "bounded",
        "owner",
        root.path().to_str().unwrap(),
        "Bounded",
        crate::session::RuntimeProjectKind::Named,
    );
    let env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    draft.project_requirements = requirements;
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
        .start_check(request("bounded"), "owner", target.clone(), || {
            Ok(project.clone())
        })
        .unwrap()
    else {
        panic!("expected new Check")
    };
    let ctx = super::review0034::context(root.path());
    let count = root.path().join("worker-count");
    let reached = root.path().join("reached");
    std::env::set_var("CHARIOX_WORKSPACE_CHECK_COUNT", &count);
    if !marker.is_empty() {
        std::env::set_var(marker, &reached);
    }
    let began = Instant::now();
    let result = store.run_check_until(
        ctx,
        environment,
        operation,
        || Ok(project.clone()),
        began + budget,
    );
    let elapsed = began.elapsed();
    if !marker.is_empty() {
        std::env::remove_var(marker);
    }
    std::env::remove_var("CHARIOX_WORKSPACE_CHECK_COUNT");
    result.unwrap();
    if !marker.is_empty() && marker != "CHARIOX_WORKSPACE_CHECK_STALL_MARKER" {
        assert!(reached.exists(), "typed content reader reached");
    }
    let checks = store.load_checks(project.id()).unwrap();
    assert_eq!(
        checks.operations.last().unwrap().recovery_state,
        EnvironmentRecoveryState::Settled
    );
    assert!(matches!(
        store
            .start_check(request("retry"), "owner", target, || Ok(project.clone()))
            .unwrap(),
        CheckStart::New { .. }
    ));
    let workers = std::fs::read_to_string(count)
        .unwrap_or_default()
        .lines()
        .count();
    (checks.observations, workers, elapsed)
}
#[test]
fn review0440_env_file_content_stall_is_bounded() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let mut r = requirement("setting");
    r.spec = RequirementSpec::Variables {
        name: "PUBLIC_SETTING".into(),
        locator: ProjectEnvironmentLocator::EnvFile {
            path: "settings".into(),
            key: "PUBLIC_SETTING".into(),
        },
    };
    let (observed, _, elapsed) = run(
        vec![r],
        "CHARIOX_TYPED_CHECK_STALL_MARKER",
        Duration::from_millis(500),
    );
    assert!(
        elapsed < Duration::from_millis(1200),
        "content read exceeded budget: {elapsed:?}"
    );
    assert_eq!(observed[0].reason_code, "check_budget_exceeded");
    assert_eq!(observed[0].measured_facts.present, None);
}
#[test]
fn review0440_npm_content_stall_is_bounded() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let mut r = requirement("package");
    if let RequirementSpec::Software {
        identity,
        install_source,
        ..
    } = &mut r.spec
    {
        *identity = "public-package".into();
        *install_source = Some("npm package".into());
    }
    let (observed, _, elapsed) = run(
        vec![r],
        "CHARIOX_TYPED_CHECK_STALL_MARKER",
        Duration::from_millis(700),
    );
    assert!(
        elapsed < Duration::from_millis(1400),
        "package read exceeded budget: {elapsed:?}"
    );
    assert_eq!(observed[0].reason_code, "check_budget_exceeded");
    assert_eq!(observed[0].measured_facts.present, None);
}
#[test]
fn review0440_workspace_is_measured_once_per_check() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let (_, workers, _) = run(
        vec![requirement("one"), requirement("two"), requirement("three")],
        "",
        Duration::from_secs(5),
    );
    assert_eq!(
        workers, 1,
        "one metadata worker per Check, independent of item count"
    );
}
#[test]
fn review0440_project_bindings_skip_unrelated_workspace_preflight() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    std::env::set_var("PUBLIC_SETTING", "public-setting");
    let mut r = requirement("setting");
    r.spec = RequirementSpec::Variables {
        name: "PUBLIC_SETTING".into(),
        locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "PUBLIC_SETTING".into(),
        },
    };
    let (observed, workers, _) = run(
        vec![r],
        "CHARIOX_WORKSPACE_CHECK_STALL_MARKER",
        Duration::from_millis(500),
    );
    std::env::remove_var("PUBLIC_SETTING");
    assert_eq!(observed[0].reason_code, "binding_available");
    assert_eq!(workers, 0);
}
