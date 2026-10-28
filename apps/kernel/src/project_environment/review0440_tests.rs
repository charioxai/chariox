//! MP-08 / MP-10 / MP-11: bounded typed I/O and operation-scoped workspace work.
use super::*;
use std::time::{Duration, Instant};
fn run(
    requirements: Vec<Requirement>,
    marker: &str,
    budget: Duration,
) -> (Vec<MachineObservation>, usize, Duration) {
    run_expect(requirements, marker, budget, true)
}
fn run_expect(
    requirements: Vec<Requirement>,
    marker: &str,
    budget: Duration,
    expect_stall: bool,
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
        *environment,
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
    if expect_stall && !marker.is_empty() && marker != "CHARIOX_WORKSPACE_CHECK_STALL_MARKER" {
        assert!(reached.exists(), "typed content reader reached");
    }
    if !expect_stall {
        assert!(
            !reached.exists(),
            "unneeded capacity syscall must be skipped"
        );
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

// MP-08 / MP-10: capacity is a distinct mounted-filesystem syscall.
#[test]
fn review0940_machine_capacity_stall_settles_and_next_check_is_admitted() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let mut r = requirement("disk");
    r.spec = RequirementSpec::MachineNeeds {
        platforms: vec![],
        minimum_cpus: Some(1),
        minimum_memory_bytes: None,
        minimum_free_disk_bytes: Some(1),
        gpu: None,
    };
    let (observations, _, elapsed) = run(
        vec![r],
        "CHARIOX_MACHINE_CAPACITY_STALL_MARKER",
        Duration::from_millis(700),
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "capacity escaped deadline: {elapsed:?}"
    );
    assert_ne!(observations[0].status, EnvironmentObservationStatus::Ready);
    assert_eq!(observations[0].measured_facts.free_disk_bytes, None);
}
#[test]
fn review0940_cpu_only_machine_needs_skip_capacity_io() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let mut r = requirement("cpu");
    r.spec = RequirementSpec::MachineNeeds {
        platforms: vec![],
        minimum_cpus: Some(1),
        minimum_memory_bytes: None,
        minimum_free_disk_bytes: None,
        gpu: None,
    };
    let (observations, _, elapsed) = run_expect(
        vec![r],
        "CHARIOX_MACHINE_CAPACITY_STALL_MARKER",
        Duration::from_millis(700),
        false,
    );
    assert!(elapsed < Duration::from_millis(1400));
    assert_eq!(observations[0].status, EnvironmentObservationStatus::Ready);
    assert_eq!(observations[0].measured_facts.free_disk_bytes, None);
}
#[test]
fn review0940_installed_packages_named_like_desktop_apps_use_package_inventory() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let (root, store, _) = environment();
    let project = crate::session::RuntimeProject::new(
        "packages",
        "owner",
        root.path().to_str().unwrap(),
        "Packages",
        crate::session::RuntimeProjectKind::Named,
    );
    let env = store.snapshot(&project).unwrap();
    let ctx = super::review0034::context(root.path());
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    for name in ["chatgpt", "notion", "canva"] {
        let dir = root.path().join("node_modules").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            serde_json::json!({"name":name,"version":"1.2.3"}).to_string(),
        )
        .unwrap();
        let mut r = requirement(name);
        r.spec = RequirementSpec::Software {
            identity: name.into(),
            version_constraint: Some(">=1 <2".into()),
            platform: None,
            install_scope: EnvironmentInstallScope::Project,
            install_source: Some("npm package".into()),
            detect_only: true,
        };
        let observation = check_requirement(
            &ctx,
            &env,
            &r,
            &target,
            "check-package",
            &mut Default::default(),
            Instant::now() + Duration::from_secs(3),
        );
        assert_eq!(
            observation.status,
            EnvironmentObservationStatus::Ready,
            "{name}: {}",
            observation.reason_code
        );
        assert_eq!(observation.measured_facts.version.as_deref(), Some("1.2.3"));
    }
}

// MP-08 / MP-10: real NFS outage exposed metadata before capacity dispatch.
#[test]
fn review0940_cpu_only_machine_needs_skip_workspace_preflight() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let mut r = requirement("cpu");
    r.spec = RequirementSpec::MachineNeeds {
        platforms: vec![],
        minimum_cpus: Some(1),
        minimum_memory_bytes: None,
        minimum_free_disk_bytes: None,
        gpu: None,
    };
    let (observations, workers, elapsed) = run_expect(
        vec![r],
        "CHARIOX_WORKSPACE_CHECK_STALL_MARKER",
        Duration::from_millis(700),
        false,
    );
    assert!(elapsed < Duration::from_secs(4));
    assert_eq!(observations[0].status, EnvironmentObservationStatus::Ready);
    assert_eq!(observations[0].measured_facts.free_disk_bytes, None);
    assert_eq!(
        workers, 0,
        "host-only facts must not inspect a mounted workspace"
    );
}
