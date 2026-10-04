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
use std::path::{Path, PathBuf};
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
        Self::new_in(&std::env::temp_dir())
    }
    fn new_in(parent: &Path) -> Self {
        let path = parent.join(format!("chariox-envlayer2-{}", rand::random::<u64>()));
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
        source: None,
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
            source: None,
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
            source: None,
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
    m28_environment_transfer(None);
}

// MP-08 / MP-10 / MP-11: Checkout and dirty overlay files precede the sealed layer.
#[test]
fn mp08_mp10_mp11_m28_transfers_git_tracked_config_and_rolls_back_safely() {
    for changed in [false, true] {
        m28_environment_transfer(Some(changed));
    }
}

fn m28_environment_transfer(tracked_config: Option<bool>) {
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
    let selected_config = "{\"endpoint\":\"selected-fixture\"}\n";
    if let Some(changed) = tracked_config {
        std::fs::write(
            source.join("app.js"),
            "console.log(process.env.TOKEN); readFileSync(\"config.json\");\n",
        )
        .unwrap();
        std::fs::write(
            source.join("config.json"),
            if changed { "{}\n" } else { selected_config },
        )
        .unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "app.js", "config.json"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "--quiet",
                "-m",
                "config fixture",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(source.join("config.json"), selected_config).unwrap();
    }
    let source_id = source.to_string_lossy().to_string();
    let project_id = format!("m28-fixture-{}", rand::random::<u64>());
    let source_config = DaemonConfig::for_tests();
    let mut target_config = DaemonConfig::for_tests()
        .with_session_history_root(fixture.0.join("target-state/sessions"));
    target_config.user_config.state.path = Some(
        fixture
            .0
            .join("target-state/state.db")
            .to_string_lossy()
            .into_owned(),
    );
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
    #[cfg(unix)]
    if tracked_config.is_some() {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::read_to_string(workspace.join("config.json")).unwrap(),
            selected_config
        );
        assert_eq!(
            std::fs::metadata(workspace.join("config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(receipt.repositories[0].workspace_kind.is_git());
        assert!(receipt.repositories[0].head_sha.len() == 40);
    }
    let state = ProjectEnvironmentStore::new(&target_config.private_runtime_state_root())
        .load(&project_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        state.manifest.entries[0].workspace_id,
        workspace.to_string_lossy()
    );
    if tracked_config.is_some() {
        // Commit fails after staged files were published; preserve the first import.
        let mut retry = request.clone();
        retry.destination_root = fixture.0.join("rollback-target");
        let error = import_development_context_with_environment(
            retry.clone(),
            "m28-rollback-fixture".into(),
            Some(&authority),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("independent Project environment"),
            "{error}"
        );
        assert!(!retry.destination_root.exists());
        assert_eq!(
            std::fs::read_to_string(workspace.join("config.json")).unwrap(),
            selected_config
        );
        assert_eq!(
            std::fs::read_to_string(source.join("config.json")).unwrap(),
            selected_config
        );
        assert_eq!(
            ProjectEnvironmentStore::new(&target_config.private_runtime_state_root())
                .load(&project_id)
                .unwrap()
                .unwrap(),
            state
        );
        assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("import-")));
    }
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
        source: None,
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
        uses: vec![ProjectEnvironmentUse {
            path: "app.js".into(),
            line: 1,
        }],
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
            &fixture.workspaces(),
            super::materialization_transaction::MaterializationTarget::MountedSource,
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
            &fixture.workspaces(),
            super::materialization_transaction::MaterializationTarget::MountedSource,
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.0.join(".env.local")).unwrap(),
        "target-owned-content"
    );
}

#[test]
fn mp08_mp10_mp11_referenced_config_cannot_also_enter_plain_overlay() {
    let mut state = review_state();
    state.manifest.entries.push(ProjectEnvironmentEntry {
        name: "app.local.json".into(),
        workspace_id: "web".into(),
        kind: ProjectEnvironmentEntryKind::ConfigFile,
        classification: ProjectEnvironmentClassification::NonSecret,
        excluded: false,
        uses: vec![ProjectEnvironmentUse {
            path: "app.ts".into(),
            line: 3,
        }],
        locator: ProjectEnvironmentLocator::ConfigFile {
            path: "app.local.json".into(),
        },
        status: ProjectEnvironmentEntryStatus::Found,
    });
    state
        .manifest
        .private_files
        .push(ProjectPrivateFileDecision {
            workspace_id: "web".into(),
            path: "app.local.json".into(),
            bring: true,
            reason: "Application configuration".into(),
            secret_looking: false,
        });
    normalize_project_config_file_decisions(&mut state.manifest);
    let file = state.manifest.private_files.last().unwrap();
    assert!(!file.bring && file.secret_looking);
    assert!(flip_project_environment_item(
        &mut state,
        &project_environment_item_id("web", "app.local.json"),
        true
    )
    .is_err());
    state.manifest.validate().unwrap();
}

#[test]
fn mp08_mp10_mp11_adjustment_excludes_concurrent_exports_and_adjustments() {
    let fixture = Fixture::new();
    let store = ProjectEnvironmentStore::new(&fixture.0);
    let first = store.try_lock("project").unwrap();
    assert!(store.try_lock("project").is_err());
    // Another Project's export remains independent.
    let other = store.try_lock("other-project").unwrap();
    drop(first);
    assert!(store.try_lock("project").is_ok());
    drop(other);
}

#[test]
fn mp08_mp10_mp11_explicit_private_fetch_rolls_back_without_overwriting_user_files() {
    let root =
        std::env::temp_dir().join(format!("chariox-envlayer5-fetch-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut additions = ProjectPrivateFileAdditions::default();
        additions
            .add(
                &root,
                "notes.md",
                b"MP-08 / MP-10 / MP-11 synthetic private notes",
            )
            .unwrap();
        assert!(root.join("notes.md").is_file());
        assert!(additions.add(&root, "notes.md", b"overwrite").is_err());
        assert!(additions.add(&root, ".env", b"sealed").is_err());
    }
    assert!(!root.join("notes.md").exists());
    {
        let mut additions = ProjectPrivateFileAdditions::default();
        additions
            .add(&root, "notes.md", b"synthetic private notes")
            .unwrap();
        std::fs::rename(root.join("notes.md"), root.join("old.md")).unwrap();
        std::fs::write(root.join("notes.md"), "user replacement").unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "user replacement"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mp08_mp10_mp11_sealed_value_edits_do_not_reclassify_metadata() {
    let fixture = Fixture::new();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(&fixture.0)
        .status()
        .unwrap()
        .success());
    std::fs::write(fixture.0.join(".gitignore"), ".env.local\n").unwrap();
    std::fs::write(
        fixture.0.join("app.ts"),
        "const label = process.env.APP_LABEL;\n",
    )
    .unwrap();
    std::fs::write(fixture.0.join(".env.local"), "APP_LABEL=first\n").unwrap();
    let before = index_project_environment(&fixture.workspaces(), &BTreeMap::new()).unwrap();
    assert_eq!(before.evidence.private_inventory["web"][".env.local"], 0);
    std::fs::write(
        fixture.0.join(".env.local"),
        "APP_LABEL=a-longer-second-value\n",
    )
    .unwrap();
    let after = index_project_environment(&fixture.workspaces(), &BTreeMap::new()).unwrap();
    assert_eq!(before.evidence, after.evidence);
    assert_eq!(before.references, after.references);
    assert!(after.evidence.changed_paths(&before.evidence).is_empty());
}

#[test]
fn mp08_mp10_mp11_incremental_utility_cannot_reclassify_unchanged_selections() {
    let mut manifest =
        fixture_manifest(vec![entry("APP_LABEL", ProjectEnvironmentLocator::Missing)]);
    manifest.entries[0].classification = ProjectEnvironmentClassification::NonSecret;
    manifest.entries[0].excluded = true;
    manifest.entries[0].status = ProjectEnvironmentEntryStatus::Found;
    let input = ProjectEnvironmentDiscoveryInput {
        revision: None,
        project_id: manifest.project_id.clone(),
        evidence_digest: manifest.evidence_digest.clone(),
        previous_manifest: Some(manifest.clone()),
        changed_paths: BTreeMap::from([("web".into(), vec!["package-lock.json".into()])]),
        references: manifest.entries.clone(),
        private_files: vec![],
    };
    let mut output = manifest.clone();
    output.entries[0].classification = ProjectEnvironmentClassification::Secret;
    output.entries[0].excluded = false;
    output.entries[0].status = ProjectEnvironmentEntryStatus::Missing;
    assert_eq!(
        parse_project_environment_discovery_output(
            &serde_json::to_string(&output).unwrap(),
            &input
        )
        .unwrap(),
        manifest
    );
    output.entries.clear();
    assert_eq!(
        parse_project_environment_discovery_output(
            &serde_json::to_string(&output).unwrap(),
            &input
        )
        .unwrap(),
        manifest
    );
    output.entries = manifest.entries.clone();
    output.entries[0].locator = ProjectEnvironmentLocator::EnvFile {
        path: "foreign.env".into(),
        key: "APP_LABEL".into(),
    };
    assert!(parse_project_environment_discovery_output(
        &serde_json::to_string(&output).unwrap(),
        &input
    )
    .is_err());
}

// MP-08 / MP-10 / MP-11: Mounted source reuse never overwrites differing target data.
#[cfg(unix)]
#[test]
fn mp08_mp10_mp11_mounted_config_reconciliation_preserves_rollback_and_target_state() {
    use super::materialization_transaction::{
        MaterializationTarget, ProjectEnvironmentMaterialization,
    };
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let fixture = Fixture::new();
    let path = fixture.0.join("config.json");
    let config_entry = |name: &str| ProjectEnvironmentEntry {
        name: name.into(),
        workspace_id: "web".into(),
        kind: ProjectEnvironmentEntryKind::ConfigFile,
        classification: ProjectEnvironmentClassification::Secret,
        excluded: false,
        uses: vec![ProjectEnvironmentUse {
            path: "app.js".into(),
            line: 1,
        }],
        locator: ProjectEnvironmentLocator::ConfigFile { path: name.into() },
        status: ProjectEnvironmentEntryStatus::Found,
    };
    let mut manifest = fixture_manifest(vec![config_entry("config.json")]);
    let mut resolved = ResolvedProjectEnvironment {
        values: BTreeMap::from([(
            ("web".into(), "config.json".into()),
            zeroize::Zeroizing::new("selected-fixture".into()),
        )]),
        unresolved: vec![],
    };
    std::fs::write(&path, "selected-fixture").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let original = std::fs::metadata(&path).unwrap();
    let prepare = |manifest: &ProjectEnvironmentManifest, resolved: &ResolvedProjectEnvironment| {
        ProjectEnvironmentMaterialization::prepare(
            manifest,
            resolved,
            &fixture.workspaces(),
            MaterializationTarget::MountedSource,
        )
    };
    let transaction = prepare(&manifest, &resolved).unwrap();
    let private = std::fs::metadata(&path).unwrap();
    assert_eq!(private.ino(), original.ino());
    assert_eq!(private.permissions().mode() & 0o777, 0o600);
    drop(transaction);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "selected-fixture");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );

    // A later file failure rolls back a reused file and a newly created file.
    manifest.entries.push(config_entry("a-new.json"));
    manifest.entries.push(config_entry("z/config.json"));
    for name in ["a-new.json", "z/config.json"] {
        resolved.values.insert(
            ("web".into(), name.into()),
            zeroize::Zeroizing::new("selected-fixture".into()),
        );
    }
    std::os::unix::fs::symlink(&fixture.0, fixture.0.join("z")).unwrap();
    assert!(prepare(&manifest, &resolved).is_err());
    assert!(!fixture.0.join("a-new.json").exists());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    manifest.entries.truncate(1);

    std::fs::write(&path, "target-owned-content").unwrap();
    assert!(prepare(&manifest, &resolved).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "target-owned-content"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::write(fixture.0.join("other.json"), "selected-fixture").unwrap();
    std::os::unix::fs::symlink(fixture.0.join("other.json"), &path).unwrap();
    assert!(prepare(&manifest, &resolved).is_err());
    assert!(std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(fixture.0.join("other.json"), &path).unwrap();
    assert!(prepare(&manifest, &resolved).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, "selected-fixture").unwrap();

    // Drop must preserve a target replacement made after prepare.
    let transaction = prepare(&manifest, &resolved).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, "later-target-replacement").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    drop(transaction);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "later-target-replacement"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    std::fs::write(&path, "selected-fixture").unwrap();
    prepare(&manifest, &resolved).unwrap().commit();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "selected-fixture");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

// MP-08 / MP-10 / MP-11: Docker shares a fresh checkout by group, not file UID.
// The worker must import a matching tracked config as a private target-owned file.
#[cfg(target_os = "linux")]
#[test]
fn mp08_mp10_mp11_foreign_owned_mounted_config_is_private_and_transactional() {
    use super::materialization_transaction::{
        MaterializationTarget, ProjectEnvironmentMaterialization,
    };
    use std::os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    };
    const CHILD_ROOT: &str = "CHARIOX_TEST_FOREIGN_CONFIG_ROOT";
    if let Ok(root) = std::env::var(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let mut manifest = fixture_manifest(vec![ProjectEnvironmentEntry {
            name: "config.json".into(),
            workspace_id: "web".into(),
            kind: ProjectEnvironmentEntryKind::ConfigFile,
            classification: ProjectEnvironmentClassification::Secret,
            excluded: false,
            uses: vec![ProjectEnvironmentUse {
                path: "app.c".into(),
                line: 1,
            }],
            locator: ProjectEnvironmentLocator::ConfigFile {
                path: "config.json".into(),
            },
            status: ProjectEnvironmentEntryStatus::Found,
        }]);
        let mut resolved = ResolvedProjectEnvironment {
            values: BTreeMap::from([(
                ("web".into(), "config.json".into()),
                zeroize::Zeroizing::new("synthetic selected config".into()),
            )]),
            unresolved: vec![],
        };
        let mode = std::env::var("CHARIOX_TEST_FOREIGN_CONFIG_MODE").unwrap();
        if mode == "fail-later" {
            let mut entry = manifest.entries[0].clone();
            entry.name = "z/config.json".into();
            entry.locator = ProjectEnvironmentLocator::ConfigFile {
                path: entry.name.clone(),
            };
            resolved.values.insert(
                ("web".into(), entry.name.clone()),
                zeroize::Zeroizing::new("synthetic selected config".into()),
            );
            manifest.entries.push(entry);
        }
        let prepared = ProjectEnvironmentMaterialization::prepare(
            &manifest,
            &resolved,
            &BTreeMap::from([("web".into(), root.clone())]),
            MaterializationTarget::MountedSource,
        );
        if mode == "fail-later" {
            assert!(prepared.is_err());
            return;
        }
        let transaction =
            prepared.expect("matching shared configuration must become private on the worker");
        let metadata = std::fs::metadata(root.join("config.json")).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        if mode == "replacement" {
            std::fs::write(root.join("replacement"), "synthetic target edit").unwrap();
            std::fs::rename(root.join("replacement"), root.join("config.json")).unwrap();
        }
        if mode == "rollback-denied" {
            // MP-08 / MP-10 / MP-11: reject only this child's rollback syscall,
            // while leaving unlink available to expose destructive cleanup.
            let mut filters = [
                libc::sock_filter {
                    code: 0x20,
                    jt: 0,
                    jf: 0,
                    k: 0,
                },
                libc::sock_filter {
                    code: 0x15,
                    jt: 0,
                    jf: 1,
                    k: libc::SYS_renameat2 as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: libc::SECCOMP_RET_ERRNO | libc::EACCES as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: libc::SECCOMP_RET_ALLOW,
                },
            ];
            let program = libc::sock_fprog {
                len: filters.len() as u16,
                filter: filters.as_mut_ptr(),
            };
            assert_eq!(
                unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
                0
            );
            assert_eq!(
                unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
                0
            );
        }
        if mode == "commit" {
            transaction.commit();
        }
        return;
    }
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    // A UID-dropped worker must not depend on a private TMPDIR's ancestors.
    let fixture = Fixture::new_in(Path::new("/tmp"));
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o775)).unwrap();
    let directory = std::ffi::CString::new(fixture.0.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::chown(directory.as_ptr(), 0, 31001) }, 0);
    let path = fixture.0.join("config.json");
    std::fs::write(&path, "synthetic selected config").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    // The child UID cannot traverse a root-owned builder checkout.
    let executable = fixture.0.join("worker-test");
    // Linking avoids a writable executable descriptor being inherited by a
    // concurrent fork, which can make the next exec fail with ETXTBSY.
    std::fs::hard_link(std::env::current_exe().unwrap(), &executable)
        .or_else(|error| {
            if error.raw_os_error() == Some(libc::EXDEV) {
                std::fs::copy(std::env::current_exe().unwrap(), &executable).map(|_| ())
            } else {
                Err(error)
            }
        })
        .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    for mode in [
        "drop",
        "fail-later",
        "replacement",
        "rollback-denied",
        "commit",
    ] {
        let file_name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(file_name.as_ptr(), 0, 31001) }, 0);
        std::fs::write(&path, "synthetic selected config").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        let original = std::fs::metadata(&path).unwrap();
        if mode == "fail-later" {
            std::os::unix::fs::symlink(&fixture.0, fixture.0.join("z")).unwrap();
        }
        let mut command = std::process::Command::new(&executable);
        command.args(["--exact", "project_environment::tests::mp08_mp10_mp11_foreign_owned_mounted_config_is_private_and_transactional", "--test-threads=1"])
            .env(CHILD_ROOT, &fixture.0).env("CHARIOX_TEST_FOREIGN_CONFIG_MODE", mode);
        unsafe {
            command.pre_exec(|| {
                if libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(31001) != 0
                    || libc::setuid(31001) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "isolated worker import failed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let after = std::fs::metadata(&path).unwrap();
        if mode == "rollback-denied" {
            let backups: Vec<_> = std::fs::read_dir(&fixture.0)
                .unwrap()
                .map(Result::unwrap)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".chariox-env-")
                })
                .collect();
            assert_eq!(
                backups.len(),
                1,
                "failed restore must retain the original inode"
            );
            assert_eq!(backups[0].metadata().unwrap().ino(), original.ino());
            assert_eq!(backups[0].metadata().unwrap().uid(), 0);
            std::fs::remove_file(backups[0].path()).unwrap();
            assert_eq!(after.uid(), 31001);
            assert_eq!(after.permissions().mode() & 0o777, 0o600);
        } else if mode == "replacement" {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "synthetic target edit"
            );
            assert_eq!(after.uid(), 31001);
        } else if mode == "commit" {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "synthetic selected config"
            );
            assert_eq!(after.uid(), 31001);
            assert_eq!(after.permissions().mode() & 0o777, 0o600);
        } else {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "synthetic selected config"
            );
            assert_eq!(after.ino(), original.ino());
            assert_eq!(after.uid(), 0);
            assert_eq!(after.permissions().mode() & 0o777, 0o664);
        }
        if mode == "fail-later" {
            std::fs::remove_file(fixture.0.join("z")).unwrap();
        }
        assert_eq!(
            std::fs::read_dir(&fixture.0).unwrap().count(),
            2,
            "no transaction backup remains"
        );
    }
}
