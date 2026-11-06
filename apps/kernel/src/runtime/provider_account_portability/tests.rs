use super::*;
use crate::account_profile::ProviderAccountContextTransfer;
use crate::config::PersistedCloudRelayProfile;
use crate::local::{LocalDaemonRequest, ManagedEnvironmentProviderAccountSelection};
use crate::runtime::command::{KernelCallerKind, KernelCommandSource};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    root: PathBuf,
    profiles: ProviderAccountProfileRegistry,
    config: DaemonConfig,
    request: PreflightProviderAccountPortabilityRequest,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-portability-{}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        let profiles = ProviderAccountProfileRegistry::open(root.join("accounts.json")).unwrap();
        let profile = profiles
            .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "Synthetic")
            .unwrap();
        let environment = profiles
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &profile.profile_id,
            )
            .unwrap();
        fs::write(
            Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"synthetic-portability-only"}"#,
        )
        .unwrap();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "realm-a".into(),
            kernel_id: Some("kernel-a".into()),
            kernel_credential: Some("synthetic-kernel-credential".into()),
            cloud_session_token: None,
            api_url: "http://127.0.0.1:1".into(),
            ..Default::default()
        });
        let request = PreflightProviderAccountPortabilityRequest {
            provider_accounts: ManagedEnvironmentProviderAccounts::Selected {
                accounts: vec![ManagedEnvironmentProviderAccountSelection {
                    provider: "codex".into(),
                    account_profile: profile.profile_id,
                }],
            },
        };
        Self {
            root,
            profiles,
            config,
            request,
        }
    }
    fn command(&self) -> KernelCommand {
        KernelCommand::from_local_request(
            "portability",
            None,
            None,
            &LocalDaemonRequest::PreflightProviderAccountPortability(self.request.clone()),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn enrolled_owner_preflight_needs_no_human_token_and_returns_no_material() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    let response = execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({"ProviderAccountPortabilityPreflightPassed": {}})
    );
    command.source = KernelCommandSource::RelayClient;
    command.caller.caller_kind = KernelCallerKind::RemoteClient;
    command.caller.user_id = Some("owner".into());
    command.caller.realm_id = Some("realm-a".into());
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
    command.caller.realm_id = Some("foreign-realm".into());
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_err());
    command.caller.realm_id = Some("realm-a".into());
    command.caller.user_id = Some("shared-session-collaborator".into());
    command.session_id = Some("shared-session".into());
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_err());
    command.caller.user_id = Some("owner".into());
    command.caller.caller_kind = KernelCallerKind::RemoteKernel;
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_err());
}

#[test]
fn preflight_rechecks_portability_and_keeps_export_errors_private() {
    let mut fixture = Fixture::new();
    let command = fixture.command();
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
    let ManagedEnvironmentProviderAccounts::Selected { accounts } =
        &mut fixture.request.provider_accounts
    else {
        panic!()
    };
    accounts.push(ManagedEnvironmentProviderAccountSelection {
        provider: "claude".into(),
        account_profile: "missing".into(),
    });
    let failure = execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request,
    )
    .unwrap_err()
    .to_string();
    assert!(failure.contains("no transferable credentials"));
    assert!(!failure.contains("synthetic-portability-only"));
    assert!(!failure.contains(&fixture.root.to_string_lossy().to_string()));
    fixture.request.provider_accounts = ManagedEnvironmentProviderAccounts::None;
    assert!(execute_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
}

#[test]
fn vault_backed_login_is_not_offered_to_context_transfer() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let fixture = Fixture::new();
    let vault = fixture.root.join("vault.enc");
    let mut config = fixture.config.clone();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault,
        "test-only-vault-passphrase",
        crate::secret::VaultUnlockLease::Operation,
    )
    .unwrap();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let token_profile = fixture
        .profiles
        .create_managed(owner, "claude", "Vault token")
        .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        owner,
        "claude",
        &token_profile.profile_id,
        "sk-ant-oat01-TESTONLY-portability-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        false,
    )
    .unwrap();

    let mut profiles = fixture.profiles.list(owner, None).unwrap();
    mark_context_transfer(owner, &mut profiles).unwrap();

    let state = |provider: &str| {
        profiles
            .iter()
            .find(|profile| profile.provider == provider)
            .unwrap()
            .context_transfer
    };
    assert_eq!(
        state("claude"),
        ProviderAccountContextTransfer::VaultCredential
    );
    assert_eq!(state("codex"), ProviderAccountContextTransfer::Available);
    // The state is the preflight's own verdict, not a second opinion.
    assert!(preflight_selection(
        &config,
        &fixture.profiles,
        owner,
        &ManagedEnvironmentProviderAccounts::Selected {
            accounts: vec![ManagedEnvironmentProviderAccountSelection {
                provider: "claude".into(),
                account_profile: token_profile.profile_id,
            }],
        },
    )
    .is_err());
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
}
