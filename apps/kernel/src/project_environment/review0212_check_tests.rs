//! MP-08 / MP-10 / MP-11: actual runner and contained inventory regressions, supplementary only.
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
fn operation(
    env: &ProjectEnvironment,
    target: &EnvironmentTargetBinding,
    selected: Vec<&str>,
) -> EnvironmentOperation {
    EnvironmentOperation {
        operation_id: "review0212-check".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        kind: EnvironmentOperationKind::Check,
        phase: EnvironmentOperationPhase::Requested,
        selected_items: selected.into_iter().map(str::to_owned).collect(),
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: crate::session::unix_epoch_ms(),
        updated_at_ms: crate::session::unix_epoch_ms(),
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Pending,
    }
}
fn project(root: &std::path::Path, id: &str) -> crate::session::RuntimeProject {
    crate::session::RuntimeProject::new(
        id,
        "owner",
        root.to_str().unwrap(),
        "Review",
        crate::session::RuntimeProjectKind::Named,
    )
}
#[test]
fn review0212_partial_check_expires_dependency_journal_evidence() {
    let root = crate::test_support::TestWorktree::new("review0212-expiry");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "expiry");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let mut a = requirement("a");
    if let RequirementSpec::Software { identity, .. } = &mut a.spec {
        *identity = "opaque-runtime".into();
    }
    let mut b = requirement("b");
    b.depends_on = vec!["a".into()];
    draft.project_requirements = vec![a, b];
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: env.revision,
        expected_content_digest: env.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store.save_revision_locked(&env, &save, "owner").unwrap();
    env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec!["b"]);
    let mut journal = EnvironmentChecks::default();
    journal.folder_binding_digest = folder_bindings_digest(&env);
    journal.observations = vec![observation(
        &env,
        "a",
        &target,
        crate::session::unix_epoch_ms() - super::super::check_store::OBSERVATION_TTL_MS - 1,
    )];
    journal.operations.push(op.clone());
    store.save_checks(project.id(), &journal).unwrap();
    store
        .run_check(context(root.path()), env, op, || Ok(project.clone()))
        .unwrap();
    let journal = store.load_checks(project.id()).unwrap();
    assert!(
        journal
            .observations
            .iter()
            .find(|o| o.requirement_id == "a")
            .is_some_and(|a| a.stale || a.status != EnvironmentObservationStatus::Ready),
        "expired prerequisite must not certify a fresh dependent"
    );
    assert_eq!(
        journal
            .observations
            .iter()
            .find(|o| o.requirement_id == "b")
            .unwrap()
            .status,
        EnvironmentObservationStatus::NeedsYourInput
    );
}
#[cfg(unix)]
#[test]
fn review0212_pnpm_contained_package_links_and_escaping_links() {
    use std::os::unix::fs::symlink;
    let root = crate::test_support::TestWorktree::new("review0212-pnpm");
    for name in ["react", "@scope/tool"] {
        let folder = name.replace('/', "+");
        let package = root.path().join(format!(
            "node_modules/.pnpm/{folder}@1.2.3/node_modules/{name}"
        ));
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            serde_json::to_vec(&serde_json::json!({"name":name,"version":"1.2.3"})).unwrap(),
        )
        .unwrap();
        let link = root.path().join(format!("node_modules/{name}"));
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        let prefix = if name.starts_with('@') { "../" } else { "" };
        symlink(
            format!("{prefix}.pnpm/{folder}@1.2.3/node_modules/{name}"),
            &link,
        )
        .unwrap();
        assert_eq!(
            inventory::software_version(name, Some("npm package"), Some(root.path()), &[]),
            Ok(Some("1.2.3".into())),
            "normal installed pnpm dependency must be proven"
        );
        std::fs::remove_file(&link).unwrap();
        let outside = crate::test_support::TestWorktree::new("review0212-pnpm-outside");
        std::fs::write(
            outside.path().join("package.json"),
            serde_json::to_vec(&serde_json::json!({"name":name,"version":"1.2.3"})).unwrap(),
        )
        .unwrap();
        symlink(outside.path(), &link).unwrap();
        assert!(
            inventory::software_version(name, Some("npm package"), Some(root.path()), &[]).is_err(),
            "escaping package links remain unproven"
        );
    }
}
#[cfg(unix)]
#[test]
fn review0212_files_check_does_not_launch_path_git_or_inherit_probe_environment() {
    // MP-08 / MP-10 / MP-11: PATH is process-wide; other parallel tests
    // must never execute this test's sentinel Git or lose their native tools.
    if std::env::var_os("CHARIOX_REVIEW0212_FILES_CHILD").is_none() {
        let name = std::thread::current().name().unwrap().to_owned();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name.as_str(), "--exact", "--test-threads=1"])
            .env("CHARIOX_REVIEW0212_FILES_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated Files Check sentinel failed");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_support::TestWorktree::new("review0212-git");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "git");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let folder = draft.folders[0].folder_id.clone();
    std::fs::write(
        root.path().join("README.md"),
        "public project documentation",
    )
    .unwrap();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"README.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: env.revision,
        expected_content_digest: env.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store.save_revision_locked(&env, &save, "owner").unwrap();
    env = store.snapshot(&project).unwrap();
    let ctx = context(root.path());
    let _guard = crate::env_lock::lock();
    let marker = root.path().join("git-pid");
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(
        bin.join("git"),
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o700)).unwrap();
    struct PathGuard(Option<std::ffi::OsString>);
    impl Drop for PathGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
    let _path_guard = PathGuard(std::env::var_os("PATH"));
    std::env::set_var("PATH", &bin);
    let marker_copy = marker.clone();
    let watchdog = std::thread::spawn(move || {
        for _ in 0..200 {
            if marker_copy.exists() {
                let pid = std::fs::read_to_string(&marker_copy)
                    .unwrap()
                    .parse::<i32>()
                    .unwrap();
                assert!(pid > 1);
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec!["file"]);
    let mut journal = EnvironmentChecks::default();
    journal.operations.push(op.clone());
    store.save_checks(project.id(), &journal).unwrap();
    let began = std::time::Instant::now();
    let result = store.run_check(ctx, env, op, || Ok(project.clone()));
    let elapsed = began.elapsed();
    watchdog.join().unwrap();
    result.unwrap();
    assert!(
        !marker.exists(),
        "Check must not call Save-only PATH git metadata"
    );
    assert!(elapsed < std::time::Duration::from_secs(1));
}

#[test]
fn r4_save_rejects_requirement_and_service_cycles_before_check() {
    for mixed in [false, true] {
        let root = crate::test_support::TestWorktree::new("r4-dependency-cycle");
        let store = ProjectEnvironmentStore::new(root.path());
        let project = project(root.path(), "cycles");
        let current = store.snapshot(&project).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut a = requirement("a");
        a.spec = RequirementSpec::Services {
            identity: "service-a".into(),
            target_binding: None,
            probe: Some(EnvironmentProbe::Tcp {
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
                timeout_ms: 1000,
            }),
            health_expectation: None,
            depends_on: vec![],
        };
        let mut b = a.clone();
        b.requirement_id = "b".into();
        if let RequirementSpec::Services { identity, .. } = &mut b.spec {
            *identity = "service-b".into()
        }
        a.depends_on = vec!["b".into()];
        if mixed {
            if let RequirementSpec::Services { depends_on, .. } = &mut b.spec {
                *depends_on = vec!["a".into()]
            }
        } else {
            b.depends_on = vec!["a".into()]
        }
        let mut draft = environment_draft(&current);
        draft.project_requirements = vec![a, b];
        let mut save = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        let error = store
            .save_revision_locked(&current, &save, "owner")
            .expect_err("Save must reject the dependency cycle before Check can publish Ready");
        assert!(error.to_string().contains("dependency cycle"));
        assert_eq!(store.snapshot(&project).unwrap().revision, 0);
        save.draft.project_requirements[1].depends_on.clear();
        if let RequirementSpec::Services { depends_on, .. } =
            &mut save.draft.project_requirements[1].spec
        {
            depends_on.clear()
        }
        let env = store
            .save_revision_locked(&current, &save, "owner")
            .unwrap()
            .0;
        let target = EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "boot".into(),
            slice_ref: None,
        };
        let op = operation(&env, &target, vec![]);
        let mut journal = EnvironmentChecks::default();
        journal.operations.push(op.clone());
        store.save_checks(project.id(), &journal).unwrap();
        store
            .run_check(context(root.path()), env, op, || Ok(project.clone()))
            .unwrap();
        let checked = store.load_checks(project.id()).unwrap();
        assert_eq!(checked.observations.len(), 2);
        assert!(checked
            .observations
            .iter()
            .all(|o| o.status == EnvironmentObservationStatus::Ready && !o.stale));
    }
}
