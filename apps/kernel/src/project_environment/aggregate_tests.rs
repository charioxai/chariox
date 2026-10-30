//! MP-02 / MP-03 / MP-08 / MP-10: P01 regression seams, not live acceptance.
use super::*;
use crate::session::{RuntimeProject, RuntimeProjectKind};

#[test]
fn envp01_two_plain_folders_identity_survives_reopen_and_content_change() {
    let root = crate::test_support::TestWorktree::new("envp01-private-store");
    let mut project = RuntimeProject::new(
        "project",
        "owner",
        "/plain/one",
        "Plain",
        RuntimeProjectKind::Named,
    );
    project.replace_workspace_ids(vec!["/plain/one".into(), "/plain/two".into()]);
    let store = ProjectEnvironmentStore::new(root.path());
    let first = store.snapshot(&project).unwrap();
    assert_eq!(first.folders.len(), 2);
    assert_ne!(first.folders[0].folder_id, first.folders[1].folder_id);
    assert!(first.folders.iter().all(|f| f.optional_git.is_none()));
    assert!(first.observations.is_empty());
    assert_eq!(
        first,
        ProjectEnvironmentStore::new(root.path())
            .snapshot(&project)
            .unwrap()
    );
    project.rename("Renamed".into());
    assert_eq!(first.lineage, store.snapshot(&project).unwrap().lineage);
    project.replace_workspace_ids(vec![
        "/plain/two".into(),
        "/plain/one".into(),
        "/plain/three".into(),
    ]);
    let changed = store.snapshot(&project).unwrap();
    assert_eq!(first.folders[0].folder_id, changed.folders[1].folder_id);
    assert_eq!(first.folders[1].folder_id, changed.folders[0].folder_id);
    store.remove(project.id()).unwrap();
    assert!(!root
        .path()
        .join("project-environments")
        .read_dir()
        .unwrap()
        .any(|p| p.unwrap().path().extension().is_some_and(|e| e == "json")));
}

#[test]
fn envp01_legacy_adapter_is_lossless_idempotent_and_never_promotes_status() {
    let root = crate::test_support::TestWorktree::new("envp01-legacy-store");
    let mut project = RuntimeProject::new(
        "legacy",
        "owner",
        "/plain/one",
        "Legacy",
        RuntimeProjectKind::Named,
    );
    let definition: crate::session::ProjectEnvironmentDefinition = serde_json::from_value(serde_json::json!({
        "schema_version":1,"origin":"user_authored","source":"commands","target_platform":"linux-x86_64","source_path":null,
        "setup_steps":[{"kind":"command","command":"echo sensitive-value"}],"validation_commands":["true"]
    })).unwrap();
    project.set_environment_definition(definition.clone());
    let evidence = ProjectEnvironmentEvidence::default();
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: project.id().into(),
        evidence_digest: evidence.digest(),
        entries: vec![ProjectEnvironmentEntry {
            name: "DB_PASSWORD".into(),
            workspace_id: "/plain/one".into(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification: ProjectEnvironmentClassification::Secret,
            excluded: true,
            uses: vec![ProjectEnvironmentUse {
                path: "src/db.rs".into(),
                line: 42,
            }],
            locator: ProjectEnvironmentLocator::EnvFile {
                path: ".env.local".into(),
                key: "DB_PASSWORD".into(),
            },
            status: ProjectEnvironmentEntryStatus::Found,
        }],
        private_files: vec![ProjectPrivateFileDecision {
            workspace_id: "/plain/one".into(),
            path: "brief.pdf".into(),
            bring: false,
            reason: "User excluded asset".into(),
            secret_looking: false,
        }],
        toolchain_hints: vec!["node".into(), "node".into()],
        package_hints: vec!["font".into()],
        service_hints: vec!["postgres".into()],
    };
    let state = StoredProjectEnvironment {
        apply_owned_bindings: Default::default(),
        source: None,
        manifest: manifest.clone(),
        evidence,
        reported_missing: Default::default(),
        reviewed_manifest: Some(manifest.clone()),
        last_review: None,
    };
    let store = ProjectEnvironmentStore::new(root.path());
    store.save(&state).unwrap();
    let original_bytes: Vec<_> = root
        .path()
        .join("project-environments")
        .read_dir()
        .unwrap()
        .filter_map(|p| {
            let p = p.unwrap().path();
            (p.extension().is_some_and(|e| e == "json")).then(|| std::fs::read(p).unwrap())
        })
        .collect();
    let first = store.snapshot(&project).unwrap();
    let second = store.snapshot(&project).unwrap();
    assert_eq!(first, second);
    assert_eq!(store.load(project.id()).unwrap(), Some(state.clone()));
    assert!(root
        .path()
        .join("project-environments")
        .read_dir()
        .unwrap()
        .any(|p| std::fs::read(p.unwrap().path())
            .ok()
            .is_some_and(|b| original_bytes.contains(&b))));
    assert_eq!(first.legacy_manifest, Some(manifest.clone()));
    assert_eq!(first.legacy_reviewed_manifest, Some(manifest));
    assert_eq!(first.project_requirements.len(), 1);
    assert_eq!(first.folders[0].requirements.len(), 2);
    assert_eq!(first.proposals.len(), 3);
    let requirement = &first.folders[0].requirements[0];
    assert!(!requirement.required);
    assert!(matches!(
        requirement.spec,
        RequirementSpec::Secrets { vault: None, .. }
    ));
    assert!(first.observations.is_empty());
    let encoded = serde_json::to_string(&first).unwrap();
    assert!(!encoded.contains("sensitive-value"));
    // MP-08/MP-10: readiness is a target observation, never specification identity.
    let mut observed = state.clone();
    for status in [
        ProjectEnvironmentEntryStatus::Missing,
        ProjectEnvironmentEntryStatus::Problem,
    ] {
        observed.manifest.entries[0].status = status;
        store.save(&observed).unwrap();
        let snapshot = store.snapshot(&project).unwrap();
        assert_eq!(snapshot.content_digest, first.content_digest);
        assert_eq!(
            snapshot.folders[0].requirements[0]
                .legacy_entry
                .as_ref()
                .unwrap()
                .status,
            status
        );
        assert_eq!(
            snapshot.legacy_manifest.as_ref().unwrap().entries[0].status,
            status
        );
    }
    observed.manifest.entries[0].excluded = false;
    store.save(&observed).unwrap();
    assert_ne!(
        store.snapshot(&project).unwrap().content_digest,
        first.content_digest
    );
    assert_eq!(project.environment_definition(), Some(&definition));
}

#[test]
fn envp01_value_fields_and_unknown_account_providers_fail_closed() {
    for value in [
        serde_json::json!({"kind":"secrets","name":"PASSWORD","vault":{"service":"db","key":"password"},"value":"secret"}),
        serde_json::json!({"kind":"variables","name":"PUBLIC","locator":{"kind":"missing"},"value":"plain"}),
        serde_json::json!({"kind":"accounts","provider":"notion","linked_profile_ref":null,"required_capabilities":[]}),
        serde_json::json!({"kind":"files","folder_id":"folder","entries":[{"relative_path":"brief","kind":"file","content_digest":null,"selected":true,"contents":"secret"}]}),
    ] {
        assert!(serde_json::from_value::<RequirementSpec>(value).is_err());
    }
    let files: RequirementSpec = serde_json::from_value(serde_json::json!({"kind":"files","folder_id":"folder","entries":[{"relative_path":"brief","kind":"folder","content_digest":null,"folder_id":"folder","user_selected":true,"git_ignored":true,"byte_count":123,"credential_filter_verdict":"not_checked","transfer_inclusion":"review_required","reason":null,"secret_looking":false}]})).unwrap();
    assert!(matches!(files, RequirementSpec::Files { .. }));
}

#[test]
fn envp01_accounts_accept_only_canonical_official_provider_ids() {
    for provider in ["codex", "claude", "opencode"] {
        let spec: RequirementSpec = serde_json::from_value(serde_json::json!({"kind":"accounts","provider":provider,"linked_profile_ref":"linked","required_capabilities":[]})).unwrap();
        assert_eq!(serde_json::to_value(spec).unwrap()["provider"], provider);
    }
}

#[test]
fn envp01_read_and_identity_extension_succeed_during_export_or_adjustment() {
    let root = crate::test_support::TestWorktree::new("envp01-read-during-adjustment");
    let mut project = RuntimeProject::new(
        "project",
        "owner",
        "/plain/one",
        "Plain",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let adjustment = store.try_lock(project.id()).unwrap();
    let first = store
        .snapshot(&project)
        .expect("read must not contend with export or interactive adjustment");
    project.replace_workspace_ids(vec!["/plain/one".into(), "/plain/two".into()]);
    let extended = store
        .snapshot(&project)
        .expect("folder identity extension must use a separate lock");
    assert_eq!(first.lineage, extended.lineage);
    assert_eq!(first.folders[0].folder_id, extended.folders[0].folder_id);
    assert!(
        store.try_lock(project.id()).is_err(),
        "read must not release the adjustment's lock"
    );
    drop(adjustment);
    assert!(
        store.try_lock(project.id()).is_ok(),
        "read must not retain an adjustment admission lock"
    );
}

#[test]
fn envp01_identity_serialization_never_blocks_adjustment_admission() {
    let root = crate::test_support::TestWorktree::new("envp01-identity-admission");
    let project = RuntimeProject::new(
        "project",
        "owner",
        "/plain/one",
        "Plain",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let first = store.snapshot(&project).unwrap();
    let identity_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store.path(project.id()).with_extension("identity.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&identity_file).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reader_store = store.clone();
    let reader_project = project.clone();
    let reader = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = reader_store.snapshot(&reader_project);
        done_tx.send(()).unwrap();
        result
    });
    started_rx.recv().unwrap();
    let completed_while_identity_locked = done_rx
        .recv_timeout(std::time::Duration::from_millis(50))
        .is_ok();
    let adjustment = store.try_lock(project.id());
    fs2::FileExt::unlock(&identity_file).unwrap();
    let second = reader.join().unwrap().unwrap();
    assert!(
        !completed_while_identity_locked,
        "the read must serialize on identity anchors"
    );
    assert!(
        adjustment.is_ok(),
        "a serialized read must leave adjustment admission available"
    );
    assert_eq!(first.lineage, second.lineage);
}

// MP-08 / MP-10 / MP-11: REVIEW_954 P2 detached saved Files survive unrelated Preview/Save.
#[test]
fn review954_detached_unchanged_files_remain_reviewable() {
    let root = crate::test_support::TestWorktree::new("review954-detached-files");
    let mut project = RuntimeProject::new(
        "files",
        "owner",
        root.path().to_str().unwrap(),
        "Files",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    std::fs::write(root.path().join("public.md"), "public instructions").unwrap();
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let folder_id = draft.folders[0].folder_id.clone();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public file","scope":{"kind":"folder","folder_id":folder_id},"origins":[],"spec":{"kind":"files","folder_id":folder_id,"entries":[{"relative_path":"public.md","kind":"file","content_digest":null,"folder_id":folder_id,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    draft.project_requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"node","title":"Node","scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":null,"platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    verify_manual_files(&current, &mut draft).unwrap();
    let request = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: current.revision,
        expected_content_digest: current.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store
        .save_revision_locked(&current, &request, "owner")
        .unwrap();
    project.replace_workspace_ids(vec![root.path().join("another").to_string_lossy().into()]);
    let detached = store.snapshot(&project).unwrap();
    assert!(detached.folders[0].local_workspace_binding.is_empty());
    let mut draft = environment_draft(&detached);
    draft.project_requirements[0].title = "Reviewed Node".into();
    environment_revision_diff(&detached, &draft).unwrap();
    verify_manual_files(&detached, &mut draft)
        .expect("unchanged detached Files need no filesystem reads for Preview");
    let request = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: detached.revision,
        expected_content_digest: detached.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store
        .save_revision_locked(&detached, &request, "owner")
        .expect("unrelated Save preserves detached Files");
    let mut changed = request.draft.clone();
    if let RequirementSpec::Files { entries, .. } = &mut changed.folders[0].requirements[0].spec {
        entries[0].relative_path = "new.md".into();
    }
    assert!(
        verify_manual_files(&detached, &mut changed).is_err(),
        "changed selection still needs a live folder"
    );
}

// MP-08 / MP-10: P04 uses the preallocated Check contract without an agent.
#[test]
fn envp04_independent_check_is_delivered_for_unsaved_legacy() {
    let root = crate::test_support::TestWorktree::new("envp04-check-delivery");
    let project = RuntimeProject::new(
        "p04",
        "owner",
        root.path().to_str().unwrap(),
        "Check",
        RuntimeProjectKind::Named,
    );
    let env = ProjectEnvironmentStore::new(root.path())
        .snapshot(&project)
        .unwrap();
    assert!(env
        .delivered_capabilities
        .enabled_environment_operations
        .contains(&EnvironmentCapability::Check));
}

#[path = "revision_disjoint_platform_tests.rs"]
mod revision_disjoint_platform;
#[path = "revision_excluded_files_tests.rs"]
mod revision_excluded_files;
#[path = "revision_filename_privacy_tests.rs"]
mod revision_filename_privacy;

// MP-08 / MP-10 / MP-11: first Apply includes its recovery controls.
#[test]
fn envp05a_first_apply_advertises_durable_controls_together() {
    let root = crate::test_support::TestWorktree::new("envp05-capabilities");
    let project = RuntimeProject::new(
        "project",
        "owner",
        "/plain/one",
        "Apply",
        RuntimeProjectKind::Named,
    );
    let env = ProjectEnvironmentStore::new(root.path())
        .snapshot(&project)
        .unwrap();
    for capability in [
        EnvironmentCapability::Plan,
        EnvironmentCapability::Apply,
        EnvironmentCapability::GetOperation,
        EnvironmentCapability::CancelOperation,
        EnvironmentCapability::RetryOperation,
    ] {
        assert!(
            env.delivered_capabilities
                .enabled_environment_operations
                .contains(&capability),
            "missing {capability:?}"
        );
    }
}
#[path = "revision_platform_compatibility_tests.rs"]
mod revision_platform_compatibility;
#[path = "revision_setup_source_privacy_tests.rs"]
mod revision_setup_source_privacy;

// MP-08 / MP-10 / MP-11: repeat edits cannot exhaust a cumulative snapshot file.
#[test]
fn revision_index_keeps_only_current_snapshot_and_preserves_immutable_archives() {
    let root = crate::test_support::TestWorktree::new("revision-archives");
    let project = RuntimeProject::new(
        "archives",
        "owner",
        root.path().to_str().unwrap(),
        "Archives",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let mut current = store.snapshot(&project).unwrap();
    let mut first_archive = None;
    for _ in 0..12 {
        let request = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft: environment_draft(&current),
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        current = store
            .save_revision_locked(&current, &request, "owner")
            .unwrap()
            .0;
        let bytes =
            std::fs::read(store.path(project.id()).with_extension("revision-0.json")).unwrap();
        if let Some(first) = &first_archive {
            assert_eq!(&bytes, first);
        } else {
            first_archive = Some(bytes);
        }
    }
    let index: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.path(project.id()).with_extension("revisions.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(index["revisions"].as_array().unwrap().len(), 1);
    assert!(store
        .path(project.id())
        .with_extension("revision-0.json")
        .exists());
    assert!(store
        .path(project.id())
        .with_extension("revision-12.json")
        .exists());
    assert_eq!(store.snapshot(&project).unwrap().revision, 12);
    store.remove(project.id()).unwrap();
    assert!(!store
        .path(project.id())
        .with_extension("revision-0.json")
        .exists());
}

// MP-08 / MP-10 / MP-11: a provider Detect holder cannot consume an unbounded Check wait.
#[test]
fn check_project_and_identity_locks_share_the_operation_deadline() {
    let root = crate::test_support::TestWorktree::new("check-lock-budget");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = RuntimeProject::new(
        "locks",
        "owner",
        root.path().to_string_lossy(),
        "Locks",
        crate::session::RuntimeProjectKind::Named,
    );
    for extension in ["lock", "identity.lock"] {
        let held = if extension == "lock" {
            store.lock(project.id()).unwrap()
        } else {
            store.identity_lock(project.id()).unwrap()
        };
        let began = std::time::Instant::now();
        assert!(store
            .lock_extension_until(
                project.id(),
                extension,
                began + std::time::Duration::from_millis(25)
            )
            .is_err());
        assert!(began.elapsed() < std::time::Duration::from_millis(250));
        drop(held);
        assert!(store
            .lock_extension_until(project.id(), extension, std::time::Instant::now())
            .is_ok());
    }
}
