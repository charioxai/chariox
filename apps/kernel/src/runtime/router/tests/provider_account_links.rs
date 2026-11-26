//! Linked credentials remain usable across transfer and profile lifecycle operations.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};
use crate::managed_context::package::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

struct Helpers;
impl Drop for Helpers {
    fn drop(&mut self) {
        crate::provider::shutdown_codex_account_endpoints();
    }
}

fn install_codex_fixture(root: &Path) {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let executable = root.join("codex");
    std::fs::write(&executable, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit; fi\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --listen ]; then shift; export CTXFIX_ACCOUNT_LISTEN=\"$1\"; break; fi; shift; done\nexport CTXFIX_ACCOUNT_LOG={}\nexec {} --exact local::provider_requests::catalog::codex_refresh_tests::account_endpoint_child --ignored --nocapture\n",
        quote(root.join("launches").to_str().unwrap()),
        quote(std::env::current_exe().unwrap().to_str().unwrap())
    )).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", executable);
}

async fn request(
    router: &CommandRouter,
    value: serde_json::Value,
) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
    let mut command = KernelCommand::from_local_request(
        format!("account-links-{}", rand::random::<u64>()),
        None,
        None,
        &request,
    );
    command.caller = router
        .local_terminal_caller(
            crate::runtime::command::KernelCommandSource::LocalIpc,
            "account-links",
        )
        .await;
    router.dispatch(command, request).await
}

async fn authenticated(router: &CommandRouter, id: &str) {
    let result = request(
        router,
        serde_json::json!({"GetProviderAuthStatus":{
            "provider":"codex", "account_profile":id
        }}),
    )
    .await;
    assert!(
        matches!(&result, Ok(LocalDaemonResponse::ProviderAuthStatus { status })
        if status.auth_state == "authenticated"),
        "linked account must launch and validate: {result:?}"
    );
}

#[tokio::test]
async fn transferred_directory_link_is_a_target_credential_root() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("linked-context-import");
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    std::env::set_var("HOME", root.path().join("home"));
    let _helpers = Helpers;
    install_codex_fixture(root.path());
    let config = DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
    let router = CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
        4,
    );
    let source =
        ProviderAccountProfileRegistry::open(root.path().join("source/registry.json")).unwrap();
    source
        .create_managed("local", "codex", "Unused default")
        .unwrap();
    let home = root.path().join("linked-native");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"tokens":{"access_token":"synthetic-accepted-login-import"}}"#,
    )
    .unwrap();
    let linked = source
        .link_existing("local", "codex", "Directory login", &home)
        .unwrap();
    let materialization = source
        .export_managed_context_materialization("local", "codex", &linked.profile_id)
        .unwrap();
    let plan = ManagedContextPlanBinding {
        context_id: "linked-import".into(),
        plan_digest: format!("sha256:{}", "1".repeat(64)),
        destination: None,
        kernel_context: ManagedContextKernelSelection::Empty,
        development: ManagedContextDevelopmentSelection::Empty,
        provider_accounts: ManagedContextProviderAccountSelection::Selected {
            accounts: vec![ManagedContextProviderAccount {
                provider: "codex".into(),
                account_profile: linked.profile_id.clone(),
            }],
        },
        git_credentials: ManagedContextGitCredentialSelection::None,
        event_connections: Default::default(),
    };
    let exported = export_managed_context_package(ManagedContextPackageExportRequest {
        plan: plan.clone(),
        target_environment_id: "linked-target".into(),
        source_kernel_id: "other-kernel".into(),
        source_key_thumbprint: "a".repeat(64),
        target_kernel_id: config.daemon_id.clone(),
        target_key_thumbprint: "b".repeat(64),
        development: ManagedContextPackageDevelopment::Empty,
        kernel_context: ManagedContextPackageKernel::Empty,
        provider_accounts: ManagedContextPackageProviderAccounts::Selected {
            materializations: vec![materialization],
            vault_logins: vec![],
            vault_seed: None,
        },
        git_credentials: ManagedContextPackageGitCredentials::None,
        event_connections: Default::default(),
        package_path: root.path().join("context.chariox"),
    })
    .unwrap();
    apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "linked-transfer".into(),
        package_path: exported.package_path,
        expected_package_sha256: exported.package_sha256,
        expected_binding: ManagedContextPackageBinding {
            plan,
            target_environment_id: "linked-target".into(),
            source_kernel_id: "other-kernel".into(),
            source_key_thumbprint: "a".repeat(64),
            target_kernel_id: config.daemon_id.clone(),
            target_key_thumbprint: "b".repeat(64),
        },
        development_destination_root: root.path().join("unused-development"),
        target_private_key: "unused-empty-context".into(),
        context_import_target: None,
        provider_account_target: Some(ManagedContextProviderAccountImportTarget {
            registry: router.provider_account_profiles.clone(),
            owner_user_id: "local".into(),
            config,
        }),
        git_credential_target: None,
        event_connection_target: None,
    })
    .unwrap();
    std::fs::remove_dir_all(home).unwrap();
    authenticated(&router, &linked.profile_id).await;
}

#[tokio::test]
async fn linked_account_in_use_blocks_source_registration_removal() {
    source_lifecycle(false).await;
}

#[tokio::test]
async fn linked_account_in_use_blocks_source_data_deletion() {
    source_lifecycle(true).await;
}

async fn source_lifecycle(delete: bool) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("linked-source-lifecycle");
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    std::env::set_var("HOME", root.path().join("home"));
    let _helpers = Helpers;
    install_codex_fixture(root.path());
    let config = DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
    let router = CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
        4,
    );
    let profiles = &router.provider_account_profiles;
    let source = profiles
        .create_managed("local", "codex", "Source login")
        .unwrap();
    let environment = profiles
        .resolve_environment("local", "codex", &source.profile_id)
        .unwrap();
    let auth = Path::new(&environment["CODEX_HOME"]).join("auth.json");
    let credential = r#"{"tokens":{"access_token":"synthetic-accepted-login-source"}}"#;
    std::fs::write(&auth, credential).unwrap();
    profiles
        .update_observation(
            "local",
            "codex",
            &source.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let copied = request(&router, serde_json::json!({"CopyProviderAccountToKernel":{
        "provider":"codex", "account_profile":source.profile_id, "operation_id":"source-link",
        "target_kernel_id":config.daemon_id,
        "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key), "replace":false
    }})).await.unwrap();
    let LocalDaemonResponse::ProviderAccountCopied { receipt } = copied else {
        panic!("expected local link")
    };
    let linked_id = receipt.account_profile;
    request(&router, serde_json::json!({"CreateSession":root.session_request().with_agent_defaults(
        crate::session::SessionAgentDefaults::new("codex").with_model("gpt-5.6").with_account_profile(&linked_id),
    )})).await.unwrap();
    authenticated(&router, &linked_id).await;
    let operation = if delete {
        serde_json::json!({"DeleteProviderAccountProfileData":{
            "provider":"codex", "account_profile":source.profile_id, "confirmation_profile_id":source.profile_id
        }})
    } else {
        serde_json::json!({"RemoveProviderAccountProfile":{
            "provider":"codex", "account_profile":source.profile_id
        }})
    };
    let result = request(&router, operation).await;
    let error = result.expect_err("a source cannot be removed while its linked account is in use");
    assert!(
        error.to_string().contains("before"),
        "the refusal must explain recovery: {error}"
    );
    assert_eq!(std::fs::read_to_string(auth).unwrap(), credential);
    profiles.get("local", "codex", &source.profile_id).unwrap();
    authenticated(&router, &linked_id).await;
}
