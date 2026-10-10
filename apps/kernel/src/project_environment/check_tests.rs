//! MP-08 / MP-10 / MP-11: Check regressions supplement the real client drills.
use super::check_inventory as inventory;
use super::*;
fn environment() -> (
    crate::test_support::TestWorktree,
    ProjectEnvironmentStore,
    ProjectEnvironment,
) {
    let root = crate::test_support::TestWorktree::new("envp04-check");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "check",
        "owner",
        root.path().to_str().unwrap(),
        "Check",
        crate::session::RuntimeProjectKind::Named,
    );
    let environment = store.snapshot(&project).unwrap();
    (root, store, environment)
}
fn observation(
    env: &ProjectEnvironment,
    id: &str,
    target: &EnvironmentTargetBinding,
    time: u64,
) -> MachineObservation {
    MachineObservation {
        environment_id: env.lineage.environment_id.clone(),
        requirement_id: id.into(),
        target: target.clone(),
        revision_digest: env.content_digest.clone(),
        status: EnvironmentObservationStatus::Ready,
        checked_at_ms: Some(time),
        measured_facts: empty_environment_facts(),
        reason_code: "named_pass".into(),
        safe_summary: "Named probe passed".into(),
        check_operation_id: Some("check".into()),
        stale: false,
    }
}
#[test]
fn envp04_observations_are_stale_for_revision_target_bindings_and_clock() {
    let (_root, store, mut env) = environment();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot-one".into(),
        slice_ref: None,
    };
    let mut checks = super::check_store::EnvironmentChecks::default();
    checks.folder_binding_digest = folder_bindings_digest(&env);
    checks.observations = vec![observation(&env, "file", &target, 1000)];
    store.save_checks(&env.local_project_id, &checks).unwrap();
    store.attach_checks(&mut env, &target, 1001).unwrap();
    assert!(!env.observations[0].stale);
    store.attach_checks(&mut env, &target, 999).unwrap();
    assert!(
        env.observations[0].stale,
        "future timestamps are never current evidence"
    );
    store
        .attach_checks(
            &mut env,
            &target,
            1000 + super::check_store::OBSERVATION_TTL_MS + 1,
        )
        .unwrap();
    assert!(env.observations[0].stale);
    let new_target = EnvironmentTargetBinding {
        target_instance_generation: "boot-two".into(),
        ..target.clone()
    };
    store.attach_checks(&mut env, &new_target, 1001).unwrap();
    assert!(env.observations[0].stale);
    env.content_digest = "another-revision".into();
    store.attach_checks(&mut env, &target, 1001).unwrap();
    assert!(env.observations[0].stale);
    env.content_digest = checks.observations[0].revision_digest.clone();
    env.folders[0].local_workspace_binding.clear();
    store.attach_checks(&mut env, &target, 1001).unwrap();
    assert!(env.observations[0].stale);
}
#[test]
fn envp04_inventory_constraints_and_unknowns_fail_closed() {
    for (v, c, want) in [
        ("22.3.1", ">=22 <23", true),
        ("22.3.1", ">=23", false),
        ("0.3.2", "^0.2.0", false),
        ("0.0.3", "^0.0.2", false),
        ("1.5.0", "~1.4.0", false),
        ("1.4.9", "1.4.*", true),
    ] {
        assert_eq!(inventory::satisfies(v, c), Some(want), "{v} {c}");
    }
    assert_eq!(inventory::satisfies("22.3.1", "latest"), None);
    assert_eq!(
        inventory::version("opaque-shell-check", &[]),
        Err("unsupported_inventory")
    );
}
#[test]
fn envp04_opaque_host_probes_never_execute() {
    let root = crate::test_support::TestWorktree::new("envp04-opaque");
    let marker = root.path().join("should-not-exist");
    let probe = EnvironmentProbe::Executable {
        name: "sh".into(),
        arguments: vec!["-c".into(), format!("touch {}", marker.display())],
        timeout_ms: 100,
    };
    assert_eq!(
        inventory::health(&probe, &None, &[]),
        Err("worker_required")
    );
    assert!(!marker.exists());
}
#[test]
fn envp04_dependencies_cannot_borrow_stale_readiness() {
    let (_root, _store, mut env) = environment();
    let make = |id: &str, deps: Vec<&str>| {
        serde_json::from_value(serde_json::json!({"requirement_id":id,"title":id,"scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":null,"platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":deps,"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
    };
    env.project_requirements = vec![
        make("runtime", vec![]),
        make("service", vec!["runtime"]),
        make("app", vec!["service"]),
    ];
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let mut rows = vec![
        observation(&env, "runtime", &target, 1000),
        observation(&env, "service", &target, 1000),
        observation(&env, "app", &target, 1000),
    ];
    rows[0].stale = true;
    rows[2].check_operation_id = Some("older-operation".into());
    invalidate_check_dependencies(&env, &mut rows, &target, "check");
    assert_eq!(rows[1].status, EnvironmentObservationStatus::NeedsYourInput);
    assert_eq!(rows[2].status, EnvironmentObservationStatus::NeedsYourInput);
    assert!(
        rows[2].stale,
        "known dependency drift invalidates previously checked dependents"
    );
}
