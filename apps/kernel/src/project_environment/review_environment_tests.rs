//! MP-08 / MP-10 / MP-11: focused regressions; real clients remain acceptance.
use super::*;
use crate::session::{RuntimeProject, RuntimeProjectKind};
use std::time::{Duration, Instant};
fn context(root: &std::path::Path) -> check::EnvironmentCheckContext {
    check::EnvironmentCheckContext {
        config: crate::config::DaemonConfig::for_tests(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.join("apps.db")).unwrap(),
    }
}
fn save(
    store: &ProjectEnvironmentStore,
    current: &ProjectEnvironment,
    draft: EnvironmentRevisionDraft,
    accepted: Vec<String>,
) -> ProjectEnvironment {
    store
        .save_revision_locked(
            current,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: current.local_project_id.clone(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
                draft,
                accepted_proposal_ids: accepted,
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap()
        .0
}
#[test]
fn review983_attached_folder_can_be_detected_saved_and_checked_after_first_save() {
    let a = crate::test_support::TestWorktree::new("review983-folder-a");
    let b = crate::test_support::TestWorktree::new("review983-folder-b");
    std::fs::write(
        b.path().join("package.json"),
        r#"{"name":"folder-b","engines":{"node":">=22"}}"#,
    )
    .unwrap();
    let store = ProjectEnvironmentStore::new(a.path());
    let mut project = RuntimeProject::new(
        "project",
        "owner",
        a.path().to_str().unwrap(),
        "Project",
        RuntimeProjectKind::Named,
    );
    let initial = store.snapshot(&project).unwrap();
    let first = save(&store, &initial, environment_draft(&initial), vec![]);
    let archive = store.path(project.id()).with_extension("revision-1.json");
    let bytes = std::fs::read(&archive).unwrap();
    project.replace_workspace_ids(vec![
        a.path().to_string_lossy().into_owned(),
        b.path().to_string_lossy().into_owned(),
    ]);
    let mut current = store.snapshot(&project).unwrap();
    assert_eq!(current.content_digest, first.content_digest);
    let folder = current
        .folders
        .iter()
        .find(|f| f.local_workspace_binding == b.path().to_string_lossy())
        .expect("new live folder must enter the editable projection")
        .clone();
    let detection = detect_environment(&[folder], &current.lineage.environment_id).unwrap();
    current.proposals = detection.proposals;
    let proposal = current.proposals.iter().find(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == "node")).unwrap().clone();
    let second = save(
        &store,
        &current,
        environment_draft(&current),
        vec![proposal.proposal_id],
    );
    assert_eq!(second.revision, 2);
    assert_eq!(
        std::fs::read(archive).unwrap(),
        bytes,
        "the acknowledged archive stays immutable"
    );
    let requirement = second
        .folders
        .iter()
        .flat_map(|f| &f.requirements)
        .find(|r| r.requirement_id == proposal.requirement.requirement_id)
        .unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let observation = check::check_requirement(
        &context(a.path()),
        &second,
        requirement,
        &target,
        "check",
        &mut std::collections::BTreeMap::new(),
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(observation.status, EnvironmentObservationStatus::Ready);
}
#[test]
fn review983_detected_rust_runtime_checks_the_installed_compiler() {
    let root = crate::test_support::TestWorktree::new("review983-rust");
    let compiler = check_inventory::version("rustc", &[])
        .unwrap()
        .expect("builder toolchain installed");
    std::fs::write(
        root.path().join("Cargo.toml"),
        format!("[package]\nname = \"project\"\nrust-version = \"{compiler}\"\n"),
    )
    .unwrap();
    let store = ProjectEnvironmentStore::new(root.path());
    let project = RuntimeProject::new(
        "rust",
        "owner",
        root.path().to_str().unwrap(),
        "Rust",
        RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    env.proposals = detect_environment(&env.folders, &env.lineage.environment_id)
        .unwrap()
        .proposals;
    let proposal = env.proposals.iter().find(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == "rust")).unwrap().clone();
    let saved = save(
        &store,
        &env,
        environment_draft(&env),
        vec![proposal.proposal_id],
    );
    let requirement = saved
        .folders
        .iter()
        .flat_map(|f| &f.requirements)
        .find(|r| r.requirement_id == proposal.requirement.requirement_id)
        .unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let observation = check::check_requirement(
        &context(root.path()),
        &saved,
        requirement,
        &target,
        "check",
        &mut std::collections::BTreeMap::new(),
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(
        observation.status,
        EnvironmentObservationStatus::Ready,
        "{}",
        observation.reason_code
    );
    assert_eq!(
        observation.measured_facts.version.as_deref(),
        Some(compiler.as_str())
    );
}
#[cfg(unix)]
#[test]
fn review983_preview_and_save_do_not_execute_workspace_git() {
    if std::env::var_os("CHARIOX_REVIEW983_FILES_CHILD").is_none() {
        let name = std::thread::current().name().unwrap().to_owned();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name.as_str(), "--exact", "--test-threads=1"])
            .env("CHARIOX_REVIEW983_FILES_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_support::TestWorktree::new("review983-files");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = RuntimeProject::new(
        "files",
        "owner",
        root.path().to_str().unwrap(),
        "Files",
        RuntimeProjectKind::Named,
    );
    let current = store.snapshot(&project).unwrap();
    std::fs::write(root.path().join("README.md"), "Public documentation").unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = root.path().join("git-ran");
    std::fs::write(
        bin.join("git"),
        format!("#!/bin/sh\necho ran > '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut draft = environment_draft(&current);
    let folder = draft.folders[0].folder_id.clone();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"public-file","title":"Public documentation","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"README.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    let _lock = crate::env_lock::lock();
    std::env::set_var("PATH", bin);
    let mut invoked = vec![];
    for preview in [true, false] {
        let _ = std::fs::remove_file(&marker);
        let mut measured = draft.clone();
        if preview {
            manual_files::verify_manual_files(&current, &mut measured).unwrap();
        } else {
            save(&store, &current, measured, vec![]);
        }
        invoked.push(marker.exists());
    }
    assert_eq!(
        invoked,
        vec![false, false],
        "Preview and Save must both reject workspace Git before execution"
    );
}

// MP-08 / MP-10 / MP-11: optional Git metadata must have a bounded sanitized capture.
#[cfg(unix)]
#[test]
fn review983_trusted_git_metadata_is_bounded_and_sanitized() {
    if std::env::var_os("CHARIOX_REVIEW983_SLOW_GIT_CHILD").is_none() {
        let name = std::thread::current().name().unwrap().to_owned();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name.as_str(), "--exact", "--test-threads=1"])
            .env("CHARIOX_REVIEW983_SLOW_GIT_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_support::TestWorktree::new("review983-slow-files");
    let tools = crate::test_support::TestWorktree::new("review983-global-tools");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = RuntimeProject::new(
        "files",
        "owner",
        root.path().to_str().unwrap(),
        "Files",
        RuntimeProjectKind::Named,
    );
    let current = store.snapshot(&project).unwrap();
    std::fs::write(root.path().join("README.md"), "Public documentation").unwrap();
    let marker = tools.path().join("ambient-present");
    std::fs::write(tools.path().join("git"), format!("#!/bin/sh\nif [ -n \"$CHARIOX_PUBLIC_GIT_CANARY\" ]; then echo present > '{}'; fi\n/bin/sleep 2\nexit 1\n", marker.display())).unwrap();
    std::fs::set_permissions(
        tools.path().join("git"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let mut draft = environment_draft(&current);
    let folder = draft.folders[0].folder_id.clone();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"public-file","title":"Public documentation","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"README.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    let _lock = crate::env_lock::lock();
    std::env::set_var("PATH", tools.path());
    std::env::set_var("CHARIOX_PUBLIC_GIT_CANARY", "public-test-value");
    let began = Instant::now();
    let mut measured = draft.clone();
    manual_files::verify_manual_files(&current, &mut measured).unwrap();
    save(&store, &current, draft, vec![]);
    assert!(
        !marker.exists(),
        "optional Git must not inherit ambient inputs"
    );
    assert!(
        began.elapsed() < Duration::from_millis(1800),
        "optional Git must not delay both review operations by four seconds"
    );
}

// MP-08 / MP-10 / MP-11: Detect -> proposal acceptance/Save -> actual named
// interpreter Check, preserving provenance and supported Python specifiers.
#[test]
fn review0634_python_manifest_constraints_reach_save_and_check() {
    let installed = check_inventory::version("python3", &[])
        .unwrap()
        .expect("installed Python 3");
    let parts: Vec<_> = installed.split('.').collect();
    let minor = format!("{}.{}", parts[0], parts[1]);
    for (constraint, expected) in [
        (">=3.10,<4".to_owned(), EnvironmentObservationStatus::Ready),
        (
            ">=3.10,<3.11".to_owned(),
            EnvironmentObservationStatus::Conflict,
        ),
        (format!("=={minor}.*"), EnvironmentObservationStatus::Ready),
        ("==2.7.*".to_owned(), EnvironmentObservationStatus::Conflict),
        (
            "~=3.12".to_owned(),
            EnvironmentObservationStatus::NeedsYourInput,
        ),
        (
            "!=3.12".to_owned(),
            EnvironmentObservationStatus::NeedsYourInput,
        ),
    ] {
        let root = crate::test_support::TestWorktree::new("review0634-python");
        std::fs::write(
            root.path().join("pyproject.toml"),
            format!("[project]\nname='project'\nrequires-python='{constraint}'\n"),
        )
        .unwrap();
        let store = ProjectEnvironmentStore::new(root.path());
        let project = RuntimeProject::new(
            "python",
            "owner",
            root.path().to_str().unwrap(),
            "Python",
            RuntimeProjectKind::Named,
        );
        let mut current = store.snapshot(&project).unwrap();
        current.proposals = detect_environment(&current.folders, &current.lineage.environment_id)
            .unwrap()
            .proposals;
        let proposal=current.proposals.iter().find(|p|matches!(&p.requirement.spec,RequirementSpec::Software{identity,..} if identity=="python")).unwrap().clone();
        let saved = save(
            &store,
            &current,
            environment_draft(&current),
            vec![proposal.proposal_id],
        );
        let requirement = saved
            .folders
            .iter()
            .flat_map(|f| &f.requirements)
            .find(|r| r.requirement_id == proposal.requirement.requirement_id)
            .unwrap();
        let target = EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "boot".into(),
            slice_ref: None,
        };
        let observation = check::check_requirement(
            &context(root.path()),
            &saved,
            requirement,
            &target,
            "check",
            &mut std::collections::BTreeMap::new(),
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(
            observation.status, expected,
            "Python {installed}, constraint {constraint}: {}",
            observation.reason_code
        );
    }
}
#[test]
fn review0634_python_specifiers_keep_unsupported_or_malformed_cases_unproven() {
    for (version, constraint, expected) in [
        ("3.12.8", ">=3.10,<4", Some(true)),
        ("3.9.9", ">=3.10,<4", Some(false)),
        ("4.0.0", ">=3.10,<4", Some(false)),
        ("3.12.8", "==3.12.*", Some(true)),
        ("3.13.0", "==3.12.*", Some(false)),
        ("3.12.8", "==3.12", Some(false)),
        ("3.12.0", "==3.12", Some(true)),
        ("3.12.8", "~=3.12", None),
        ("3.12.8", "!=3.12.8", None),
        ("3.12.8", ">=3.10,,<4", None),
        ("3.12.8", ">=3.10,", None),
    ] {
        assert_eq!(
            check_inventory::satisfies(version, constraint),
            expected,
            "{constraint}"
        );
    }
    assert_eq!(
        check_inventory::constraints_overlap(Some(">=3.10,<4"), Some("==3.12.*")),
        Some(true)
    );
    assert_eq!(
        check_inventory::constraints_overlap(Some(">=3.10,<3.12"), Some("==3.12.*")),
        Some(false)
    );
}
