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
