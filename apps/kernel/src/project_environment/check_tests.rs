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
fn requirement(id: &str) -> Requirement {
    serde_json::from_value(serde_json::json!({"requirement_id":id,"title":id,"scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":null,"platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
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
    env.project_requirements.push(requirement("file"));
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot-one".into(),
        slice_ref: None,
    };
    let mut checks = super::check_store::EnvironmentChecks {
        folder_binding_digest: folder_bindings_digest(&env),
        ..Default::default()
    };
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
    invalidate_check_dependencies(&env, &mut rows, &target, Some("check"));
    assert_eq!(rows[1].status, EnvironmentObservationStatus::NeedsYourInput);
    assert_eq!(rows[2].status, EnvironmentObservationStatus::NeedsYourInput);
    assert!(
        rows[2].stale,
        "known dependency drift invalidates previously checked dependents"
    );
}

// MP-08 / MP-10 / MP-11: current reads stay small; older operation IDs retain their outcome.
#[test]
fn envp04_current_projection_omits_removed_items_and_historical_result_bodies() {
    let (_root, store, mut env) = environment();
    env.project_requirements.push(requirement("file"));
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let operation = |id: &str| EnvironmentOperation {
        operation_id: id.into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        kind: EnvironmentOperationKind::Check,
        phase: EnvironmentOperationPhase::Ready,
        selected_items: vec![],
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: 1000,
        updated_at_ms: 1000,
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Settled,
    };
    let checks = super::check_store::EnvironmentChecks {
        folder_binding_digest: folder_bindings_digest(&env),
        observations: vec![
            observation(&env, "file", &target, 1000),
            observation(&env, "removed", &target, 1000),
        ],
        operations: vec![operation("older"), operation("current")],
        ..Default::default()
    };
    store.save_checks(&env.local_project_id, &checks).unwrap();
    store.attach_checks(&mut env, &target, 1001).unwrap();
    assert_eq!(env.observations.len(), 1);
    assert_eq!(env.observations[0].requirement_id, "file");
    assert_eq!(env.operations.len(), 1);
    assert_eq!(env.operations[0].operation_id, "current");
    assert_eq!(
        store
            .check_operation(&env.local_project_id, "older", &target)
            .unwrap()
            .unwrap()
            .phase,
        EnvironmentOperationPhase::Ready
    );
    assert!(store
        .check_operation(&env.local_project_id, "unknown", &target)
        .unwrap()
        .is_none());
}

// MP-08 / MP-10 / MP-11: rebooted processes must not grow current-view payloads forever.
#[test]
fn envp04_new_check_replaces_obsolete_generation_but_preserves_other_targets() {
    let (_root, store, mut env) = environment();
    env.project_requirements.push(requirement("file"));
    env.project_requirements.push(requirement("unselected"));
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot-two".into(),
        slice_ref: None,
    };
    let old = EnvironmentTargetBinding {
        target_instance_generation: "boot-one".into(),
        ..target.clone()
    };
    let other_machine = EnvironmentTargetBinding {
        machine_id: "other-machine".into(),
        ..old.clone()
    };
    let other_slice = EnvironmentTargetBinding {
        slice_ref: Some("other-slice".into()),
        ..old.clone()
    };
    let mut checks = super::check_store::EnvironmentChecks {
        folder_binding_digest: folder_bindings_digest(&env),
        ..Default::default()
    };
    for (id, binding) in [
        ("file", &old),
        ("file", &other_machine),
        ("file", &other_slice),
        ("unselected", &old),
    ] {
        checks.record_observation(observation(&env, id, binding, 1000));
    }
    store.save_checks(&env.local_project_id, &checks).unwrap();
    store.attach_checks(&mut env, &target, 1001).unwrap();
    assert_eq!(env.observations.len(), 4);
    assert!(env.observations.iter().all(|o| o.stale));
    checks.record_observation(observation(&env, "file", &target, 1001));
    assert_eq!(
        checks.observations.len(),
        4,
        "new probe replaces this machine/slice's old process observation"
    );
    assert!(!checks
        .observations
        .iter()
        .any(|o| o.requirement_id == "file" && o.target == old));
    assert!(checks
        .observations
        .iter()
        .any(|o| o.target == other_machine));
    assert!(checks.observations.iter().any(|o| o.target == other_slice));
    assert!(checks
        .observations
        .iter()
        .any(|o| o.requirement_id == "unselected" && o.target == old));
    store.save_checks(&env.local_project_id, &checks).unwrap();
    store.attach_checks(&mut env, &target, 1002).unwrap();
    assert_eq!(env.observations.iter().filter(|o| !o.stale).count(), 1);
    assert_eq!(
        env.observations.iter().find(|o| !o.stale).unwrap().target,
        target
    );
}

#[path = "check_measurement_tests.rs"]
mod review0212;

#[path = "check_probe_validation_tests.rs"]
mod review0821;

#[path = "check_provider_safety_tests.rs"]
mod review1118;

// MP-08 / MP-10 / MP-11: a later partial Check cannot extend prerequisite evidence.
#[test]
fn review1301_partial_check_dependency_expires_on_snapshot() {
    let (_root, store, mut env) = environment();
    let mut dependent = requirement("dependent");
    dependent.depends_on = vec!["base".into()];
    let mut transitive = requirement("transitive");
    transitive.depends_on = vec!["dependent".into()];
    env.project_requirements = vec![requirement("base"), dependent, transitive];
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let mut checks = super::check_store::EnvironmentChecks {
        folder_binding_digest: folder_bindings_digest(&env),
        ..Default::default()
    };
    checks.observations = vec![
        observation(&env, "base", &target, 1000),
        observation(&env, "dependent", &target, 241000),
        observation(&env, "transitive", &target, 241000),
    ];
    store.save_checks(&env.local_project_id, &checks).unwrap();
    store.attach_checks(&mut env, &target, 241001).unwrap();
    assert!(env
        .observations
        .iter()
        .all(|o| !o.stale && o.status == EnvironmentObservationStatus::Ready));
    store.attach_checks(&mut env, &target, 301001).unwrap();
    assert!(
        env.observations.iter().all(|o| o.stale),
        "dependent and transitive evidence must expire with their prerequisite"
    );
}

#[test]
fn check_project_lock_wait_respects_the_absolute_deadline() {
    let (_root, store, env) = environment();
    let _detect = store.lock(&env.local_project_id).unwrap();
    let started = std::time::Instant::now();
    let result = store.lock_until(
        &env.local_project_id,
        started + std::time::Duration::from_millis(30),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_millis(160));
}

#[test]
#[cfg(unix)]
fn named_host_probe_keeps_its_symlink_invocation_name() {
    let root = crate::test_support::TestWorktree::new("named-shim");
    let alias = root.path().join("named-tool");
    std::os::unix::fs::symlink("/bin/sh", &alias).unwrap();
    let output = super::check_executable::command(&alias, &[])
        .unwrap()
        .args(["-c", "printf '%s' \"$0\""])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        alias.to_string_lossy()
    );
}

#[path = "review0634_dependency_tests.rs"]
mod review0634_dependencies;

#[test]
fn r4_multicall_fixture() {
    if std::env::var_os("CHARIOX_MULTICALL_FIXTURE").is_some() {
        let invocation = std::env::args_os().next().unwrap();
        assert_eq!(
            std::path::Path::new(&invocation).file_name().unwrap(),
            "cargo"
        );
        println!("cargo fixture invocation preserved");
    }
}

#[cfg(unix)]
#[test]
fn r4_inventory_preserves_symlinked_multicall_invocation() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("r4-multicall-inventory");
    let invocation = root.path().join("cargo");
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &invocation).unwrap();
    std::env::set_var("PATH", root.path());
    let selected = inventory::executable("cargo").unwrap();
    let target_parent = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(
        super::check_executable::command(&invocation, &[target_parent]).is_err(),
        "canonical target trust checks must still reject a project executable"
    );
    for path in [&selected, &invocation] {
        let mut command = super::check_executable::command(path, &[]).unwrap();
        command.env("CHARIOX_MULTICALL_FIXTURE", "1").args([
            "--exact",
            "project_environment::check_tests::r4_multicall_fixture",
            "--nocapture",
        ]);
        let output = crate::local::provider_requests::capture_inventory_probe(
            command,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "multicall fixture must receive cargo argv[0]: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("cargo fixture invocation preserved")
        );
    }
}

// MP-08 / MP-10 / MP-11: full Check publication cannot exhaust retained journal bodies.
#[test]
fn r6_environment_repeated_full_checks_keep_history_within_byte_capacity() {
    let (root, store, env) = environment();
    let project = crate::session::RuntimeProject::new(
        "check",
        "owner",
        root.path().to_str().unwrap(),
        "Check",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut draft = environment_draft(&env);
    draft.project_requirements = (0..4096)
        .map(|index| {
            let mut r = requirement(&format!("public-setting-{index:04}"));
            r.spec = RequirementSpec::Secrets {
                name: format!("SETTING_{index}"),
                vault: None,
            };
            r
        })
        .collect();
    let env = store
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
        .unwrap()
        .0;
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "generation".into(),
        slice_ref: None,
    };
    let context = EnvironmentCheckContext {
        config: crate::config::DaemonConfig::for_tests(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.path().join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.path().join("apps.db"))
            .unwrap(),
    };
    let mut seed = super::check_store::EnvironmentChecks {
        folder_binding_digest: folder_bindings_digest(&env),
        ..Default::default()
    };
    // Establish identical current evidence so progress exercises bounded batch publication.
    for r in &env.project_requirements {
        seed.observations.push(check_requirement(
            &context,
            &env,
            r,
            &target,
            "seed",
            &mut seed.public_bindings,
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        ));
    }
    store.save_checks(project.id(), &seed).unwrap();
    for index in 0..12 {
        let request = crate::local::CheckProjectEnvironmentRequest {
            project_id: project.id().into(),
            revision_digest: env.content_digest.clone(),
            target: target.clone(),
            operation_id: format!("full-{index}"),
            selected_items: env
                .project_requirements
                .iter()
                .map(|r| r.requirement_id.clone())
                .collect(),
        };
        let super::check_start::CheckStart::New { mut operation, .. } = store
            .start_check(request, "owner", target.clone(), || Ok(project.clone()))
            .expect("full Check admission must remain available")
        else {
            panic!("new Check expected")
        };
        let mut progress = store.load_checks(project.id()).unwrap();
        operation.phase = EnvironmentOperationPhase::Running;
        operation.per_item_results = progress
            .observations
            .iter_mut()
            .map(|observation| {
                observation.check_operation_id = Some(operation.operation_id.clone());
                EnvironmentItemResult {
                    requirement_id: observation.requirement_id.clone(),
                    status: observation.status.clone(),
                    reason_code: observation.reason_code.clone(),
                    safe_summary: observation.safe_summary.clone(),
                    receipt_ids: vec![],
                }
            })
            .collect();
        *progress.operations.last_mut().unwrap() = operation.clone();
        store
            .save_checks(project.id(), &progress)
            .expect("full Check progress must fit the journal");
        operation.phase = EnvironmentOperationPhase::Ready;
        operation.recovery_state = EnvironmentRecoveryState::Settled;
        *progress.operations.last_mut().unwrap() = operation;
        store
            .save_checks(project.id(), &progress)
            .expect("full Check settlement must fit the journal");
        let journal = store.load_checks(project.id()).unwrap();
        assert!(serde_json::to_vec(&journal).unwrap().len() <= 8 * 1024 * 1024);
        assert_eq!(journal.observations.len(), 4096);
        assert_eq!(
            journal.operations.last().unwrap().per_item_results.len(),
            4096
        );
        assert_eq!(journal.operations.len(), index + 1);
        for prior in 0..=index {
            let retained = store
                .check_operation(project.id(), &format!("full-{prior}"), &target)
                .unwrap()
                .expect("retained operation identity must remain queryable");
            assert_eq!(retained.phase, EnvironmentOperationPhase::Ready);
            assert_eq!(retained.selected_items.len(), 4096);
        }
    }
    assert!(store
        .load_checks(project.id())
        .unwrap()
        .operations
        .iter()
        .any(|o| o.per_item_results.is_empty()));
    let request = crate::local::CheckProjectEnvironmentRequest {
        project_id: project.id().into(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        operation_id: "after-full-history".into(),
        selected_items: vec![env.project_requirements[0].requirement_id.clone()],
    };
    let super::check_start::CheckStart::New {
        environment,
        operation,
        ..
    } = store
        .start_check(request, "owner", target, || Ok(project.clone()))
        .unwrap()
    else {
        panic!("new Check expected")
    };
    store
        .run_check(context, *environment, operation, || Ok(project.clone()))
        .expect("ordinary Check must remain usable after repeated full result publication");
}

#[cfg(unix)]
#[test]
fn r6_environment_inventory_does_not_promote_prerelease_probe_output() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_support::TestWorktree::new("r6-prerelease-probe");
    let executable = root.path().join("rustc");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf 'rustc 1.93.0-nightly (abc123 2026-10-28)\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("PATH", root.path());
    let version = inventory::version("rustc", &[]);
    assert!(
        version
            .as_ref()
            .ok()
            .and_then(|v| v.as_deref())
            .is_none_or(|v| inventory::satisfies(v, ">=1.93.0") != Some(true)),
        "prerelease output cannot establish stable readiness: {version:?}"
    );
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf 'rustc 1.93.0 (abc123 2026-10-28)\\n'\n",
    )
    .unwrap();
    assert_eq!(
        inventory::version("rustc", &[]).unwrap().as_deref(),
        Some("1.93.0")
    );
    for (name, output) in [
        ("go", "go version go1.93.0 linux/amd64"),
        ("java", "openjdk version \"1.93.0\""),
    ] {
        let executable = root.path().join(name);
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            inventory::version(name, &[]).unwrap().as_deref(),
            Some("1.93.0")
        );
    }
}
