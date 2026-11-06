//! MP-08 / MP-09 / MP-10 / MP-11: a Vault-backed provider login travels in the
//! managed-context package, sealed, and lands in the new machine's own Vault.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, VaultLoginSeal};
use crate::config::{CredentialVaultBackend, DaemonConfig};
use crate::runtime::terminal_pairings::public_key_thumbprint;

const OWNER: &str = "owner-a";
const PASSPHRASE: &str = "vault-carry-test-passphrase";
const FIRST_TOKEN: &str = "sk-ant-oat01-FAKEVAULTCARRYFIRST-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnop";
const SECOND_TOKEN: &str = "sk-ant-oat01-FAKEVAULTCARRYSECOND-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmn";

/// One kernel: its own home (so its own credential registry), Vault, relay key and accounts.
struct Kernel {
    home: PathBuf,
    config: DaemonConfig,
    registry: ProviderAccountProfileRegistry,
}

impl Kernel {
    fn new(root: &Path, id: &str) -> Self {
        let home = root.join(id);
        fs::create_dir_all(&home).expect("create kernel home");
        let mut config = DaemonConfig::new(id, id, "owner");
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            home.join("vault.json").to_string_lossy().into_owned();
        let registry = ProviderAccountProfileRegistry::open(home.join("accounts.json"))
            .expect("open account registry");
        let kernel = Self {
            home,
            config,
            registry,
        };
        kernel.enter();
        crate::secret::unlock_chariox_encrypted_vault(
            Path::new(&kernel.config.user_config.credential_vault.path),
            PASSPHRASE,
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .expect("open the kernel's Vault");
        kernel
    }

    /// The credential registry is per kernel home; a test plays both kernels in one process.
    fn enter(&self) {
        std::env::set_var("CHARIOX_HOME", &self.home);
    }

    fn thumbprint(&self) -> String {
        public_key_thumbprint(&self.config.relay_public_key)
    }

    fn vault_login_profile(&self, label: &str, token: &str) -> String {
        self.enter();
        let profile = self
            .registry
            .create_managed(OWNER, "claude", label)
            .expect("create profile");
        crate::test_support::authenticate_provider_account(
            &self.registry,
            OWNER,
            "claude",
            &profile.profile_id,
        )
        .expect("authenticate profile");
        crate::provider::store_provider_account_credential(
            &self.config,
            OWNER,
            "claude",
            &profile.profile_id,
            token,
            false,
        )
        .expect("store Vault credential");
        profile.profile_id
    }

    fn vault_token(&self, profile_id: &str) -> Option<String> {
        self.enter();
        crate::provider::resolve_provider_account_credentials(
            &self.config,
            OWNER,
            "claude",
            profile_id,
        )
        .ok()?
        .iter()
        .next()
        .map(|(_, value)| value.to_string())
    }
}

struct Transfer {
    fixture: PackageFixture,
    source: Kernel,
    target: Kernel,
    _claude: crate::test_support::TestClaudeExecutable,
    _env: crate::env_lock::EnvGuard,
}

impl Transfer {
    fn new(label: &str) -> Self {
        let env = crate::env_lock::lock();
        let claude = crate::test_support::TestClaudeExecutable::new();
        let mut fixture = PackageFixture::new(label);
        let source = Kernel::new(&fixture.root, "source-kernel-1");
        let target = Kernel::new(&fixture.root, "target-kernel-1");
        fixture.binding.source_key_thumbprint = source.thumbprint();
        fixture.binding.target_key_thumbprint = target.thumbprint();
        Self {
            fixture,
            source,
            target,
            _claude: claude,
            _env: env,
        }
    }

    fn seal<'a>(&'a self, context_id: &'a str) -> VaultLoginSeal<'a> {
        VaultLoginSeal {
            source_private_key: &self.source.config.relay_private_key,
            target_public_key: &self.target.config.relay_public_key,
            context_id,
            source_kernel_id: &self.fixture.binding.source_kernel_id,
            target_kernel_id: &self.fixture.binding.target_kernel_id,
        }
    }

    /// The package the source kernel composes for `profile_ids`, as the outbound service does.
    fn export(&self, context_id: &str, profile_ids: &[&str]) -> ManagedContextPackageExportResult {
        self.source.enter();
        let mut ids = profile_ids.to_vec();
        ids.sort_unstable();
        let vault_logins = ids
            .iter()
            .map(|id| {
                self.source
                    .registry
                    .export_vault_login(
                        &self.source.config,
                        OWNER,
                        "claude",
                        id,
                        &self.seal(context_id),
                    )
                    .expect("export Vault login")
            })
            .collect();
        let mut request = self
            .fixture
            .export_request(ManagedContextPackageKernel::Empty);
        request.plan.context_id = context_id.to_string();
        request.plan.development = ManagedContextDevelopmentSelection::Empty;
        request.development = ManagedContextPackageDevelopment::Empty;
        request.plan.provider_accounts = ManagedContextProviderAccountSelection::Selected {
            accounts: ids
                .iter()
                .map(|id| ManagedContextProviderAccount {
                    provider: "claude".to_string(),
                    account_profile: (*id).to_string(),
                })
                .collect(),
        };
        request.provider_accounts = ManagedContextPackageProviderAccounts::Selected {
            materializations: Vec::new(),
            vault_logins,
        };
        request.package_path = self.fixture.root.join(format!("{context_id}.chariox"));
        export_managed_context_package(request).expect("export package")
    }

    fn apply(
        &self,
        exported: &ManagedContextPackageExportResult,
    ) -> Result<ManagedContextPackageImportReceipt, DaemonError> {
        self.target.enter();
        apply_managed_context_package(ManagedContextPackageApplicationRequest {
            transfer_id: format!("ctx_{}", exported.plan.context_id),
            package_path: exported.package_path.clone(),
            expected_package_sha256: exported.package_sha256.clone(),
            expected_binding: ManagedContextPackageBinding {
                plan: exported.plan.clone(),
                ..self.fixture.binding.clone()
            },
            development_destination_root: self.fixture.root.join("unused-development"),
            target_private_key: self.target.config.relay_private_key.clone(),
            context_import_target: None,
            provider_account_target: Some(self.target_accounts()),
            git_credential_target: None,
            event_connection_target: None,
        })
    }

    fn target_accounts(&self) -> ManagedContextProviderAccountImportTarget {
        ManagedContextProviderAccountImportTarget {
            registry: self.target.registry.clone(),
            owner_user_id: OWNER.to_string(),
            config: self.target.config.clone(),
        }
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        std::env::remove_var("CHARIOX_HOME");
        let _ = fs::remove_dir_all(&self.fixture.root);
    }
}

/// Every file under `root` as text, to prove a value is nowhere on disk but in a Vault.
fn plain_files_containing(root: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            stack.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else if metadata.is_file() {
            let bytes = fs::read(&path).unwrap_or_default();
            if bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn a_vault_login_is_exported_sealed_and_carries_no_plain_value() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-export");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-export", &[&profile_id]);

    assert!(exported.provider_accounts_sha256.is_some());
    let package = fs::read(&exported.package_path).expect("read package");
    assert!(
        !package
            .windows(FIRST_TOKEN.len())
            .any(|w| w == FIRST_TOKEN.as_bytes()),
        "the package digest covers ciphertext only"
    );
    let login = transfer
        .source
        .registry
        .export_vault_login(
            &transfer.source.config,
            OWNER,
            "claude",
            &profile_id,
            &transfer.seal("context-export"),
        )
        .expect("export Vault login");
    assert!(!format!("{login:?}").contains(FIRST_TOKEN));
    assert!(!serde_json::to_string(&login).unwrap().contains(FIRST_TOKEN));
    assert!(!format!("{:?}", exported).contains(FIRST_TOKEN));
}

#[test]
fn the_new_machine_writes_the_login_into_its_own_vault_and_registers_the_profile() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-import");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-import", &[&profile_id]);

    let receipt = transfer.apply(&exported).expect("apply package");

    let ManagedContextImportedProviderAccounts::Selected { accounts } = &receipt.provider_accounts
    else {
        panic!("selected provider account became None")
    };
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].profile_id, profile_id);
    transfer.target.enter();
    let imported = transfer
        .target
        .registry
        .get(OWNER, "claude", &profile_id)
        .expect("the profile is registered on the new machine");
    assert_eq!(imported.auth_state, ProviderAccountAuthState::Authenticated);
    assert!(
        imported.last_validated_at_ms.is_some(),
        "the official check left its marker"
    );
    assert_eq!(
        transfer.target.vault_token(&profile_id).as_deref(),
        Some(FIRST_TOKEN),
        "the token is in the machine's own Vault under the same handle"
    );
    assert_ne!(
        transfer.target.config.user_config.credential_vault.path,
        transfer.source.config.user_config.credential_vault.path
    );
    // A retry of the same package installs nothing twice.
    transfer
        .apply(&exported)
        .expect("repeat apply is idempotent");
}

#[test]
fn no_secret_value_reaches_the_receipt_the_package_or_any_plain_file() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-scan");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-scan", &[&profile_id]);
    let receipt = transfer.apply(&exported).expect("apply package");

    let receipt_text = format!("{receipt:?}{}", serde_json::to_string(&receipt).unwrap());
    assert!(!receipt_text.contains(FIRST_TOKEN));
    assert!(!format!("{:?}", transfer.target_accounts()).contains(FIRST_TOKEN));
    let leaks = plain_files_containing(&transfer.fixture.root, FIRST_TOKEN);
    let vault_files = [
        &transfer.source.config.user_config.credential_vault.path,
        &transfer.target.config.user_config.credential_vault.path,
    ];
    assert!(
        leaks
            .iter()
            .all(|path| vault_files.iter().any(|vault| Path::new(vault) == path)),
        "the token is plain only inside a Vault file: {leaks:?}"
    );
    assert!(
        leaks.is_empty(),
        "even the Vault files hold it encrypted: {leaks:?}"
    );
}

#[test]
fn a_conflicting_target_profile_is_refused_and_the_earlier_login_rolls_back() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-conflict");
    let first = transfer.source.vault_login_profile("First", FIRST_TOKEN);
    let second = transfer.source.vault_login_profile("Second", SECOND_TOKEN);
    // The second login already reached the new machine through another context.
    transfer
        .apply(&transfer.export("context-earlier", &[&second]))
        .expect("earlier context installs the second login");

    let both = transfer.export("context-both", &[&first, &second]);
    let error = transfer
        .apply(&both)
        .expect_err("the second login conflicts");

    assert!(
        error.to_string().contains("refusing to replace"),
        "typed refusal: {error}"
    );
    transfer.target.enter();
    assert!(
        transfer
            .target
            .registry
            .get(OWNER, "claude", &first)
            .is_err(),
        "the login installed before the conflict is rolled back"
    );
    assert_eq!(
        transfer.target.vault_token(&first),
        None,
        "and so is its Vault value"
    );
    assert_eq!(
        transfer.target.vault_token(&second).as_deref(),
        Some(SECOND_TOKEN),
        "the earlier context's login is untouched"
    );
}

#[test]
fn a_seal_for_another_machine_or_context_is_refused() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-binding");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-binding", &[&profile_id]);
    let mut other_target = transfer.fixture.binding.clone();
    other_target.target_kernel_id = "another-kernel".to_string();
    transfer.target.enter();
    let error = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_binding".to_string(),
        package_path: exported.package_path.clone(),
        expected_package_sha256: exported.package_sha256.clone(),
        expected_binding: ManagedContextPackageBinding {
            plan: exported.plan.clone(),
            ..other_target
        },
        development_destination_root: transfer.fixture.root.join("unused-development"),
        target_private_key: transfer.target.config.relay_private_key.clone(),
        context_import_target: None,
        provider_account_target: Some(transfer.target_accounts()),
        git_credential_target: None,
        event_connection_target: None,
    })
    .expect_err("a seal made for kernel-1 is not opened for another-kernel");
    assert!(!error.to_string().contains(FIRST_TOKEN));
    assert!(transfer
        .target
        .registry
        .get(OWNER, "claude", &profile_id)
        .is_err());
    assert_eq!(transfer.target.vault_token(&profile_id), None);
}
