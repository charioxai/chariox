use super::*;

use crate::managed_context::kernel::{
    KernelContextCompatibility, KernelContextPayload, KernelContextSnapshot,
};

#[test]
fn explicit_empty_package_applies_a_real_development_context_without_kernel_state() {
    let root = std::env::temp_dir().join(format!(
        "chariox-managed-context-empty-apply-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let repository = root.join("repository");
    fs::create_dir_all(&repository).expect("create source repository");
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &["config", "user.email", "tests@chariox.local"],
    );
    git(&repository, &["config", "user.name", "Chariox Tests"]);
    fs::write(repository.join("tracked.txt"), "explicit Empty\n").expect("write source file");
    git(&repository, &["add", "tracked.txt"]);
    git(&repository, &["commit", "-m", "initial"]);
    let development = crate::managed_context::development::export_development_context(
        crate::managed_context::development::DevelopmentContextExportRequest {
            project_id: "project-empty-apply".to_string(),
            repositories: vec![
                crate::managed_context::development::DevelopmentRepositorySelection {
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                    worktree_path: repository,
                    role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                },
            ],
            archive_path: root.join("development.tar.gz"),
        },
    )
    .expect("export development context");
    let binding = ManagedContextPackageBinding {
        plan: ManagedContextPlanBinding {
            context_id: "context-empty-apply".to_string(),
            plan_digest: format!("sha256:{}", "1".repeat(64)),
            kernel_context: ManagedContextKernelSelection::Empty,
            development: ManagedContextDevelopmentSelection::SourceProject {
                project_id: "project-empty-apply".to_string(),
                repositories: vec![DevelopmentSourceRepositoryBinding {
                    role: DevelopmentRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                }],
            },
            provider_accounts: ManagedContextProviderAccountSelection::None,
            git_credentials: ManagedContextGitCredentialSelection::None,
        },
        target_environment_id: "environment-empty-apply".to_string(),
        source_kernel_id: "source-kernel-empty-apply".to_string(),
        source_key_thumbprint: "a".repeat(64),
        target_kernel_id: "target-kernel-empty-apply".to_string(),
        target_key_thumbprint: "b".repeat(64),
    };
    let package = export_managed_context_package(ManagedContextPackageExportRequest {
        plan: binding.plan.clone(),
        target_environment_id: binding.target_environment_id.clone(),
        source_kernel_id: binding.source_kernel_id.clone(),
        source_key_thumbprint: binding.source_key_thumbprint.clone(),
        target_kernel_id: binding.target_kernel_id.clone(),
        target_key_thumbprint: binding.target_key_thumbprint.clone(),
        development: ManagedContextPackageDevelopment::FromSource {
            archive_path: development.archive_path,
            archive_sha256: development.archive_sha256,
        },
        kernel_context: ManagedContextPackageKernel::Empty,
        provider_accounts: ManagedContextPackageProviderAccounts::None,
        git_credentials: ManagedContextPackageGitCredentials::None,
        package_path: root.join("context.chariox"),
    })
    .expect("compose explicit Empty package");
    let receipt = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_empty_apply".to_string(),
        package_path: package.package_path,
        expected_package_sha256: package.package_sha256,
        expected_binding: binding,
        development_destination_root: root.join("managed/project"),
        target_private_key: "unused-for-explicit-empty".to_string(),
        provider_account_target: None,
        git_credential_target: None,
    })
    .expect("apply explicit Empty package");
    assert!(matches!(
        receipt.kernel_context,
        ManagedContextImportedKernelContext::Empty
    ));
    let ManagedContextImportedDevelopment::FromSource { receipt, .. } = receipt.development else {
        panic!("selected development context became Empty")
    };
    assert_eq!(
        fs::read_to_string(receipt.repositories[0].destination_path.join("tracked.txt"))
            .expect("read imported file"),
        "explicit Empty\n"
    );
    fs::remove_dir_all(root).expect("remove Empty apply fixture");
}

#[test]
fn empty_package_round_trips_as_explicit_empty_context() {
    let fixture = PackageFixture::new("empty");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export explicit Empty package");
    assert!(exported.kernel_context_snapshot_sha256.is_none());
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract explicit Empty package");
    assert!(matches!(
        extracted.kernel_context,
        ManagedContextPackageKernel::Empty
    ));
    let ExtractedManagedContextDevelopment::FromSource { archive_path, .. } =
        &extracted.development
    else {
        panic!("selected development context became Empty")
    };
    assert_eq!(
        fs::read(archive_path).expect("read development component"),
        fixture.development_bytes
    );
    let component_root = extracted.component_root().to_path_buf();
    drop(extracted);
    assert!(!component_root.exists());
    fixture.cleanup();
}

#[test]
fn provider_account_package_reader_recovers_a_retained_schema_two_package() {
    let fixture = PackageFixture::new("schema-two");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export current package");
    let legacy_export = rewrite_package_schema(exported, 2, true, true);
    let extracted = extract_managed_context_package(fixture.import_request(&legacy_export))
        .expect("extract retained schema-two package");
    assert!(matches!(
        extracted.provider_accounts,
        ManagedContextPackageProviderAccounts::None
    ));
    assert!(matches!(
        extracted.git_credentials,
        ManagedContextPackageGitCredentials::None
    ));
    drop(extracted);
    fixture.cleanup();
}

#[test]
fn git_credential_package_reader_recovers_a_retained_schema_three_package() {
    let fixture = PackageFixture::new("schema-three");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export current package");
    let legacy_export = rewrite_package_schema(exported, 3, false, true);
    let extracted = extract_managed_context_package(fixture.import_request(&legacy_export))
        .expect("extract retained schema-three package");
    assert!(matches!(
        extracted.git_credentials,
        ManagedContextPackageGitCredentials::None
    ));
    drop(extracted);
    fixture.cleanup();
}

fn rewrite_package_schema(
    exported: ManagedContextPackageExportResult,
    schema_version: u32,
    remove_provider_accounts: bool,
    remove_git_credentials: bool,
) -> ManagedContextPackageExportResult {
    let current = fs::read(&exported.package_path).expect("read current package");
    let manifest_start = PACKAGE_MAGIC.len() + 4;
    let manifest_length = u32::from_be_bytes(
        current[PACKAGE_MAGIC.len()..manifest_start]
            .try_into()
            .expect("manifest length"),
    ) as usize;
    let manifest_end = manifest_start + manifest_length;
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&current[manifest_start..manifest_end]).expect("parse manifest");
    manifest["schemaVersion"] = serde_json::json!(schema_version);
    let manifest_object = manifest.as_object_mut().expect("manifest object");
    if remove_provider_accounts {
        manifest_object.remove("providerAccounts");
    }
    if remove_git_credentials {
        manifest_object.remove("gitCredentials");
    }
    let plan_object = manifest_object
        .get_mut("plan")
        .and_then(serde_json::Value::as_object_mut)
        .expect("plan object");
    if remove_provider_accounts {
        plan_object.remove("providerAccounts");
    }
    if remove_git_credentials {
        plan_object.remove("gitCredentials");
    }
    let legacy_manifest = serde_json::to_vec(&manifest).expect("serialize legacy manifest");
    let mut legacy = Vec::with_capacity(current.len());
    legacy.extend_from_slice(PACKAGE_MAGIC);
    legacy.extend_from_slice(&(legacy_manifest.len() as u32).to_be_bytes());
    legacy.extend_from_slice(&legacy_manifest);
    legacy.extend_from_slice(&current[manifest_end..]);
    fs::write(&exported.package_path, &legacy).expect("write retained legacy package");
    ManagedContextPackageExportResult {
        package_sha256: sha256_bytes(&legacy),
        package_size_bytes: legacy.len() as u64,
        ..exported
    }
}

#[test]
fn package_applies_without_a_development_component_when_the_plan_selects_empty() {
    let fixture = PackageFixture::new("no-development");
    let mut request = fixture.export_request(ManagedContextPackageKernel::Empty);
    request.plan.development = ManagedContextDevelopmentSelection::Empty;
    request.development = ManagedContextPackageDevelopment::Empty;
    let exported = export_managed_context_package(request).expect("export no-development package");
    assert!(exported.development_archive_sha256.is_none());
    let binding = ManagedContextPackageBinding {
        plan: exported.plan.clone(),
        ..fixture.binding.clone()
    };
    let receipt = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_no_development".to_string(),
        package_path: exported.package_path,
        expected_package_sha256: exported.package_sha256,
        expected_binding: binding,
        development_destination_root: fixture.root.join("unused-development"),
        target_private_key: "unused-for-empty-context".to_string(),
        provider_account_target: None,
        git_credential_target: None,
    })
    .expect("apply no-development package");
    assert!(matches!(
        receipt.development,
        ManagedContextImportedDevelopment::Empty
    ));
    assert!(!fixture.root.join("unused-development").exists());
    fixture.cleanup();
}

#[test]
fn package_manifest_carries_a_near_cloud_limit_plan() {
    let fixture = PackageFixture::new("large-plan");
    let mut request = fixture.export_request(ManagedContextPackageKernel::Empty);
    request.plan.development = ManagedContextDevelopmentSelection::SourceProject {
        project_id: "project-large-plan".to_string(),
        repositories: (0..16)
            .map(|index| DevelopmentSourceRepositoryBinding {
                role: if index == 0 {
                    DevelopmentRepositoryRole::Primary
                } else {
                    DevelopmentRepositoryRole::Supporting
                },
                workspace_id: format!("workspace-{index}-{}", "x".repeat(4_060)),
                worktree_id: None,
            })
            .collect(),
    };
    let plan_bytes = serde_json::to_vec(&request.plan).expect("serialize large plan");
    assert!(plan_bytes.len() > 63 * 1024);
    assert!(plan_bytes.len() <= MAX_PLAN_BINDING_BYTES);

    let exported = export_managed_context_package(request).expect("export near-limit plan");
    let package_bytes = fs::read(&exported.package_path).expect("read near-limit package");
    let manifest_length = u32::from_be_bytes(
        package_bytes[PACKAGE_MAGIC.len()..PACKAGE_MAGIC.len() + 4]
            .try_into()
            .expect("manifest length field"),
    ) as usize;
    assert!(manifest_length > 64 * 1024);
    assert!(manifest_length <= MAX_MANIFEST_BYTES);
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract near-limit plan");
    assert!(matches!(
        extracted.kernel_context,
        ManagedContextPackageKernel::Empty
    ));
    drop(extracted);
    fixture.cleanup();
}

#[test]
fn provider_account_package_round_trips_replays_without_overwrite_and_rolls_back() {
    let fixture = PackageFixture::new("provider-account");
    let source_registry =
        ProviderAccountProfileRegistry::open(fixture.root.join("source/registry.json"))
            .expect("open source account registry");
    let source_profile = source_registry
        .create_managed("owner-a", "codex", "Source default")
        .expect("create source account");
    source_registry
        .set_default("owner-a", "codex", &source_profile.profile_id)
        .expect("select source default");
    let source_environment = source_registry
        .resolve_environment("owner-a", "codex", "default")
        .expect("resolve source account");
    fs::write(
        Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
        br#"{"token":"provider-package-secret"}"#,
    )
    .expect("write source provider credential");
    let materialization = source_registry
        .export_managed_context_materialization("owner-a", "codex", "default")
        .expect("export source provider account");
    assert!(!format!("{materialization:?}").contains("provider-package-secret"));

    let mut export = fixture.export_request(ManagedContextPackageKernel::Empty);
    export.plan.development = ManagedContextDevelopmentSelection::Empty;
    export.development = ManagedContextPackageDevelopment::Empty;
    export.plan.provider_accounts = ManagedContextProviderAccountSelection::Selected {
        accounts: vec![ManagedContextProviderAccount {
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
        }],
    };
    export.provider_accounts = ManagedContextPackageProviderAccounts::Selected {
        materializations: vec![materialization],
    };
    assert!(!format!("{export:?}").contains("provider-package-secret"));
    let exported = export_managed_context_package(export).expect("export provider account package");
    assert!(exported.provider_accounts_sha256.is_some());

    let target_registry =
        ProviderAccountProfileRegistry::open(fixture.root.join("target/registry.json"))
            .expect("open target account registry");
    let target_home = fixture.root.join("target-home");
    target_registry
        .migrate_effective_defaults("owner-a", &target_home)
        .expect("create target defaults");
    let provider_target = ManagedContextProviderAccountImportTarget {
        registry: target_registry.clone(),
        owner_user_id: "owner-a".to_string(),
    };
    let request = || ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_provider_account".to_string(),
        package_path: exported.package_path.clone(),
        expected_package_sha256: exported.package_sha256.clone(),
        expected_binding: ManagedContextPackageBinding {
            plan: exported.plan.clone(),
            ..fixture.binding.clone()
        },
        development_destination_root: fixture.root.join("unused-development"),
        target_private_key: "unused-for-empty-context".to_string(),
        provider_account_target: Some(provider_target.clone()),
        git_credential_target: None,
    };
    assert!(!format!("{:?}", request()).contains("provider-package-secret"));
    let receipt = apply_managed_context_package(request()).expect("apply provider account package");
    let ManagedContextImportedProviderAccounts::Selected { accounts } = &receipt.provider_accounts
    else {
        panic!("selected provider account became None")
    };
    assert_eq!(accounts.len(), 1);
    let imported_environment = target_registry
        .resolve_environment("owner-a", "codex", "default")
        .expect("resolve imported target account");
    let imported_auth = Path::new(&imported_environment["CODEX_HOME"]).join("auth.json");
    assert_eq!(
        fs::read_to_string(&imported_auth).expect("read imported provider credential"),
        r#"{"token":"provider-package-secret"}"#
    );

    fs::write(&imported_auth, br#"{"token":"rotated-on-target"}"#)
        .expect("rotate target credential");
    let replay = apply_managed_context_package(request()).expect("replay provider account package");
    assert_eq!(replay.provider_accounts, receipt.provider_accounts);
    assert_eq!(
        fs::read_to_string(&imported_auth).expect("read rotated target credential"),
        r#"{"token":"rotated-on-target"}"#
    );

    rollback_managed_context_package_application(
        &receipt,
        "unused-for-empty-context",
        Some(&provider_target),
        None,
    )
    .expect("roll back provider account package");
    let restored_environment = target_registry
        .resolve_environment("owner-a", "codex", "default")
        .expect("resolve restored target default");
    assert_eq!(
        Path::new(&restored_environment["CODEX_HOME"]),
        target_home.join(".codex")
    );
    assert!(!imported_auth.exists());
    rollback_managed_context_package_application(
        &receipt,
        "unused-for-empty-context",
        Some(&provider_target),
        None,
    )
    .expect("repeat provider-account rollback");
    fixture.cleanup();
}

#[test]
fn provider_account_package_rejects_payload_not_selected_by_the_plan() {
    let fixture = PackageFixture::new("provider-account-binding");
    let source_registry =
        ProviderAccountProfileRegistry::open(fixture.root.join("source/registry.json"))
            .expect("open source account registry");
    let source_profile = source_registry
        .create_managed("owner-a", "codex", "Work")
        .expect("create source account");
    let source_environment = source_registry
        .resolve_environment("owner-a", "codex", &source_profile.profile_id)
        .expect("resolve source account");
    fs::write(
        Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
        br#"{"token":"must-not-package"}"#,
    )
    .expect("write source provider credential");
    let materialization = source_registry
        .export_managed_context_materialization("owner-a", "codex", &source_profile.profile_id)
        .expect("export source provider account");
    let mut export = fixture.export_request(ManagedContextPackageKernel::Empty);
    export.provider_accounts = ManagedContextPackageProviderAccounts::Selected {
        materializations: vec![materialization],
    };
    let package_path = export.package_path.clone();
    let error = export_managed_context_package(export)
        .expect_err("unselected provider payload must be rejected");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    assert!(!package_path.exists());
    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn github_credential_package_replays_and_rolls_back_through_official_cli() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = PackageFixture::new("github-credential");
    let bin = fixture.root.join("fake-bin");
    let source_home = fixture.root.join("source-git-home");
    let target_home = fixture.root.join("target-git-home");
    fs::create_dir_all(&bin).expect("create fake command directory");
    fs::create_dir_all(&source_home).expect("create source Git home");
    fs::create_dir_all(&target_home).expect("create target Git home");
    fs::write(source_home.join("token"), "github-package-secret\n")
        .expect("write source GitHub token");
    let gh = bin.join("gh");
    fs::write(
        &gh,
        r#"#!/bin/sh
set -eu
case "$1 $2" in
  "auth token") test -f "$HOME/token" && /bin/cat "$HOME/token" ;;
  "auth login")
    /bin/cat > "$HOME/token"
    /bin/mkdir -p "$GH_CONFIG_DIR"
    /usr/bin/printf 'github.com:\n  user: test\n' > "$GH_CONFIG_DIR/hosts.yml"
    ;;
  "auth setup-git") /usr/bin/touch "$HOME/git-helper" ;;
  "auth logout")
    /bin/rm -f "$HOME/token" "$GH_CONFIG_DIR/hosts.yml"
    ;;
  *) exit 2 ;;
esac
"#,
    )
    .expect("write fake gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).expect("make fake gh executable");
    let git = bin.join("git");
    fs::write(
        &git,
        r#"#!/bin/sh
set -eu
case "$1 $2 $3" in
  "config --global --get-regexp") test -f "$HOME/git-helper" ;;
  "config --global --remove-section") /bin/rm -f "$HOME/git-helper" ;;
  *) exit 2 ;;
esac
"#,
    )
    .expect("write fake git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).expect("make fake git executable");
    let path = bin.into_os_string();
    let source_context = crate::managed_context::scm::GitCredentialCommandContext::for_tests(
        source_home,
        path.clone(),
    );
    let target_context = crate::managed_context::scm::GitCredentialCommandContext::for_tests(
        target_home.clone(),
        path,
    );
    let selection = ManagedContextGitCredentialSelection::Selected {
        credential_ids: vec![crate::managed_context::scm::GITHUB_CREDENTIAL_ID.to_string()],
    };
    let materializations =
        crate::managed_context::scm::export_selected_git_credentials(&selection, &source_context)
            .expect("export selected GitHub credential");
    let mut export = fixture.export_request(ManagedContextPackageKernel::Empty);
    export.plan.development = ManagedContextDevelopmentSelection::Empty;
    export.development = ManagedContextPackageDevelopment::Empty;
    export.plan.git_credentials = selection;
    export.git_credentials = ManagedContextPackageGitCredentials::Selected { materializations };
    assert!(!format!("{export:?}").contains("github-package-secret"));
    let exported = export_managed_context_package(export).expect("export Git credential package");
    assert!(exported.git_credentials_sha256.is_some());
    let target = ManagedContextGitCredentialImportTarget {
        command_context: target_context,
    };
    let request = || ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_github_credential".to_string(),
        package_path: exported.package_path.clone(),
        expected_package_sha256: exported.package_sha256.clone(),
        expected_binding: ManagedContextPackageBinding {
            plan: exported.plan.clone(),
            ..fixture.binding.clone()
        },
        development_destination_root: fixture.root.join("unused-development"),
        target_private_key: "unused-for-empty-context".to_string(),
        provider_account_target: None,
        git_credential_target: Some(target.clone()),
    };
    let receipt = apply_managed_context_package(request()).expect("apply Git credential package");
    let ManagedContextImportedGitCredentials::Selected { credentials } = &receipt.git_credentials
    else {
        panic!("selected Git credential became None")
    };
    assert_eq!(credentials.len(), 1);
    assert_eq!(
        fs::read_to_string(target_home.join("token")).expect("read imported GitHub token"),
        "github-package-secret\n"
    );
    assert_eq!(
        apply_managed_context_package(request())
            .expect("replay Git credential package")
            .git_credentials,
        receipt.git_credentials
    );
    rollback_managed_context_package_application(
        &receipt,
        "unused-for-empty-context",
        None,
        Some(&target),
    )
    .expect("roll back Git credential package");
    assert!(!target_home.join("token").exists());
    fixture.cleanup();
}

#[test]
fn provider_account_rollback_attempts_every_import_after_one_failure() {
    let fixture = PackageFixture::new("provider-account-rollback-all");
    let source = ProviderAccountProfileRegistry::open(fixture.root.join("source/registry.json"))
        .expect("open source account registry");
    let mut materializations = Vec::new();
    for (provider, environment_key, relative_path) in [
        ("codex", "CODEX_HOME", "auth.json"),
        ("opencode", "XDG_DATA_HOME", "opencode/auth.json"),
    ] {
        let profile = source
            .create_managed("owner-a", provider, provider)
            .expect("create source provider account");
        let environment = source
            .resolve_environment("owner-a", provider, &profile.profile_id)
            .expect("resolve source provider account");
        let credential = Path::new(&environment[environment_key]).join(relative_path);
        fs::create_dir_all(credential.parent().expect("credential parent"))
            .expect("create credential parent");
        fs::write(&credential, br#"{"token":"secret"}"#).expect("write provider credential");
        materializations.push(
            source
                .export_managed_context_materialization("owner-a", provider, &profile.profile_id)
                .expect("export provider credential"),
        );
    }

    let target = ProviderAccountProfileRegistry::open(fixture.root.join("target/registry.json"))
        .expect("open target account registry");
    let receipts = materializations
        .iter()
        .map(|materialization| {
            target
                .materialize_managed_context_replica(
                    "owner-a",
                    "context-rollback-all",
                    &"a".repeat(64),
                    materialization,
                )
                .expect("materialize target provider account")
        })
        .collect::<Vec<_>>();
    let mut conflicting_receipt = receipts[1].clone();
    conflicting_receipt.materialization_sha256 = "b".repeat(64);
    let imported = ManagedContextImportedProviderAccounts::Selected {
        accounts: vec![receipts[0].clone(), conflicting_receipt],
    };
    let target_binding = ManagedContextProviderAccountImportTarget {
        registry: target.clone(),
        owner_user_id: "owner-a".to_string(),
    };
    rollback_imported_provider_accounts(Some(&target_binding), &imported)
        .expect_err("one conflicting receipt must report rollback failure");
    assert!(target
        .get("owner-a", "codex", &receipts[0].profile_id)
        .is_err());
    assert!(target
        .get("owner-a", "opencode", &receipts[1].profile_id)
        .is_ok());
    target
        .rollback_managed_context_replica("owner-a", &receipts[1])
        .expect("clean remaining provider account");
    fixture.cleanup();
}

#[test]
fn source_kernel_package_round_trips_with_identity_bindings() {
    let fixture = PackageFixture::new("source");
    let snapshot = fixture.kernel_snapshot();
    let exported = export_managed_context_package(fixture.export_request(
        ManagedContextPackageKernel::FromKernel(Box::new(snapshot.clone())),
    ))
    .expect("export source-kernel package");
    assert_eq!(
        exported.kernel_context_snapshot_sha256.as_deref(),
        Some(snapshot.snapshot_sha256.as_str())
    );
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract source-kernel package");
    match &extracted.kernel_context {
        ManagedContextPackageKernel::FromKernel(imported) => {
            assert_eq!(imported.as_ref(), &snapshot)
        }
        ManagedContextPackageKernel::Empty => panic!("source context silently became Empty"),
    }
    drop(extracted);
    fixture.cleanup();
}

#[test]
fn receipt_capacity_is_rejected_before_context_publication() {
    let _env_guard = crate::env_lock::lock();
    let fixture = PackageFixture::new("receipt-capacity");
    let exported = export_managed_context_package(fixture.export_request(
        ManagedContextPackageKernel::FromKernel(Box::new(fixture.kernel_snapshot())),
    ))
    .expect("export receipt-capacity package");
    let binding = ManagedContextPackageBinding {
        plan: exported.plan.clone(),
        ..fixture.binding.clone()
    };
    let prior_capability_root = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    let prior_vault_path = std::env::var_os("CHARIOX_MANAGED_VAULT_PATH");
    std::env::set_var(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        format!(
            "/{}",
            "c".repeat(crate::managed_context::transfer::MAX_IMPORT_RECEIPT_BYTES)
        ),
    );
    std::env::set_var("CHARIOX_MANAGED_VAULT_PATH", "/managed-vault.json");
    let destination = fixture.root.join("must-not-publish");
    let request = ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_receipt_capacity".to_string(),
        package_path: exported.package_path,
        expected_package_sha256: exported.package_sha256,
        expected_binding: binding,
        development_destination_root: destination.clone(),
        target_private_key: "unused-before-preflight".to_string(),
        provider_account_target: None,
        git_credential_target: None,
    };
    let error = apply_managed_context_package(request)
        .expect_err("oversized durable receipt must fail before publication");
    match prior_capability_root {
        Some(value) => std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", value),
        None => std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT"),
    }
    match prior_vault_path {
        Some(value) => std::env::set_var("CHARIOX_MANAGED_VAULT_PATH", value),
        None => std::env::remove_var("CHARIOX_MANAGED_VAULT_PATH"),
    }
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    assert!(!destination.exists());
    fixture.cleanup();
}

#[test]
fn package_rejects_a_different_authenticated_binding() {
    let fixture = PackageFixture::new("binding");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut request = fixture.import_request(&exported);
    request.expected_binding.target_environment_id = "environment-other".to_string();
    let error = extract_managed_context_package(request).expect_err("binding mismatch must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

#[test]
fn package_rejects_a_substituted_workspace_selection_before_publication() {
    let fixture = PackageFixture::new("workspace-binding");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut request = fixture.import_request(&exported);
    let ManagedContextDevelopmentSelection::SourceProject { repositories, .. } =
        &mut request.expected_binding.plan.development
    else {
        panic!("fixture must select source development")
    };
    repositories[0].workspace_id = "workspace-substituted".to_string();
    let error = extract_managed_context_package(request)
        .expect_err("substituted Workspace selection must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

#[test]
fn package_rejects_mutation_after_outer_digest_is_fixed() {
    let fixture = PackageFixture::new("mutation");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut bytes = fs::read(&exported.package_path).expect("read package");
    let index = bytes.len() / 2;
    bytes[index] ^= 1;
    fs::write(&exported.package_path, bytes).expect("mutate package in place");
    let error = extract_managed_context_package(fixture.import_request(&exported))
        .expect_err("mutated package must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

struct PackageFixture {
    root: PathBuf,
    development_path: PathBuf,
    development_bytes: Vec<u8>,
    binding: ManagedContextPackageBinding,
}

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

impl PackageFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-context-package-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&root).expect("create package fixture");
        let development_path = root.join("development.tar.gz");
        let development_bytes = b"bounded-development-archive".to_vec();
        fs::write(&development_path, &development_bytes).expect("write development fixture");
        Self {
            root,
            development_path,
            development_bytes,
            binding: ManagedContextPackageBinding {
                plan: ManagedContextPlanBinding {
                    context_id: "context-1".to_string(),
                    plan_digest: format!("sha256:{}", "1".repeat(64)),
                    kernel_context: ManagedContextKernelSelection::Empty,
                    development: ManagedContextDevelopmentSelection::SourceProject {
                        project_id: "project-1".to_string(),
                        repositories: vec![DevelopmentSourceRepositoryBinding {
                            role: DevelopmentRepositoryRole::Primary,
                            workspace_id: "workspace-1".to_string(),
                            worktree_id: None,
                        }],
                    },
                    provider_accounts: ManagedContextProviderAccountSelection::None,
                    git_credentials: ManagedContextGitCredentialSelection::None,
                },
                target_environment_id: "environment-1".to_string(),
                source_kernel_id: "source-kernel-1".to_string(),
                source_key_thumbprint: "a".repeat(64),
                target_kernel_id: "target-kernel-1".to_string(),
                target_key_thumbprint: "b".repeat(64),
            },
        }
    }

    fn export_request(
        &self,
        kernel_context: ManagedContextPackageKernel,
    ) -> ManagedContextPackageExportRequest {
        let mut plan = self.binding.plan.clone();
        plan.kernel_context = match &kernel_context {
            ManagedContextPackageKernel::Empty => ManagedContextKernelSelection::Empty,
            ManagedContextPackageKernel::FromKernel(_) => {
                ManagedContextKernelSelection::SourceKernel
            }
        };
        ManagedContextPackageExportRequest {
            plan,
            target_environment_id: self.binding.target_environment_id.clone(),
            source_kernel_id: self.binding.source_kernel_id.clone(),
            source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
            target_kernel_id: self.binding.target_kernel_id.clone(),
            target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: self.development_path.clone(),
                archive_sha256: sha256_bytes(&self.development_bytes),
            },
            kernel_context,
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials: ManagedContextPackageGitCredentials::None,
            package_path: self.root.join("context.chariox"),
        }
    }

    fn import_request(
        &self,
        exported: &ManagedContextPackageExportResult,
    ) -> ManagedContextPackageImportRequest {
        ManagedContextPackageImportRequest {
            package_path: exported.package_path.clone(),
            expected_package_sha256: exported.package_sha256.clone(),
            expected_binding: ManagedContextPackageBinding {
                plan: exported.plan.clone(),
                ..self.binding.clone()
            },
        }
    }

    fn kernel_snapshot(&self) -> KernelContextSnapshot {
        KernelContextSnapshot {
            payload: KernelContextPayload {
                schema_version: 1,
                context_id: self.binding.plan.context_id.clone(),
                source_kernel_id: self.binding.source_kernel_id.clone(),
                source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
                target_kernel_id: self.binding.target_kernel_id.clone(),
                target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
                compatibility: KernelContextCompatibility {
                    source_kernel_version: "0.1.0".to_string(),
                    local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                    relay_peer_protocol_version:
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                },
                extensions: Vec::new(),
                dependencies: Vec::new(),
                vault: crate::secret::TransferredVaultSnapshot {
                    schema_version: 1,
                    context_id: self.binding.plan.context_id.clone(),
                    source_kernel_id: self.binding.source_kernel_id.clone(),
                    source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
                    target_kernel_id: self.binding.target_kernel_id.clone(),
                    target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
                    vault_sha256: "c".repeat(64),
                    vault_size_bytes: 1,
                    vault_file_base64: "AA==".to_string(),
                    sealed_unlock_key: chariox_relay::protocol::EncryptedRelayPayload {
                        sender_public_key: "sender".to_string(),
                        nonce: "nonce".to_string(),
                        ciphertext: "ciphertext".to_string(),
                    },
                },
            },
            snapshot_sha256: "d".repeat(64),
        }
    }

    fn cleanup(&self) {
        fs::remove_dir_all(&self.root).expect("remove package fixture");
    }
}
