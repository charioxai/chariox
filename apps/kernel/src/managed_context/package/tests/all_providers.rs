//! Row 582: a plan whose provider accounts are `all` names no account; the package payload is the list.
use super::*;

fn source_codex_materialization(
    fixture: &PackageFixture,
    secret: &str,
) -> ProviderAccountMaterialization {
    let source_registry =
        ProviderAccountProfileRegistry::open(fixture.root.join("source/registry.json"))
            .expect("open source account registry");
    let profile = source_registry
        .create_managed("owner-a", "codex", "Source default")
        .expect("create source account");
    source_registry
        .set_default("owner-a", "codex", &profile.profile_id)
        .expect("select source default");
    let environment = source_registry
        .resolve_environment("owner-a", "codex", "default")
        .expect("resolve source account");
    fs::write(
        Path::new(&environment["CODEX_HOME"]).join("auth.json"),
        format!(r#"{{"token":"{secret}"}}"#),
    )
    .expect("write source provider credential");
    source_registry
        .export_managed_context_materialization("owner-a", "codex", "default")
        .expect("export source provider account")
}

fn all_export(
    fixture: &PackageFixture,
    payload: ManagedContextPackageProviderAccounts,
) -> ManagedContextPackageExportRequest {
    let mut export = fixture.export_request(ManagedContextPackageKernel::Empty);
    export.plan.development = ManagedContextDevelopmentSelection::Empty;
    export.development = ManagedContextPackageDevelopment::Empty;
    export.plan.provider_accounts = ManagedContextProviderAccountSelection::All;
    export.provider_accounts = payload;
    export
}

#[test]
fn an_all_plan_installs_the_login_the_source_resolved() {
    let _lock = crate::env_lock::lock();
    let fixture = PackageFixture::new("all-providers-install");
    let _provider_homes = FixtureProviderHomes::new(&fixture.root.join("target-home"));
    let materialization = source_codex_materialization(&fixture, "all-providers-secret");
    let exported = export_managed_context_package(all_export(
        &fixture,
        ManagedContextPackageProviderAccounts::Selected {
            materializations: vec![materialization],
            vault_logins: Vec::new(),
            vault_seed: None,
        },
    ))
    .expect("export an all-providers package");
    assert!(exported.provider_accounts_sha256.is_some());
    assert_eq!(
        exported.plan.provider_accounts,
        ManagedContextProviderAccountSelection::All,
        "the package binds the plan the target expects, not a list"
    );

    let target_registry =
        ProviderAccountProfileRegistry::open(fixture.root.join("target/registry.json"))
            .expect("open target account registry");
    target_registry
        .migrate_effective_defaults("owner-a", &fixture.root.join("target-home"))
        .expect("create target defaults");
    let receipt = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_all_providers".to_string(),
        package_path: exported.package_path.clone(),
        expected_package_sha256: exported.package_sha256.clone(),
        expected_binding: ManagedContextPackageBinding {
            plan: exported.plan.clone(),
            ..fixture.binding.clone()
        },
        development_destination_root: fixture.root.join("unused-development"),
        target_private_key: "unused-for-empty-context".to_string(),
        context_import_target: None,
        provider_account_target: Some(ManagedContextProviderAccountImportTarget {
            registry: target_registry.clone(),
            owner_user_id: "owner-a".to_string(),
            config: crate::config::DaemonConfig::for_tests(),
        }),
        git_credential_target: None,
        event_connection_target: None,
    })
    .expect("apply an all-providers package");
    let ManagedContextImportedProviderAccounts::Selected { accounts, .. } =
        &receipt.provider_accounts
    else {
        panic!("the installed logins became None")
    };
    assert_eq!(accounts.len(), 1);
    let environment = target_registry
        .resolve_environment("owner-a", "codex", "default")
        .expect("resolve installed account");
    assert_eq!(
        fs::read_to_string(Path::new(&environment["CODEX_HOME"]).join("auth.json"))
            .expect("read installed credential"),
        r#"{"token":"all-providers-secret"}"#
    );
    fixture.cleanup();
}

#[test]
fn an_all_plan_for_an_owner_with_no_login_installs_nothing_and_is_valid() {
    let _lock = crate::env_lock::lock();
    let fixture = PackageFixture::new("all-providers-none");
    let exported = export_managed_context_package(all_export(
        &fixture,
        ManagedContextPackageProviderAccounts::None,
    ))
    .expect("an owner without a login still exports");
    assert!(exported.provider_accounts_sha256.is_none());
    fixture.cleanup();
}

#[test]
fn an_all_plan_refuses_an_empty_or_repeating_payload() {
    let _lock = crate::env_lock::lock();
    let fixture = PackageFixture::new("all-providers-refused");
    let materialization = source_codex_materialization(&fixture, "all-providers-secret");
    assert!(export_managed_context_package(all_export(
        &fixture,
        ManagedContextPackageProviderAccounts::Selected {
            materializations: Vec::new(),
            vault_logins: Vec::new(),
            vault_seed: None,
        },
    ))
    .is_err());
    let mut repeated = fixture.export_request(ManagedContextPackageKernel::Empty);
    repeated.plan.development = ManagedContextDevelopmentSelection::Empty;
    repeated.development = ManagedContextPackageDevelopment::Empty;
    repeated.plan.provider_accounts = ManagedContextProviderAccountSelection::All;
    repeated.provider_accounts = ManagedContextPackageProviderAccounts::Selected {
        materializations: vec![materialization.clone(), materialization],
        vault_logins: Vec::new(),
        vault_seed: None,
    };
    assert!(export_managed_context_package(repeated).is_err());
    fixture.cleanup();
}
