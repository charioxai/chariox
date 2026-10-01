use super::*;

#[test]
fn mp08_mp10_skipped_inputs_do_not_block_available_bindings_or_transfer() {
    let fixture = Fixture::new();
    let manifest = fixture_manifest(vec![entry("MISSING", ProjectEnvironmentLocator::Missing)]);
    let resolved = resolve_project_environment(
        &manifest,
        &fixture.workspaces(),
        &BTreeMap::new(),
        &TestVault::default(),
    )
    .unwrap();
    assert!(
        project_environment_launch_bindings(&manifest, &resolved, "web")
            .unwrap()
            .is_empty()
    );
    materialize_project_environment(&manifest, &resolved, &fixture.workspaces()).unwrap();
    use base64::Engine;
    let source_key = base64::engine::general_purpose::STANDARD.encode([41_u8; 32]);
    let target_key = base64::engine::general_purpose::STANDARD.encode([42_u8; 32]);
    let source_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&source_key).unwrap();
    let target_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&target_key).unwrap();
    let sealed = seal_project_environment(
        "skipped",
        "source",
        "target",
        &source_key,
        &target_public,
        &manifest,
        &resolved,
    )
    .unwrap();
    let target_vault = TestVault::default();
    let imported = unseal_project_environment(
        &sealed,
        &sealed.binding,
        &target_key,
        &source_public,
        &target_vault,
    )
    .unwrap();
    assert_eq!(imported.unresolved.len(), 1);
    assert!(target_vault.0.lock().unwrap().is_empty());
    let target = target_project_environment_manifest(
        &sealed.manifest,
        &BTreeMap::from([("web".into(), "target-web".into())]),
        &target_vault,
    )
    .unwrap();
    assert_eq!(
        target.entries[0].status,
        ProjectEnvironmentEntryStatus::Missing
    );
    assert_eq!(target.entries[0].workspace_id, "target-web");
}

#[test]
fn mp08_discovery_defaults_to_secret_and_rejects_literal_values() {
    let entry: ProjectEnvironmentEntry = serde_json::from_value(serde_json::json!({
        "name": "DATABASE_URL", "workspace_id": "workspace-1", "kind": "variable",
        "uses": [{"path": "src/db.ts", "line": 3}],
        "locator": {"kind": "env_file", "path": ".env.local", "key": "DATABASE_URL"}
    }))
    .unwrap();
    assert_eq!(
        entry.classification,
        ProjectEnvironmentClassification::Secret
    );
    let mut json = serde_json::to_value(&entry).unwrap();
    json["value"] = serde_json::json!("must never reach discovery");
    assert!(serde_json::from_value::<ProjectEnvironmentEntry>(json).is_err());
}

#[test]
fn mp08_env_parser_never_executes_shell_or_expands_values() {
    assert_eq!(
        parse_environment_assignment("export A='literal $B'", "A").unwrap(),
        Some("literal $B".into())
    );
    assert!(parse_environment_assignment("A=$(cat /etc/passwd)", "A").is_err());
    assert!(parse_environment_assignment("A=$OTHER", "A").is_err());
    assert_eq!(
        parse_environment_assignment("UNRELATED=hidden", "A").unwrap(),
        None
    );
}

use crate::error::DaemonError;
use crate::secret::CredentialVaultStore;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct TestVault(Mutex<BTreeMap<(String, String), String>>);
impl std::fmt::Debug for TestVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestVault(redacted)")
    }
}
impl CredentialVaultStore for TestVault {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        self.0
            .lock()
            .unwrap()
            .get(&(service.into(), key.into()))
            .cloned()
            .ok_or_else(|| resolver::environment_error("missing fixture value"))
    }
    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        self.0
            .lock()
            .unwrap()
            .insert((service.into(), key.into()), value.into());
        Ok(())
    }
    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        self.0.lock().unwrap().remove(&(service.into(), key.into()));
        Ok(())
    }
}

fn entry(name: &str, locator: ProjectEnvironmentLocator) -> ProjectEnvironmentEntry {
    ProjectEnvironmentEntry {
        name: name.into(),
        workspace_id: "web".into(),
        kind: ProjectEnvironmentEntryKind::Variable,
        classification: ProjectEnvironmentClassification::Secret,
        excluded: false,
        uses: vec![ProjectEnvironmentUse {
            path: "src/db.ts".into(),
            line: 3,
        }],
        locator,
        status: ProjectEnvironmentEntryStatus::Missing,
    }
}
fn fixture_manifest(entries: Vec<ProjectEnvironmentEntry>) -> ProjectEnvironmentManifest {
    ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: "fixture-project".into(),
        evidence_digest: "a".repeat(64),
        entries,
        private_files: Vec::new(),
        toolchain_hints: vec!["node".into()],
        package_hints: vec!["libpq-dev".into()],
        service_hints: vec!["postgres".into()],
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("chariox-envlayer2-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn workspaces(&self) -> BTreeMap<String, PathBuf> {
        BTreeMap::from([("web".into(), self.0.clone())])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn mp08_resolver_captures_only_referenced_names_and_refreshes_values() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join(".env.local"),
        "DATABASE_URL='postgres://fixture/one'\nUNRELATED=not-captured\n",
    )
    .unwrap();
    let vault = TestVault::default();
    let manifest = fixture_manifest(vec![entry(
        "DATABASE_URL",
        ProjectEnvironmentLocator::EnvFile {
            path: ".env.local".into(),
            key: "DATABASE_URL".into(),
        },
    )]);
    let first =
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap();
    assert!(first.unresolved.is_empty());
    assert_eq!(first.values.len(), 1);
    assert!(!format!("{first:?}").contains("postgres://"));
    assert!(!serde_json::to_string(&manifest)
        .unwrap()
        .contains("postgres://"));
    std::fs::write(
        fixture.0.join(".env.local"),
        "DATABASE_URL='postgres://fixture/two'\n",
    )
    .unwrap();
    let second =
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap();
    assert_eq!(
        second.values.values().next().unwrap().as_str(),
        "postgres://fixture/two"
    );
    assert_eq!(vault.0.lock().unwrap().len(), 1);
}

#[test]
fn mp08_missing_input_reuses_project_scoped_vault_value() {
    let fixture = Fixture::new();
    let vault = TestVault::default();
    let missing = entry("PAYMENT_TOKEN", ProjectEnvironmentLocator::Missing);
    let manifest = fixture_manifest(vec![missing.clone()]);
    assert_eq!(
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap()
            .unresolved
            .len(),
        1
    );
    vault
        .set_secret(
            &project_environment_vault_service(&manifest.project_id),
            &project_environment_vault_key(&missing),
            "synthetic-input",
        )
        .unwrap();
    let resolved =
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap();
    assert!(resolved.unresolved.is_empty());
    assert_eq!(resolved.values.len(), 1);
}

#[cfg(unix)]
#[test]
fn mp08_resolver_rejects_symlink_fifo_and_ambiguous_assignments() {
    let fixture = Fixture::new();
    let manifest = fixture_manifest(vec![entry(
        "TOKEN",
        ProjectEnvironmentLocator::EnvFile {
            path: ".env".into(),
            key: "TOKEN".into(),
        },
    )]);
    let vault = TestVault::default();
    std::os::unix::fs::symlink("/etc/passwd", fixture.0.join(".env")).unwrap();
    assert_eq!(
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap()
            .unresolved
            .len(),
        1
    );
    std::fs::remove_file(fixture.0.join(".env")).unwrap();
    std::fs::write(fixture.0.join(".env"), "TOKEN=first\nTOKEN=second").unwrap();
    assert_eq!(
        resolve_project_environment(&manifest, &fixture.workspaces(), &BTreeMap::new(), &vault)
            .unwrap()
            .unresolved
            .len(),
        1
    );
    assert!(vault.0.lock().unwrap().is_empty());
    assert!(resolver::open_workspace_file(&fixture.0, "../escape").is_err());
}

#[test]
fn mp08_evidence_refresh_is_incremental_and_kernel_copies_are_independent() {
    let source = Fixture::new();
    let target = Fixture::new();
    std::fs::write(source.0.join("package-lock.json"), "initial-lock").unwrap();
    let paths = BTreeMap::from([("web".into(), vec!["package-lock.json".into()])]);
    let original = ProjectEnvironmentEvidence::capture(&source.workspaces(), &paths).unwrap();
    let mut manifest = fixture_manifest(vec![]);
    manifest.evidence_digest = original.digest();
    let state = StoredProjectEnvironment {
        manifest,
        evidence: original.clone(),
        reported_missing: Default::default(),
        reviewed_manifest: None,
        last_review: None,
    };
    let source_store = ProjectEnvironmentStore::new(&source.0.join("private"));
    let target_store = ProjectEnvironmentStore::new(&target.0.join("private"));
    source_store.save(&state).unwrap();
    target_store.save(&state).unwrap();
    assert!(original.changed_paths(&state.evidence).is_empty());
    std::fs::write(target.0.join("package-lock.json"), "changed-lock").unwrap();
    let changed = ProjectEnvironmentEvidence::capture(&target.workspaces(), &paths).unwrap();
    assert_eq!(changed.changed_paths(&original), paths);
    let mut target_state = state.clone();
    target_state.manifest.evidence_digest = changed.digest();
    target_state.evidence = changed;
    target_store.save(&target_state).unwrap();
    assert_eq!(source_store.load("fixture-project").unwrap(), Some(state));
    assert_eq!(
        target_store.load("fixture-project").unwrap(),
        Some(target_state)
    );
    let env_paths = BTreeMap::from([("web".into(), vec![".env.local".into()])]);
    assert!(ProjectEnvironmentEvidence::capture(&source.workspaces(), &env_paths).is_err());
}

#[test]
fn mp08_selected_layer_is_target_sealed_and_rejects_tampering() {
    let fixture = Fixture::new();
    let source_vault = TestVault::default();
    let target_vault = TestVault::default();
    let referenced = entry("TOKEN", ProjectEnvironmentLocator::Missing);
    let manifest = fixture_manifest(vec![referenced.clone()]);
    source_vault
        .set_secret(
            &project_environment_vault_service(&manifest.project_id),
            &project_environment_vault_key(&referenced),
            "synthetic-sealed-value",
        )
        .unwrap();
    let resolved = resolve_project_environment(
        &manifest,
        &fixture.workspaces(),
        &BTreeMap::new(),
        &source_vault,
    )
    .unwrap();
    // Ephemeral relay identities stay in memory; this does not use release signing keys.
    use base64::Engine;
    let source_key = base64::engine::general_purpose::STANDARD.encode([31_u8; 32]);
    let target_key = base64::engine::general_purpose::STANDARD.encode([32_u8; 32]);
    let wrong_key = base64::engine::general_purpose::STANDARD.encode([33_u8; 32]);
    let source_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&source_key).unwrap();
    let target_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&target_key).unwrap();
    let sealed = seal_project_environment(
        "fixture-transfer",
        "source",
        "target",
        &source_key,
        &target_public,
        &manifest,
        &resolved,
    )
    .unwrap();
    assert!(!serde_json::to_string(&sealed)
        .unwrap()
        .contains("synthetic-sealed-value"));
    assert!(unseal_project_environment(
        &sealed,
        &sealed.binding,
        &wrong_key,
        &source_public,
        &target_vault
    )
    .is_err());
    let mut tampered = sealed.clone();
    tampered.manifest.entries[0].classification = ProjectEnvironmentClassification::NonSecret;
    assert!(unseal_project_environment(
        &tampered,
        &sealed.binding,
        &target_key,
        &source_public,
        &target_vault
    )
    .is_err());
    assert!(target_vault.0.lock().unwrap().is_empty());
    let imported = unseal_project_environment(
        &sealed,
        &sealed.binding,
        &target_key,
        &source_public,
        &target_vault,
    )
    .unwrap();
    assert!(imported.unresolved.is_empty());
    assert_eq!(imported.values.len(), 1);
}

#[cfg(unix)]
#[test]
fn mp08_materialization_roundtrips_literals_and_omits_unreferenced_keys() {
    use std::os::unix::fs::PermissionsExt;
    let source = Fixture::new();
    let target = Fixture::new();
    let vault = TestVault::default();
    std::fs::write(
        source.0.join(".env.local"),
        "TOKEN='literal $dollar \"quote\" `backtick`'\nUNRELATED=omit\n",
    )
    .unwrap();
    let manifest = fixture_manifest(vec![entry(
        "TOKEN",
        ProjectEnvironmentLocator::EnvFile {
            path: ".env.local".into(),
            key: "TOKEN".into(),
        },
    )]);
    let resolved =
        resolve_project_environment(&manifest, &source.workspaces(), &BTreeMap::new(), &vault)
            .unwrap();
    materialize_project_environment(&manifest, &resolved, &target.workspaces()).unwrap();
    let path = target.0.join(".env.local");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!std::fs::read_to_string(path).unwrap().contains("UNRELATED"));
    let target_values =
        resolve_project_environment(&manifest, &target.workspaces(), &BTreeMap::new(), &vault)
            .unwrap();
    assert!(target_values.unresolved.is_empty());
    assert_eq!(target_values.values, resolved.values);
    let bindings = project_environment_launch_bindings(&manifest, &target_values, "web").unwrap();
    assert_eq!(bindings.len(), 1);
}

#[test]
fn mp08_export_reuses_discovery_but_resolves_current_values_and_new_target_entries() {
    use zeroize::Zeroizing;
    let source = Fixture::new();
    let target = Fixture::new();
    let second = Fixture::new();
    let vault = TestVault::default();
    let target_vault = TestVault::default();
    let source_store = ProjectEnvironmentStore::new(&source.0.join("private"));
    let target_store = ProjectEnvironmentStore::new(&target.0.join("private"));
    std::fs::write(
        source.0.join(".env"),
        "DATABASE_URL='postgres://fixture/initial'\n",
    )
    .unwrap();
    std::fs::write(source.0.join("package-lock.json"), "initial-lock").unwrap();
    let paths = BTreeMap::from([("web".into(), vec!["package-lock.json".into()])]);
    let evidence = ProjectEnvironmentEvidence::capture(&source.workspaces(), &paths).unwrap();
    let shell_environment = BTreeMap::from([(
        "web".into(),
        BTreeMap::from([(
            "SHELL_SECRET".into(),
            Zeroizing::new("synthetic-shell-input".into()),
        )]),
    )]);
    let prepared = prepare_project_environment_export(
        &source_store,
        "fixture-project",
        evidence.clone(),
        &source.workspaces(),
        &shell_environment,
        &vault,
        |request| {
            assert!(request.previous_manifest.is_none());
            assert_eq!(request.changed_paths, paths);
            let mut manifest = fixture_manifest(vec![
                entry(
                    "DATABASE_URL",
                    ProjectEnvironmentLocator::EnvFile {
                        path: ".env".into(),
                        key: "DATABASE_URL".into(),
                    },
                ),
                entry(
                    "SHELL_SECRET",
                    ProjectEnvironmentLocator::WorkspaceEnvironment {
                        name: "SHELL_SECRET".into(),
                    },
                ),
                entry("MISSING_TOKEN", ProjectEnvironmentLocator::Missing),
            ]);
            manifest.evidence_digest = request.evidence_digest;
            Ok(manifest)
        },
    )
    .unwrap();
    assert!(prepared.discovery_ran);
    assert_eq!(prepared.newly_missing.len(), 1);
    let mut state = prepared.state;
    supply_project_environment_inputs(
        &mut state,
        BTreeMap::from([(
            ("web".into(), "MISSING_TOKEN".into()),
            Zeroizing::new("synthetic-user-input".into()),
        )]),
        &vault,
    )
    .unwrap();
    source_store.save(&state).unwrap();
    let source_ready = prepare_project_environment_export(
        &source_store,
        "fixture-project",
        evidence.clone(),
        &source.workspaces(),
        &shell_environment,
        &vault,
        |_| panic!("unchanged evidence must not run discovery"),
    )
    .unwrap();
    assert!(!source_ready.discovery_ran);
    assert!(source_ready.resolved.unresolved.is_empty());
    let saved_source = source_store.load("fixture-project").unwrap().unwrap();
    // This core regression uses target staging directories. It is not the MP-10 Docker/provider acceptance drill.
    materialize_project_environment(
        &source_ready.state.manifest,
        &source_ready.resolved,
        &target.workspaces(),
    )
    .unwrap();
    // Simulate the authenticated M28 delivery using the production target-sealed layer.
    use base64::Engine;
    let source_key = base64::engine::general_purpose::STANDARD.encode([41_u8; 32]);
    let target_key = base64::engine::general_purpose::STANDARD.encode([42_u8; 32]);
    let source_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&source_key).unwrap();
    let target_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&target_key).unwrap();
    let layer = seal_project_environment(
        "chain-transfer",
        "source",
        "target",
        &source_key,
        &target_public,
        &saved_source.manifest,
        &source_ready.resolved,
    )
    .unwrap();
    unseal_project_environment(
        &layer,
        &layer.binding,
        &target_key,
        &source_public,
        &target_vault,
    )
    .unwrap();
    let target_manifest = target_project_environment_manifest(
        &saved_source.manifest,
        &BTreeMap::from([("web".into(), "web".into())]),
        &target_vault,
    )
    .unwrap();
    target_store
        .save(&StoredProjectEnvironment {
            manifest: target_manifest,
            evidence: evidence.clone(),
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
    std::fs::write(
        target.0.join(".env"),
        "DATABASE_URL='postgres://fixture/changed-on-target'\nNEW_TOKEN='synthetic-new-value'\n",
    )
    .unwrap();
    std::fs::write(target.0.join("package-lock.json"), "new-lock").unwrap();
    let target_evidence =
        ProjectEnvironmentEvidence::capture(&target.workspaces(), &paths).unwrap();
    let target_ready = prepare_project_environment_export(
        &target_store,
        "fixture-project",
        target_evidence,
        &target.workspaces(),
        &BTreeMap::new(),
        &target_vault,
        |request| {
            assert_eq!(request.changed_paths, paths);
            let mut manifest = request.previous_manifest.unwrap();
            manifest.evidence_digest = request.evidence_digest;
            manifest.entries.push(entry(
                "NEW_TOKEN",
                ProjectEnvironmentLocator::EnvFile {
                    path: ".env".into(),
                    key: "NEW_TOKEN".into(),
                },
            ));
            Ok(manifest)
        },
    )
    .unwrap();
    assert!(target_ready.discovery_ran);
    assert!(target_ready.resolved.unresolved.is_empty());
    assert_eq!(
        target_ready.resolved.values[&("web".into(), "DATABASE_URL".into())].as_str(),
        "postgres://fixture/changed-on-target"
    );
    materialize_project_environment(
        &target_ready.state.manifest,
        &target_ready.resolved,
        &second.workspaces(),
    )
    .unwrap();
    let second_values = resolve_project_environment(
        &target_ready.state.manifest,
        &second.workspaces(),
        &BTreeMap::new(),
        &target_vault,
    )
    .unwrap();
    assert!(second_values.unresolved.is_empty());
    assert_eq!(second_values.values.len(), 4);
    assert_eq!(
        source_store.load("fixture-project").unwrap().unwrap(),
        saved_source
    );
    assert!(std::fs::read_to_string(source.0.join(".env"))
        .unwrap()
        .contains("initial"));
}

#[test]
fn mp08_discovery_cannot_invent_value_locators_or_omit_references() {
    let manifest = fixture_manifest(vec![entry("TOKEN", ProjectEnvironmentLocator::Missing)]);
    let input = ProjectEnvironmentDiscoveryInput {
        revision: None,
        project_id: manifest.project_id.clone(),
        evidence_digest: manifest.evidence_digest.clone(),
        changed_paths: BTreeMap::new(),
        private_files: Vec::new(),
        previous_manifest: None,
        references: manifest.entries.clone(),
    };
    assert_eq!(
        parse_project_environment_discovery_output(
            &serde_json::to_string(&manifest).unwrap(),
            &input
        )
        .unwrap(),
        manifest
    );
    let mut invented = manifest.clone();
    invented.entries[0].locator = ProjectEnvironmentLocator::ConfigFile {
        path: "credential-profile.json".into(),
    };
    assert!(parse_project_environment_discovery_output(
        &serde_json::to_string(&invented).unwrap(),
        &input
    )
    .is_err());
    let mut omitted = manifest.clone();
    omitted.entries.clear();
    assert!(parse_project_environment_discovery_output(
        &serde_json::to_string(&omitted).unwrap(),
        &input
    )
    .is_err());
    let mut secret = serde_json::to_value(&manifest).unwrap();
    secret["entries"][0]["value"] = serde_json::json!("synthetic-do-not-echo");
    let error =
        parse_project_environment_discovery_output(&secret.to_string(), &input).unwrap_err();
    assert!(!error.to_string().contains("synthetic-do-not-echo"));
}

#[test]
fn mp08_project_cleanup_removes_only_its_captured_values() {
    let fixture = Fixture::new();
    let vault = TestVault::default();
    let store = ProjectEnvironmentStore::new(&fixture.0.join("private"));
    let evidence = ProjectEnvironmentEvidence::default();
    let mut manifest = fixture_manifest(vec![entry("TOKEN", ProjectEnvironmentLocator::Missing)]);
    manifest.evidence_digest = evidence.digest();
    vault
        .set_secret("external", "unrelated", "synthetic-unrelated")
        .unwrap();
    vault
        .set_secret(
            &project_environment_vault_service(&manifest.project_id),
            &project_environment_vault_key(&manifest.entries[0]),
            "synthetic-captured",
        )
        .unwrap();
    store
        .save(&StoredProjectEnvironment {
            manifest,
            evidence,
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
    remove_project_environment(&store, "fixture-project", &vault).unwrap();
    assert!(store.load("fixture-project").unwrap().is_none());
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    assert_eq!(
        vault.get_secret("external", "unrelated").unwrap(),
        "synthetic-unrelated"
    );
}

#[test]
fn mp08_index_discovers_new_deleted_references_without_values() {
    let fixture = Fixture::new();
    let root = fixture.0.clone();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/app.ts"), "const db = process.env.DATABASE_URL;\nconst s = process.env.SHELL_ONLY;\nconst m = process.env.MISSING;\n").unwrap();
    std::fs::write(
        root.join(".env"),
        "DATABASE_URL=never-in-metadata\nUNREFERENCED=never-capture\n",
    )
    .unwrap();
    std::fs::write(root.join(".env.local"), "DATABASE_URL=local-precedence\n").unwrap();
    let roots = BTreeMap::from([("workspace".into(), root.clone())]);
    let index = index_project_environment(
        &roots,
        &BTreeMap::from([("workspace".into(), BTreeSet::from(["SHELL_ONLY".into()]))]),
    )
    .unwrap();
    let json = serde_json::to_string(&index.references).unwrap();
    assert!(
        !json.contains("never-in-metadata")
            && !json.contains("local-precedence")
            && !json.contains("UNREFERENCED")
    );
    assert_eq!(index.references.len(), 3);
    assert!(
        matches!(&index.references.iter().find(|e| e.name == "DATABASE_URL").unwrap().locator, ProjectEnvironmentLocator::EnvFile {path, ..} if path == ".env.local")
    );
    std::fs::remove_file(root.join("src/app.ts")).unwrap();
    std::fs::write(root.join("src/new.py"), "os.getenv('NEW_INPUT')").unwrap();
    let next = index_project_environment(&roots, &BTreeMap::new()).unwrap();
    let changed = next.evidence.changed_paths(&index.evidence);
    assert!(changed["workspace"].contains(&"src/app.ts".into()));
    assert!(changed["workspace"].contains(&"src/new.py".into()));
    assert_eq!(next.references[0].name, "NEW_INPUT");
}

#[test]
fn mp08_m28_stages_selected_environment_and_rejects_an_unauthenticated_import() {
    use crate::config::{CredentialVaultBackend, DaemonConfig};
    use crate::managed_context::development::*;
    let fixture = Fixture::new();
    let source = fixture.0.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("app.js"), "console.log(process.env.TOKEN)\n").unwrap();
    std::fs::write(
        source.join(".env.local"),
        "TOKEN=synthetic-selected-input\nUNRELATED=leave\n",
    )
    .unwrap();
    let source_id = source.to_string_lossy().to_string();
    let project_id = format!("m28-fixture-{}", rand::random::<u64>());
    let source_config = DaemonConfig::for_tests();
    let mut target_config = DaemonConfig::for_tests()
        .with_session_history_root(fixture.0.join("target-state/sessions"));
    target_config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
    target_config.user_config.credential_vault.path = fixture
        .0
        .join("target-state/vault.json")
        .to_string_lossy()
        .into_owned();
    crate::secret::unlock_chariox_encrypted_vault(
        &target_config.user_config.credential_vault.path,
        "synthetic-fixture-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let roots = BTreeMap::from([(source_id.clone(), source.clone())]);
    let index = index_project_environment(&roots, &BTreeMap::new()).unwrap();
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: project_id.clone(),
        evidence_digest: index.evidence.digest(),
        entries: index.references,
        private_files: Vec::new(),
        toolchain_hints: Vec::new(),
        package_hints: Vec::new(),
        service_hints: Vec::new(),
    };
    let resolved =
        resolve_project_environment(&manifest, &roots, &BTreeMap::new(), &TestVault::default())
            .unwrap();
    let sealed = seal_project_environment(
        "m28-layer-fixture",
        &source_config.daemon_id,
        &target_config.daemon_id,
        &source_config.relay_private_key,
        &target_config.relay_public_key,
        &manifest,
        &resolved,
    )
    .unwrap();
    let exported = export_development_context_with_environment(
        DevelopmentContextExportRequest {
            project_id: project_id.clone(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: source_id.clone(),
                worktree_id: None,
                worktree_path: source.clone(),
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: fixture.0.join("development.tar.gz"),
        },
        Some(DevelopmentProjectEnvironment {
            sealed,
            evidence: index.evidence,
            repository_workspaces: BTreeMap::new(),
        }),
    )
    .unwrap();
    let request = DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: project_id.clone(),
        expected_source_repositories: Some(vec![DevelopmentSourceRepositoryBinding {
            workspace_id: source_id,
            worktree_id: None,
            role: DevelopmentRepositoryRole::Primary,
        }]),
        destination_root: fixture.0.join("target"),
    };
    assert!(import_development_context(request.clone()).is_err());
    assert!(!request.destination_root.exists());
    let authority = ProjectEnvironmentImportAuthority {
        config: target_config.clone(),
        context_id: "m28-layer-fixture".into(),
        source_kernel_id: source_config.daemon_id.clone(),
        source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
            &source_config.relay_public_key,
        ),
        target_kernel_id: target_config.daemon_id.clone(),
    };
    let receipt = import_development_context_with_environment(
        request.clone(),
        "m28-layer-fixture".into(),
        Some(&authority),
    )
    .unwrap();
    let workspace = &receipt.repositories[0].destination_path;
    let envfile = std::fs::read_to_string(workspace.join(".env.local")).unwrap();
    assert!(envfile.contains("TOKEN="));
    assert!(!envfile.contains("UNRELATED"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(workspace.join(".env.local"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let state = ProjectEnvironmentStore::new(&target_config.private_runtime_state_root())
        .load(&project_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        state.manifest.entries[0].workspace_id,
        workspace.to_string_lossy()
    );
    assert_eq!(receipt.schema_version, 4);
    assert!(recover_development_context_publication_with_environment(
        &request,
        "m28-layer-fixture",
        Some(&authority)
    )
    .unwrap()
    .is_some());
    let completion = request
        .destination_root
        .join(".chariox-project-environment-complete.json");
    let proof = std::fs::read(&completion).unwrap();
    std::fs::remove_file(&completion).unwrap();
    assert!(recover_development_context_publication_with_environment(
        &request,
        "m28-layer-fixture",
        Some(&authority)
    )
    .is_err());
    std::fs::write(&completion, proof).unwrap();
    let store = ProjectEnvironmentStore::new(&target_config.private_runtime_state_root());
    store.remove(&project_id).unwrap();
    assert!(recover_development_context_publication_with_environment(
        &request,
        "m28-layer-fixture",
        Some(&authority)
    )
    .is_err());
    store.save(&state).unwrap();
    let bindings = project_launch_environment(
        &target_config,
        &project_id,
        &workspace.to_string_lossy(),
        workspace,
    )
    .unwrap();
    assert_eq!(bindings.iter().count(), 1);
    assert!(!format!("{bindings:?}").contains("synthetic-selected-input"));
    crate::secret::lock_chariox_encrypted_vault(&target_config.user_config.credential_vault.path)
        .unwrap();
}

// MP-08 / MP-10 / MP-11: Review policy covers both interactive and unattended exports.
fn review_state() -> StoredProjectEnvironment {
    let evidence = ProjectEnvironmentEvidence::default();
    let mut manifest = fixture_manifest(vec![entry("NEEDED", ProjectEnvironmentLocator::Missing)]);
    manifest.evidence_digest = evidence.digest();
    manifest.private_files.push(ProjectPrivateFileDecision {
        secret_looking: false,
        workspace_id: "web".into(),
        path: "CLAUDE.local.md".into(),
        bring: true,
        reason: "Personal Project instructions".into(),
    });
    StoredProjectEnvironment {
        manifest,
        evidence,
        reported_missing: Default::default(),
        reviewed_manifest: None,
        last_review: None,
    }
}
#[test]
fn mp08_mp10_review_confirm_and_saved_setup_fast_path() {
    let mut state = review_state();
    assert!(project_environment_needs_review(&state));
    let review = ProjectEnvironmentReview::build(
        &state,
        "App",
        "Fresh slice",
        "main@1234567 · 2 uncommitted".into(),
        false,
        false,
    );
    assert!(!review.expanded && !review.changed_only && !review.unattended);
    assert_eq!(review.rows.len(), 4);
    accept_project_environment_review(&mut state, review);
    assert!(!project_environment_needs_review(&state));
    assert!(state
        .reported_missing
        .contains(&("web".into(), "NEEDED".into())));
}
#[test]
fn mp08_mp10_review_flip_keeps_secret_files_out_of_plain_overlay() {
    let mut state = review_state();
    flip_project_environment_item(
        &mut state,
        &project_environment_item_id("web", "CLAUDE.local.md"),
        true,
    )
    .unwrap();
    assert!(!state.manifest.private_files[0].bring);
    flip_project_environment_item(
        &mut state,
        &project_environment_item_id("web", "NEEDED"),
        false,
    )
    .unwrap();
    assert!(state.manifest.entries[0].excluded);
    state
        .manifest
        .private_files
        .push(ProjectPrivateFileDecision {
            secret_looking: false,
            workspace_id: "web".into(),
            path: ".env.local".into(),
            bring: false,
            reason: "Secrets move through Vault".into(),
        });
    assert!(flip_project_environment_item(
        &mut state,
        &project_environment_item_id("web", ".env.local"),
        true
    )
    .is_err());
}
#[test]
fn mp08_mp10_review_missing_value_paste_is_vault_only_and_skip_is_persistent() {
    let mut state = review_state();
    let vault = TestVault::default();
    supply_project_environment_inputs(
        &mut state,
        BTreeMap::from([(
            ("web".into(), "NEEDED".into()),
            zeroize::Zeroizing::new("synthetic-review-value".into()),
        )]),
        &vault,
    )
    .unwrap();
    let review =
        ProjectEnvironmentReview::build(&state, "App", "Slice", String::new(), false, false);
    assert!(!review.inputs[0].missing);
    assert!(!serde_json::to_string(&review)
        .unwrap()
        .contains("synthetic-review-value"));
    accept_project_environment_review(&mut state, review);
    assert!(!project_environment_needs_review(&state));
    assert!(matches!(
        state.manifest.entries[0].locator,
        ProjectEnvironmentLocator::Vault { .. }
    ));
}
#[test]
fn mp08_mp10_review_changed_only_and_unattended_summary() {
    let mut state = review_state();
    let first =
        ProjectEnvironmentReview::build(&state, "App", "Slice", String::new(), false, false);
    accept_project_environment_review(&mut state, first);
    state
        .manifest
        .private_files
        .push(ProjectPrivateFileDecision {
            secret_looking: false,
            workspace_id: "web".into(),
            path: "notes.md".into(),
            bring: false,
            reason: "Unused personal notes".into(),
        });
    let review =
        ProjectEnvironmentReview::build(&state, "App", "Slice 2", String::new(), true, true);
    assert!(review.changed_only && review.unattended);
    assert!(!review.files[0].changed && review.files[1].changed);
    assert!(!review.inputs[0].changed);
    accept_project_environment_review(&mut state, review);
    assert!(state.last_review.as_ref().unwrap().unattended);
}
#[test]
fn mp08_mp10_review_utility_revision_rejects_new_locators() {
    let state = review_state();
    let input = ProjectEnvironmentDiscoveryInput {
        project_id: state.manifest.project_id.clone(),
        evidence_digest: state.manifest.evidence_digest.clone(),
        changed_paths: Default::default(),
        previous_manifest: Some(state.manifest.clone()),
        references: state.manifest.entries.clone(),
        private_files: vec![ProjectPrivateFileCandidate {
            workspace_id: "web".into(),
            path: "CLAUDE.local.md".into(),
            bytes: 20,
            secret_looking: false,
        }],
        revision: Some("Leave CLAUDE.local.md".into()),
    };
    let mut revised = state.manifest.clone();
    revised.private_files[0].bring = false;
    revised.private_files[0].reason = "Your requested change".into();
    assert!(parse_project_environment_discovery_output(
        &serde_json::to_string(&revised).unwrap(),
        &input
    )
    .is_ok());
    revised.entries[0].locator = ProjectEnvironmentLocator::EnvFile {
        path: "other-secret".into(),
        key: "NEEDED".into(),
    };
    assert!(parse_project_environment_discovery_output(
        &serde_json::to_string(&revised).unwrap(),
        &input
    )
    .is_err());
}

#[test]
fn mp08_mp10_configfile_reference_reaches_discovery_without_contents() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("app.ts"), "const local = readFileSync('app.local.json');\nconst key = readFileSync('dev.pem');\nconst home = process.env.HOME;\n").unwrap();
    std::fs::write(
        fixture.0.join("app.local.json"),
        "{\"password\":\"synthetic-private-value\"}",
    )
    .unwrap();
    std::fs::write(fixture.0.join("dev.pem"), "synthetic-pem-value").unwrap();
    std::fs::write(fixture.0.join(".gitignore"), "app.local.json\ndev.pem\n").unwrap();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&fixture.0)
        .arg("init")
        .arg("-q")
        .status()
        .unwrap();
    assert!(status.success());
    let index = index_project_environment(&fixture.workspaces(), &BTreeMap::new()).unwrap();
    assert_eq!(index.references.len(), 2);
    assert!(index
        .references
        .iter()
        .all(|entry| entry.kind == ProjectEnvironmentEntryKind::ConfigFile));
    assert!(index.private_files.iter().all(|file| file.secret_looking));
    let input = ProjectEnvironmentDiscoveryInput {
        project_id: "fixture-project".into(),
        evidence_digest: index.evidence.digest(),
        changed_paths: index
            .evidence
            .changed_paths(&ProjectEnvironmentEvidence::default()),
        previous_manifest: None,
        references: index.references,
        private_files: index.private_files,
        revision: None,
    };
    let prompt = project_environment_discovery_prompt(&input).unwrap();
    assert!(prompt.contains("app.local.json"));
    assert!(!prompt.contains("synthetic-private-value") && !prompt.contains("synthetic-pem-value"));
}

// MP-08 / MP-10 / MP-11: A declaration named password is code evidence, not a config secret.
#[test]
fn mp08_mp10_reference_code_changes_invalidate_discovery() {
    let fixture = Fixture::new();
    let code = fixture.0.join("app.js");
    std::fs::write(&code, "const password = process.env.DATABASE_PASSWORD;\n").unwrap();
    let first = index_project_environment(&fixture.workspaces(), &BTreeMap::new()).unwrap();
    assert!(first.evidence.files["web"].contains_key("app.js"));
    std::fs::write(&code, "const password = process.env.OTHER_PASSWORD;\n").unwrap();
    let second = index_project_environment(&fixture.workspaces(), &BTreeMap::new()).unwrap();
    assert_eq!(
        second.evidence.changed_paths(&first.evidence)["web"],
        vec!["app.js"]
    );
}

#[test]
fn mp08_mp10_secret_file_cannot_be_reincluded_by_expert_rules() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join(".gitignore"), "app.local.json\n").unwrap();
    std::fs::write(fixture.0.join(".charioxignore"), "!app.local.json\n").unwrap();
    std::fs::write(fixture.0.join(".worktreeinclude"), "app.local.json\n").unwrap();
    let mut manifest = fixture_manifest(Vec::new());
    manifest.private_files.push(ProjectPrivateFileDecision {
        workspace_id: "web".into(),
        path: "app.local.json".into(),
        bring: false,
        secret_looking: true,
        reason: "Referenced secret configuration; Vault only".into(),
    });
    register_project_file_rules(&manifest, &fixture.workspaces());
    let rules =
        crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(&fixture.0);
    assert_eq!(rules.last().unwrap(), "app.local.json");
    let id = project_environment_item_id("web", "app.local.json");
    let mut state = review_state();
    state.manifest = manifest;
    assert!(flip_project_environment_item(&mut state, &id, true).is_err());
    forget_project_file_rules(&state.manifest);
}

#[cfg(unix)]
#[test]
fn mp08_mp10_failed_fresh_materialization_rolls_back_and_preserves_existing_files() {
    let fixture = Fixture::new();
    let outside = fixture.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.0.join("z")).unwrap();
    let mut manifest = fixture_manifest(vec![entry(
        "TOKEN",
        ProjectEnvironmentLocator::EnvFile {
            path: ".env.local".into(),
            key: "TOKEN".into(),
        },
    )]);
    manifest.entries.push(ProjectEnvironmentEntry {
        name: "z/config.json".into(),
        workspace_id: "web".into(),
        kind: ProjectEnvironmentEntryKind::ConfigFile,
        classification: ProjectEnvironmentClassification::Secret,
        excluded: false,
        uses: vec![],
        locator: ProjectEnvironmentLocator::ConfigFile {
            path: "z/config.json".into(),
        },
        status: ProjectEnvironmentEntryStatus::Found,
    });
    let resolved = ResolvedProjectEnvironment {
        values: BTreeMap::from([
            (
                ("web".into(), "TOKEN".into()),
                zeroize::Zeroizing::new("synthetic-token".into()),
            ),
            (
                ("web".into(), "z/config.json".into()),
                zeroize::Zeroizing::new("synthetic-config".into()),
            ),
        ]),
        unresolved: Vec::new(),
    };
    assert!(
        super::materialization_transaction::ProjectEnvironmentMaterialization::prepare(
            &manifest,
            &resolved,
            &fixture.workspaces()
        )
        .is_err()
    );
    assert!(!fixture.0.join(".env.local").exists());
    assert!(!outside.join("config.json").exists());
    std::fs::write(fixture.0.join(".env.local"), "target-owned-content").unwrap();
    manifest.entries.pop();
    assert!(
        super::materialization_transaction::ProjectEnvironmentMaterialization::prepare(
            &manifest,
            &resolved,
            &fixture.workspaces()
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.0.join(".env.local")).unwrap(),
        "target-owned-content"
    );
}
