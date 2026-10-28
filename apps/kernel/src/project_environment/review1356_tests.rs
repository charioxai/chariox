//! MP-08 / MP-10 / MP-11: frozen-review follow-up regressions, supplementary to real cells.
use super::*;
fn context(root: &std::path::Path) -> EnvironmentCheckContext {
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
fn review1356_partial_comparators_cross_software_check() {
    let (root, _store, env) = environment();
    std::fs::create_dir_all(root.path().join("node_modules/public-tool")).unwrap();
    let ctx = context(root.path());
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    for (version, constraint, expected) in [
        ("20.19.0", ">20", EnvironmentObservationStatus::Conflict),
        ("20.19.0", "<=20", EnvironmentObservationStatus::Ready),
        ("20.19.4", ">20.19", EnvironmentObservationStatus::Conflict),
        ("20.19.4", "<=20.19", EnvironmentObservationStatus::Ready),
    ] {
        std::fs::write(
            root.path().join("node_modules/public-tool/package.json"),
            serde_json::to_vec(&serde_json::json!({"name":"public-tool","version":version}))
                .unwrap(),
        )
        .unwrap();
        let mut r = requirement("public-tool");
        if let RequirementSpec::Software {
            identity,
            version_constraint,
            install_source,
            ..
        } = &mut r.spec
        {
            *identity = "public-tool".into();
            *version_constraint = Some(constraint.into());
            *install_source = Some("npm package".into());
        }
        let observation = check_requirement(
            &ctx,
            &env,
            &r,
            &target,
            "partial",
            &mut Default::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        );
        assert_eq!(
            observation.status, expected,
            "partial comparator must follow the admitted wildcard grammar"
        );
    }
}
#[test]
fn review1356_new_dependency_failure_remains_fresh_through_snapshot_projection() {
    let root = crate::test_support::TestWorktree::new("review1356-dependency");
    let project = crate::session::RuntimeProject::new(
        "dependency",
        "owner",
        root.path().to_str().unwrap(),
        "Dependency",
        crate::session::RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let mut a = requirement("impossible");
    let b = requirement("dependent");
    a.spec=serde_json::from_value(serde_json::json!({"kind":"machine_needs","platforms":[],"minimum_cpus":4294967295u32,"minimum_memory_bytes":null,"minimum_free_disk_bytes":null,"gpu":null})).unwrap();
    let mut b = b;
    b.spec=serde_json::from_value(serde_json::json!({"kind":"machine_needs","platforms":[],"minimum_cpus":1,"minimum_memory_bytes":null,"minimum_free_disk_bytes":null,"gpu":null})).unwrap();
    b.depends_on = vec![a.requirement_id.clone()];
    draft.project_requirements = vec![a, b];
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
    env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let request = crate::local::CheckProjectEnvironmentRequest {
        project_id: project.id().into(),
        operation_id: "fresh-dependency".into(),
        revision_digest: env.content_digest.clone(),
        selected_items: vec![],
        target: target.clone(),
    };
    let CheckStart::New { operation, .. } = store
        .start_check(request, "owner", target.clone(), || Ok(project.clone()))
        .unwrap()
    else {
        panic!("new Check expected")
    };
    store
        .run_check(context(root.path()), env, operation, || Ok(project.clone()))
        .unwrap();
    let mut shown = store.snapshot(&project).unwrap();
    store
        .attach_checks(&mut shown, &target, crate::session::unix_epoch_ms())
        .unwrap();
    let dependent = shown
        .observations
        .iter()
        .find(|o| o.requirement_id == "dependent")
        .unwrap();
    assert_eq!(
        dependent.status,
        EnvironmentObservationStatus::NeedsYourInput
    );
    assert!(
        !dependent.stale,
        "a just-measured dependency failure is fresh, not an invitation to repeat Check"
    );
}

#[test]
fn review1356_named_executable_discovery_settles_and_admits_another_check() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1356-executable");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "executable",
        "owner",
        root.path().to_str().unwrap(),
        "Executable",
        crate::session::RuntimeProjectKind::Named,
    );
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    draft.project_requirements = vec![requirement("node")];
    let env = store
        .save_revision_locked(
            &current,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
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
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let request = |id: &str| crate::local::CheckProjectEnvironmentRequest {
        project_id: project.id().into(),
        operation_id: id.into(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        selected_items: vec![],
    };
    let CheckStart::New { operation, .. } = store
        .start_check(
            request("stalled-executable"),
            "owner",
            target.clone(),
            || Ok(project.clone()),
        )
        .unwrap()
    else {
        panic!("new Check")
    };
    let marker = root.path().join("executable-read");
    std::env::set_var("CHARIOX_NAMED_EXECUTABLE_STALL_MARKER", &marker);
    let began = std::time::Instant::now();
    let result = store.run_check_until(
        context(root.path()),
        env.clone(),
        operation,
        || Ok(project.clone()),
        began + std::time::Duration::from_secs(5),
    );
    std::env::remove_var("CHARIOX_NAMED_EXECUTABLE_STALL_MARKER");
    if result.is_err() {
        store
            .settle_failed_check(project.id(), "stalled-executable", || true)
            .unwrap()
    }
    assert!(
        marker.exists(),
        "the real named executable read was reached"
    );
    let pid = std::fs::read_to_string(&marker)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert!(
        pid > 1 && pid != std::process::id(),
        "discovery belongs to an owned child"
    );
    #[cfg(target_os = "linux")]
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "timed-out discovery worker must be reaped"
    );
    assert!(
        began.elapsed() < std::time::Duration::from_secs(9),
        "discovery/shebang I/O escaped the owned Check deadline"
    );
    assert!(
        matches!(
            store
                .start_check(request("next-executable"), "owner", target.clone(), || Ok(
                    project.clone()
                ))
                .unwrap(),
            CheckStart::New { .. }
        ),
        "settlement must release Check admission"
    );
}

#[test]
fn review1356_non_files_and_excluded_files_never_preflight_workspaces() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let (root, _store, current) = environment();
    let marker = root.path().join("manual-root-read");
    for excluded in [false, true] {
        let mut draft = environment_draft(&current);
        draft.project_requirements = vec![requirement("software")];
        if excluded {
            let folder = &current.folders[0].folder_id;
            draft.project_requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"excluded","title":"Excluded","scope":{"kind":"project"},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"missing.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"exclude","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":false,"legacy_entry":null})).unwrap());
        }
        std::env::set_var("CHARIOX_MANUAL_ROOT_STALL_MARKER", &marker);
        let began = std::time::Instant::now();
        let result = verify_manual_files(&current, &mut draft);
        std::env::remove_var("CHARIOX_MANUAL_ROOT_STALL_MARKER");
        result.unwrap();
        assert!(
            !marker.exists(),
            "metadata-only Preview/Save must not inspect any workspace"
        );
        assert!(began.elapsed() < std::time::Duration::from_millis(500));
    }
}
