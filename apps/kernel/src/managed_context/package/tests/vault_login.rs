//! MP-08 / MP-09 / MP-10 / MP-11: a Vault-backed provider login travels in the
//! managed-context package, sealed, and lands in the new machine's own Vault.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, VaultLoginOpen, VaultLoginSeal};
use crate::config::{CredentialVaultBackend, DaemonConfig};
use crate::provider_vault_login::VaultLoginMaterialization;
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

    /// A new machine whose configured Vault nobody has created or opened yet.
    fn with_unopened_vault(root: &Path, id: &str) -> Self {
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

    fn with_unopened_target(label: &str) -> Self {
        let env = crate::env_lock::lock();
        let claude = crate::test_support::TestClaudeExecutable::new();
        let mut fixture = PackageFixture::new(label);
        let source = Kernel::new(&fixture.root, "source-kernel-1");
        let target = Kernel::with_unopened_vault(&fixture.root, "target-kernel-1");
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
        self.export_with_kernel(context_id, profile_ids, ManagedContextPackageKernel::Empty)
    }

    /// The source kernel's own Vault, sealed for the target as a managed kernel context carries it.
    fn kernel_context_with_vault(&self, context_id: &str) -> ManagedContextPackageKernel {
        self.source.enter();
        let vault = crate::secret::export_transferred_vault_snapshot(
            &self.source.config.user_config.credential_vault.path,
            context_id,
            &self.fixture.binding.source_kernel_id,
            &self.source.config.relay_private_key,
            &self.fixture.binding.target_kernel_id,
            &self.target.config.relay_public_key,
        )
        .expect("export the source Vault");
        let payload = KernelContextPayload {
            schema_version: crate::managed_context::kernel::KERNEL_CONTEXT_SCHEMA_VERSION,
            context_id: context_id.to_string(),
            source_kernel_id: self.fixture.binding.source_kernel_id.clone(),
            source_key_thumbprint: self.source.thumbprint(),
            target_kernel_id: self.fixture.binding.target_kernel_id.clone(),
            target_key_thumbprint: self.target.thumbprint(),
            compatibility: KernelContextCompatibility {
                source_kernel_version: "0.1.0".to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version:
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions: Vec::new(),
            dependencies: Vec::new(),
            vault: Some(vault),
        };
        let snapshot_sha256 = sha256_bytes(&serde_json::to_vec(&payload).unwrap());
        ManagedContextPackageKernel::FromKernel(Box::new(KernelContextSnapshot {
            payload,
            snapshot_sha256,
        }))
    }

    fn export_with_kernel(
        &self,
        context_id: &str,
        profile_ids: &[&str],
        kernel_context: ManagedContextPackageKernel,
    ) -> ManagedContextPackageExportResult {
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
        let mut request = self.fixture.export_request(kernel_context);
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

    let ManagedContextImportedProviderAccounts::Selected { accounts, .. } =
        &receipt.provider_accounts
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

/// The official check of this test machine waits until two checks run at once, so two imports
/// of the same profile are both past their first look at the registry before either writes.
fn make_provider_checks_overlap() {
    let executable = PathBuf::from(std::env::var_os("CHARIOX_CLAUDE_BIN").expect("test claude"));
    fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = -p ]; then
  dir=$(dirname "$0")
  touch "$dir/check.$$"
  waited=0
  while [ "$(ls "$dir"/check.* 2>/dev/null | wc -l)" -lt 2 ] && [ "$waited" -lt 50 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{"output_tokens":7},"result":"CHARIOX_AUTH_OK"}'
  exit 0
fi
while IFS= read -r line; do :; done
"#,
    )
    .expect("rewrite the test claude executable");
}

#[test]
fn two_imports_of_one_login_never_delete_the_winning_credential() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-race");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let export = |context_id: &str| {
        transfer.source.enter();
        transfer
            .source
            .registry
            .export_vault_login(
                &transfer.source.config,
                OWNER,
                "claude",
                &profile_id,
                &transfer.seal(context_id),
            )
            .expect("export Vault login")
    };
    let first = export("context-race-a");
    crate::provider::store_provider_account_credential(
        &transfer.source.config,
        OWNER,
        "claude",
        &profile_id,
        SECOND_TOKEN,
        true,
    )
    .expect("rotate the source login");
    let second = export("context-race-b");
    make_provider_checks_overlap();
    transfer.target.enter();
    let binding = &transfer.fixture.binding;
    let install = |context_id: &str, login: &VaultLoginMaterialization| {
        transfer.target.registry.install_vault_login_replica(
            &transfer.target.config,
            OWNER,
            context_id,
            &format!("package-{context_id}"),
            &VaultLoginOpen {
                target_private_key: &transfer.target.config.relay_private_key,
                source_key_thumbprint: &binding.source_key_thumbprint,
                source_kernel_id: &binding.source_kernel_id,
                target_kernel_id: &binding.target_kernel_id,
            },
            login,
        )
    };

    let (result_a, result_b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| install("context-race-a", &first));
        let b = scope.spawn(|| install("context-race-b", &second));
        (a.join().expect("import a"), b.join().expect("import b"))
    });

    let outcomes = [
        ("context-race-a", FIRST_TOKEN, result_a),
        ("context-race-b", SECOND_TOKEN, result_b),
    ];
    let winners: Vec<_> = outcomes
        .iter()
        .filter(|(_, _, result)| result.is_ok())
        .collect();
    assert_eq!(
        winners.len(),
        1,
        "one import wins the profile: {outcomes:?}"
    );
    for (context_id, _, result) in &outcomes {
        if let Err(error) = result {
            let message = error.to_string();
            assert!(
                message.contains("in progress") || message.contains("refusing to replace"),
                "{context_id} gets a typed refusal: {message}"
            );
        }
    }
    let (context_id, token, _) = winners[0];
    transfer.target.enter();
    let stored = transfer
        .target
        .registry
        .get(OWNER, "claude", &profile_id)
        .expect("the winning import published the profile");
    assert_eq!(stored.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(
        transfer.target.vault_token(&profile_id).as_deref(),
        Some(*token),
        "the winning login ({context_id}) still resolves from the Vault"
    );
}

/// The refusal every client reads: the whole import is refused, before anything is written.
fn assert_target_vault_locked(error: &DaemonError) {
    match error {
        DaemonError::ManagedContext {
            code, retryable, ..
        } => {
            assert_eq!(*code, "target_vault_locked", "{error:?}");
            assert!(*retryable, "the owner retries after unlocking: {error:?}");
        }
        other => panic!("expected the target_vault_locked refusal: {other:?}"),
    }
}

#[test]
fn a_new_machine_without_an_open_vault_refuses_the_whole_import_and_writes_nothing() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::with_unopened_target("vault-login-unopened");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-unopened", &[&profile_id]);
    let target_vault = PathBuf::from(&transfer.target.config.user_config.credential_vault.path);
    assert!(!target_vault.exists(), "the new machine has no Vault yet");

    let error = transfer
        .apply(&exported)
        .expect_err("a package that carries a Vault login is refused while the Vault is closed");

    assert_target_vault_locked(&error);
    assert!(
        transfer
            .target
            .registry
            .get(OWNER, "claude", &profile_id)
            .is_err(),
        "no profile points at a value that was never stored"
    );
    assert!(
        !target_vault.exists(),
        "the refusal neither creates nor half-installs a Vault"
    );
    assert!(!format!("{error:?}").contains(FIRST_TOKEN));
    assert!(plain_files_containing(&transfer.fixture.root, FIRST_TOKEN).is_empty());
}

#[test]
fn the_retry_after_unlocking_the_receiving_vault_completes_the_same_package() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::with_unopened_target("vault-login-locked-retry");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-locked-retry", &[&profile_id]);

    let error = transfer
        .apply(&exported)
        .expect_err("the closed Vault refuses the whole import");
    assert_target_vault_locked(&error);
    assert!(transfer
        .target
        .registry
        .get(OWNER, "claude", &profile_id)
        .is_err());

    transfer.target.enter();
    crate::secret::unlock_chariox_encrypted_vault(
        Path::new(&transfer.target.config.user_config.credential_vault.path),
        PASSPHRASE,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .expect("the owner unlocks the receiving Vault");
    let receipt = transfer
        .apply(&exported)
        .expect("the same package imports once the Vault is open");
    let ManagedContextImportedProviderAccounts::Selected { accounts, .. } =
        &receipt.provider_accounts
    else {
        panic!("the selection stays recorded");
    };
    assert_eq!(accounts.len(), 1, "the login is carried on the retry");
    assert_eq!(
        transfer.target.vault_token(&profile_id).as_deref(),
        Some(FIRST_TOKEN)
    );
}

#[test]
fn a_package_without_vault_information_imports_while_the_vault_is_closed() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::with_unopened_target("vault-login-none");
    let mut request = transfer
        .fixture
        .export_request(ManagedContextPackageKernel::Empty);
    request.plan.context_id = "context-none".to_string();
    request.plan.development = ManagedContextDevelopmentSelection::Empty;
    request.development = ManagedContextPackageDevelopment::Empty;
    request.package_path = transfer.fixture.root.join("context-none.chariox");
    let exported = export_managed_context_package(request).expect("export package");

    transfer
        .apply(&exported)
        .expect("nothing needs the receiving Vault");
}

#[test]
fn a_login_that_reaches_the_vault_is_never_listed_as_left_out() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-not-omitted");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export("context-not-omitted", &[&profile_id]);

    let receipt = transfer.apply(&exported).expect("apply the package");

    let serialized = serde_json::to_value(&receipt).expect("serialize the receipt");
    assert_eq!(
        serialized.pointer("/providerAccounts/accounts/0/profileId"),
        Some(&serde_json::json!(profile_id))
    );
    assert!(
        serialized.pointer("/providerAccounts/omitted").is_none(),
        "a full import keeps its shape: {serialized}"
    );
}

/// A Path 1 target whose configured Vault is where the package's transferred Vault lands.
fn managed_target_without_vault(label: &str) -> Transfer {
    let transfer = Transfer::with_unopened_target(label);
    let home = transfer.fixture.root.join("target-home");
    fs::create_dir_all(home.join(".chariox")).expect("create target home");
    std::env::set_var("HOME", &home);
    std::env::set_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "path1");
    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::set_var(
        "CHARIOX_MANAGED_VAULT_PATH",
        &transfer.target.config.user_config.credential_vault.path,
    );
    transfer
}

#[test]
fn a_transferred_vault_is_installed_before_the_login_is_written_into_it() {
    crate::test_support::isolated_env_test!();
    let transfer = managed_target_without_vault("vault-login-transferred");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let exported = transfer.export_with_kernel(
        "context-transferred",
        &[&profile_id],
        transfer.kernel_context_with_vault("context-transferred"),
    );

    let receipt = transfer
        .apply(&exported)
        .expect("the import completes on a machine whose Vault arrives in the package");

    let ManagedContextImportedProviderAccounts::Selected { accounts, .. } =
        &receipt.provider_accounts
    else {
        panic!("the selection stays recorded");
    };
    assert_eq!(accounts.len(), 1, "the login is carried");
    let imported = transfer
        .target
        .registry
        .get(OWNER, "claude", &profile_id)
        .expect("the profile is registered on the new machine");
    assert_eq!(imported.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(
        transfer.target.vault_token(&profile_id).as_deref(),
        Some(FIRST_TOKEN),
        "the token reads back from the installed Vault"
    );
    assert!(!format!("{receipt:?}").contains(FIRST_TOKEN));
}

#[test]
fn a_failure_after_the_login_import_leaves_neither_the_login_nor_the_transferred_vault() {
    crate::test_support::isolated_env_test!();
    let transfer = managed_target_without_vault("vault-login-transferred-rollback");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let context_id = "context-transferred-rollback";
    let kernel_context = transfer.kernel_context_with_vault(context_id);
    transfer.source.enter();
    let vault_logins = vec![transfer
        .source
        .registry
        .export_vault_login(
            &transfer.source.config,
            OWNER,
            "claude",
            &profile_id,
            &transfer.seal(context_id),
        )
        .expect("export Vault login")];
    // The development archive is not an archive, so the last component fails.
    let mut request = transfer.fixture.export_request(kernel_context);
    request.plan.context_id = context_id.to_string();
    request.plan.provider_accounts = ManagedContextProviderAccountSelection::Selected {
        accounts: vec![ManagedContextProviderAccount {
            provider: "claude".to_string(),
            account_profile: profile_id.clone(),
        }],
    };
    request.provider_accounts = ManagedContextPackageProviderAccounts::Selected {
        materializations: Vec::new(),
        vault_logins,
    };
    request.package_path = transfer.fixture.root.join(format!("{context_id}.chariox"));
    let exported = export_managed_context_package(request).expect("export package");

    let error = transfer
        .apply(&exported)
        .expect_err("a development archive that does not open fails the import");
    let message = format!("{error:?}");
    assert!(
        message.contains("development") && !message.contains("schema version"),
        "the import must fail at the development step, after the login was written: {message}"
    );

    assert!(
        transfer
            .target
            .registry
            .get(OWNER, "claude", &profile_id)
            .is_err(),
        "the login's profile is rolled back"
    );
    let target_vault = PathBuf::from(&transfer.target.config.user_config.credential_vault.path);
    assert!(!target_vault.exists(), "the transferred Vault is removed");
    assert_eq!(
        fs::read_dir(target_vault.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("vault"))
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>(),
        Vec::<std::ffi::OsString>::new(),
        "no Vault file or key envelope is left half-installed"
    );
    assert!(
        !transfer
            .fixture
            .root
            .join("target-home/.chariox/managed-context/kernel-context")
            .exists(),
        "the kernel context is rolled back"
    );
}

#[test]
fn a_rollback_on_a_locked_vault_keeps_the_profile_so_a_later_rollback_clears_the_login() {
    crate::test_support::isolated_env_test!();
    let transfer = Transfer::new("vault-login-locked-rollback");
    let profile_id = transfer
        .source
        .vault_login_profile("Vault token", FIRST_TOKEN);
    let receipt = transfer
        .apply(&transfer.export("context-locked", &[&profile_id]))
        .expect("apply package");
    let vault = Path::new(&transfer.target.config.user_config.credential_vault.path);
    // The new machine's Vault lapsed (restart, unlock lease) before Cloud rejected the transfer.
    crate::secret::lock_chariox_encrypted_vault(vault).expect("lock the target Vault");

    let rollback = || {
        transfer.target.enter();
        rollback_managed_context_package_application(
            &receipt,
            &transfer.target.config.relay_private_key,
            Some(&transfer.target_accounts()),
            None,
            None,
        )
    };
    rollback().expect_err("a locked Vault cannot remove the login");
    crate::secret::unlock_chariox_encrypted_vault(
        vault,
        PASSPHRASE,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .expect("reopen the target Vault");
    rollback().expect("the retried rollback succeeds");

    assert_eq!(
        transfer.target.vault_token(&profile_id),
        None,
        "the retried rollback removes the login from the rejected machine's Vault"
    );
    assert!(
        transfer
            .target
            .registry
            .get(OWNER, "claude", &profile_id)
            .is_err(),
        "and the profile"
    );
}
