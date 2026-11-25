use super::*;
use crate::account_profile::ProviderAccountContextTransfer;
use crate::config::PersistedCloudRelayProfile;
use crate::local::{LocalDaemonRequest, ProviderAccountTransferProfile};
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
            provider_accounts: ProviderAccountTransferSelection::Selected {
                accounts: vec![ProviderAccountTransferProfile {
                    provider: "codex".into(),
                    account_profile: profile.profile_id,
                }],
            },
            kernel_context: crate::managed_context::package::ManagedContextKernelSelection::Empty,
            launch_id: None,
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
    let carries_vault_login = check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request,
    )
    .unwrap();
    assert!(!carries_vault_login, "a file login needs no Vault");
    command.source = KernelCommandSource::RelayClient;
    command.caller.caller_kind = KernelCallerKind::RemoteClient;
    command.caller.user_id = Some("owner".into());
    command.caller.realm_id = Some("realm-a".into());
    assert!(check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
    command.caller.realm_id = Some("foreign-realm".into());
    assert!(check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_err());
    command.caller.realm_id = Some("realm-a".into());
    command.caller.user_id = Some("shared-session-collaborator".into());
    command.session_id = Some("shared-session".into());
    assert!(check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_err());
    command.caller.user_id = Some("owner".into());
    command.caller.caller_kind = KernelCallerKind::RemoteKernel;
    assert!(check_preflight(
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
    assert!(check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
    let ProviderAccountTransferSelection::Selected { accounts } =
        &mut fixture.request.provider_accounts
    else {
        panic!()
    };
    accounts.push(ProviderAccountTransferProfile {
        provider: "claude".into(),
        account_profile: "missing".into(),
    });
    let failure = check_preflight(
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
    fixture.request.provider_accounts = ProviderAccountTransferSelection::None;
    assert!(check_preflight(
        &fixture.config,
        &fixture.profiles,
        &command,
        &fixture.request
    )
    .is_ok());
}

#[test]
fn vault_backed_login_is_accepted_by_preflight_and_offered_to_context_transfer() {
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
    let selection = ProviderAccountTransferSelection::Selected {
        accounts: vec![ProviderAccountTransferProfile {
            provider: "claude".into(),
            account_profile: token_profile.profile_id,
        }],
    };

    let state = |config: &DaemonConfig| {
        let mut profiles = fixture.profiles.list(owner, None).unwrap();
        mark_context_transfer(owner, &config.user_config.credential_vault, &mut profiles).unwrap();
        profiles
            .iter()
            .find(|profile| profile.provider == "claude")
            .unwrap()
            .context_transfer
    };
    // A Chariox Vault can be sealed for the new machine: the login is offered and preflight
    // passes, asking the caller to find the Vault open.
    assert_eq!(state(&config), ProviderAccountContextTransfer::Available);
    assert!(preflight_selection(&config, &fixture.profiles, owner, &selection).unwrap());
    // A secret kept outside a Chariox Vault cannot be sealed: the state is the preflight's own verdict.
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    assert_eq!(state(&config), ProviderAccountContextTransfer::Unavailable);
    assert!(preflight_selection(&config, &fixture.profiles, owner, &selection).is_err());
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
}

// Row 582: `all` is every signed-in login the owner holds, every provider and profile, none picked.
#[test]
fn all_is_every_signed_in_login_of_every_provider_in_a_fixed_order() {
    let fixture = Fixture::new();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let sign_in = |provider: &str, profile: &str| {
        fixture
            .profiles
            .update_observation(
                owner,
                provider,
                profile,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
    };
    let codex = fixture
        .profiles
        .list(owner, Some("codex"))
        .unwrap()
        .remove(0);
    let second_codex = fixture
        .profiles
        .create_managed(owner, "codex", "Second")
        .unwrap();
    let claude = fixture
        .profiles
        .create_managed(owner, "claude", "Claude")
        .unwrap();
    let signed_out = fixture
        .profiles
        .create_managed(owner, "opencode", "Signed out")
        .unwrap();
    assert!(
        all_owner_accounts(&fixture.profiles, owner)
            .unwrap()
            .is_empty(),
        "a profile that was never signed in has nothing to move"
    );
    sign_in("codex", &codex.profile_id);
    sign_in("codex", &second_codex.profile_id);
    sign_in("claude", &claude.profile_id);
    let all: Vec<_> = all_owner_accounts(&fixture.profiles, owner)
        .unwrap()
        .into_iter()
        .map(|account| (account.provider, account.account_profile))
        .collect();
    let mut expected = vec![
        ("claude".to_string(), claude.profile_id.clone()),
        ("codex".to_string(), codex.profile_id.clone()),
        ("codex".to_string(), second_codex.profile_id.clone()),
    ];
    expected.sort();
    assert_eq!(
        all, expected,
        "every profile of every signed-in provider, sorted"
    );
    assert!(!all
        .iter()
        .any(|(_, profile)| profile == &signed_out.profile_id));
}

#[test]
fn preflight_of_all_names_the_login_that_cannot_travel_and_the_one_step() {
    let fixture = Fixture::new();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let codex = fixture
        .profiles
        .list(owner, Some("codex"))
        .unwrap()
        .remove(0);
    fixture
        .profiles
        .update_observation(
            owner,
            "codex",
            &codex.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    // The fixture's Codex login carries its auth file: `all` passes and reads no Vault.
    assert!(!preflight_selection(
        &fixture.config,
        &fixture.profiles,
        owner,
        &ProviderAccountTransferSelection::All
    )
    .unwrap());
    // A signed-in login with nothing to move is refused by name, never skipped.
    let bare = fixture
        .profiles
        .create_managed(owner, "codex", "Bare")
        .unwrap();
    fixture
        .profiles
        .update_observation(
            owner,
            "codex",
            &bare.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let refusal = preflight_selection(
        &fixture.config,
        &fixture.profiles,
        owner,
        &ProviderAccountTransferSelection::All,
    )
    .unwrap_err()
    .to_string();
    assert!(
        refusal.contains("cannot be copied as it is") && refusal.contains("Sign in to it again")
    );
    assert!(refusal.contains(&bare.profile_id));
}
