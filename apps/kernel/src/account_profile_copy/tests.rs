use super::*;

fn binding(operation: &str) -> ProviderAccountCopyBinding {
    ProviderAccountCopyBinding {
        operation_id: operation.into(),
        provider: "opencode".into(),
        source_profile_id: "profile-1".into(),
        source_kernel_id: "source".into(),
        source_key_thumbprint: "a".repeat(64),
        target_kernel_id: "target".into(),
        target_key_thumbprint: "b".repeat(64),
        replace: false,
    }
}

fn code_of(error: DaemonError) -> &'static str {
    match error {
        DaemonError::ProviderAccountCopy { code, .. } => code,
        other => panic!("expected an account copy refusal, got {other}"),
    }
}

#[test]
fn retrying_an_operation_with_only_replace_changed_names_that_difference() {
    let recorded = binding("copy-replace");
    let mut retry = recorded.clone();
    retry.replace = true;
    assert_eq!(
        code_of(recorded_operation_conflict(&recorded, &retry)),
        "provider_account_copy_replace_differs"
    );
    // Any other difference stays the generic conflict, replace changed or not.
    retry.source_profile_id = "profile-2".into();
    assert_eq!(
        code_of(recorded_operation_conflict(&recorded, &retry)),
        "provider_account_copy_conflict"
    );
}

#[test]
fn a_receiver_without_the_provider_cli_says_so() {
    for adapter in ["claude", "codex", "opencode"] {
        let missing = DaemonError::ProviderExecutableNotFound {
            adapter_key: adapter.into(),
            executable: adapter.into(),
        };
        assert_eq!(
            code_of(native::cli_missing_or_failed(missing)),
            "provider_account_copy_cli_missing"
        );
    }
    let other = DaemonError::LocalTransport {
        operation: "resolve provider",
        message: "unreadable".into(),
    };
    assert_eq!(
        code_of(native::cli_missing_or_failed(other)),
        "provider_account_copy_verification_failed"
    );
}

#[test]
fn only_a_locked_vault_is_reported_as_locked() {
    let locked = DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: "locked".into(),
    };
    assert_eq!(
        code_of(crate::provider_account_copy::credential_read_failure(
            &locked
        )),
        "provider_account_copy_vault_locked"
    );
    let other = DaemonError::LocalTransport {
        operation: "read credential",
        message: "record missing".into(),
    };
    assert_eq!(
        code_of(crate::provider_account_copy::credential_read_failure(
            &other
        )),
        "provider_account_copy_credential_unreadable"
    );
}

fn opencode_profile(
    root: &crate::test_support::TestWorktree,
    login: &[u8],
) -> (ProviderAccountProfileRegistry, String) {
    let registry = ProviderAccountProfileRegistry::open(root.path().join("registry.json")).unwrap();
    let profile = registry
        .create_managed("owner", "opencode", "Source")
        .unwrap();
    let environment = registry
        .resolve_environment("owner", "opencode", &profile.profile_id)
        .unwrap();
    let auth = Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    fs::create_dir_all(auth.parent().unwrap()).unwrap();
    fs::write(auth, login).unwrap();
    (registry, profile.profile_id)
}

#[test]
fn an_opencode_login_exports_as_the_one_credential_string() {
    let root = crate::test_support::TestWorktree::new("opencode-copy-export");
    let login = br#"{"openai":{"type":"api","key":"fixture-export-login"}}"#;
    let (registry, id) = opencode_profile(&root, login);
    let config = crate::config::DaemonConfig::for_tests();
    let credential = registry
        .export_account_copy_credential(&config, "owner", "opencode", &id)
        .unwrap();
    assert_eq!(credential.value().as_bytes(), login);
}

#[test]
fn an_opencode_login_over_the_copy_limit_is_refused_by_name() {
    let root = crate::test_support::TestWorktree::new("opencode-copy-too-large");
    let mut login = br#"{"openai":{"type":"api","key":""#.to_vec();
    login.extend(std::iter::repeat_n(b'k', MAX_COPY_CREDENTIAL_BYTES));
    login.extend_from_slice(br#""}}"#);
    let (registry, id) = opencode_profile(&root, &login);
    let config = crate::config::DaemonConfig::for_tests();
    let error = registry
        .export_account_copy_credential(&config, "owner", "opencode", &id)
        .unwrap_err();
    assert_eq!(code_of(error), "provider_account_copy_credential_too_large");
}

#[test]
fn a_vault_that_will_not_take_the_login_is_locked_only_when_it_is() {
    let locked = DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: "locked".into(),
    };
    assert_eq!(
        code_of(vault_store_failure(&locked)),
        "provider_account_copy_vault_locked"
    );
    let damaged = DaemonError::LocalTransport {
        operation: "write credential",
        message: "vault file is damaged".into(),
    };
    assert_eq!(
        code_of(vault_store_failure(&damaged)),
        "provider_account_copy_vault_unreadable"
    );
}
