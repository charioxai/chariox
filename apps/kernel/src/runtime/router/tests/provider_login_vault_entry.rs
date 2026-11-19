//! A Vault entry typed into an opencode API-key login by the kernel, through
//! the public requests and a fake official login program. The dummy key below
//! must never show in a status, an answer or a log.
use super::*;
use crate::local::{
    KernelConnectionClass, LocalDaemonResponse, ProviderLoginProcessState,
    SendProviderLoginVaultEntryRequest,
};
use base64::Engine as _;
use std::os::unix::fs::PermissionsExt;

const KEY: &str = "ocv-dummy-key-7d2f91c0aa";
const OPENCODE_SITE: &str = "https://opencode.ai";

/// A login that, unlike opencode, echoes what it is typed, so the test shows
/// the kernel masks the screen itself.
const FAKE_OPENCODE: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then echo 1.19.0; exit 0; fi
if [ "$1" = auth ] && [ "$2" = list ]; then exit 0; fi
[ "$1" = auth ] && [ "$2" = login ] || exit 90
printf 'Enter your API key\n'
IFS= read -r key
mkdir -p "$XDG_DATA_HOME/opencode"
printf '{"opencode-go":{"type":"api","key":"%s"}}' "$key" > "$XDG_DATA_HOME/opencode/auth.json"
printf '%s' "$key" > "$(dirname "$0")/received"
printf 'Login successful\n'
"#;

/// A login that echoes the key in three fragments and waits between them for
/// the test to say go, so a status poll lands while only part of it is out.
const FRAGMENTED_OPENCODE: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then echo 1.19.0; exit 0; fi
dir=$(dirname "$0")
if [ "$1" = auth ] && [ "$2" = list ]; then [ -e "$dir/listfail" ] && exit 5; exit 0; fi
[ "$1" = auth ] && [ "$2" = login ] || exit 90
stty -echo 2>/dev/null
printf 'Enter your API key\n'
IFS= read -r key
printf 'echo: %s' "$(printf '%s' "$key" | cut -c1-10)"
: > "$dir/sent1"
n=0; while [ ! -e "$dir/go2" ]; do n=$((n+1)); [ $n -lt 400 ] || exit 91; sleep 0.05; done
printf '%s' "$(printf '%s' "$key" | cut -c11-17)"
: > "$dir/sent2"
n=0; while [ ! -e "$dir/go3" ]; do n=$((n+1)); [ $n -lt 400 ] || exit 91; sleep 0.05; done
[ -e "$dir/truncate" ] && exit 0
printf '%s\n' "$(printf '%s' "$key" | cut -c18-)"
printf 'Login successful\n'
"#;

struct Fixture {
    router: CommandRouter,
    root: std::path::PathBuf,
    profile: String,
    environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (name, value) in &self.environment {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(backend: crate::config::CredentialVaultBackend) -> Self {
        Self::with_program(backend, FAKE_OPENCODE)
    }

    fn with_program(backend: crate::config::CredentialVaultBackend, program: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-ocvault-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let names = [
            "HOME",
            "CHARIOX_HOME",
            "CHARIOX_LOG_DIR",
            "CHARIOX_OPENCODE_BIN",
            "CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS",
            "CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT",
        ];
        let environment = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("CHARIOX_HOME", root.join("state"));
        std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        let binary = root.join("opencode");
        std::fs::write(&binary, program).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_OPENCODE_BIN", &binary);
        std::env::set_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS", "200");
        let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
        config.user_config.credential_vault.backend = backend;
        if backend == crate::config::CredentialVaultBackend::CharioxEncrypted {
            // The Vault needs a folder of its own: agent confinement hides that folder.
            let vault = root.join("vault/vault.enc");
            std::fs::create_dir_all(vault.parent().unwrap()).unwrap();
            config.user_config.credential_vault.path = vault.display().to_string();
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, "correct horse battery")
                .unwrap();
            crate::secret::unlock_chariox_encrypted_vault(
                &vault,
                "correct horse battery",
                crate::secret::VaultUnlockLease::TtlMinutes(30),
            )
            .unwrap();
        }
        let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
        let router = CommandRouter::with_interactive_capacity(app, 1);
        let profile = router
            .provider_account_profiles
            .create_managed("local", "opencode", "fixture")
            .unwrap()
            .profile_id;
        Self {
            router,
            root,
            profile,
            environment,
        }
    }

    async fn send(&self, request: LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        let mut command = KernelCommand::from_local_request(
            format!("ocvault-{}", rand::random::<u64>()),
            None,
            None,
            &request,
        );
        command.caller.connection_class = Some(KernelConnectionClass::Terminal);
        self.router.dispatch(command, request).await
    }

    async fn owner_entry(&self, site: &str) -> String {
        let created = self
            .send(LocalDaemonRequest::OwnerVault(
                crate::local::OwnerVaultRequest::Create {
                    name: "OpenCode key".into(),
                    kind: "API key".into(),
                    use_binding: crate::config::VaultUseBinding {
                        target: crate::config::VaultUseTarget::site(site).unwrap(),
                        value_kind: Default::default(),
                    },
                    value: crate::local::VaultPassphrase::new(KEY),
                },
            ))
            .await
            .unwrap();
        match created {
            LocalDaemonResponse::OwnerVault {
                result: crate::local::OwnerVaultResult::Saved { id },
            } => id,
            other => panic!("owner creation failed: {other:?}"),
        }
    }

    /// Reshapes an entry the way `chariox.vault.copy_for_use` writes one for a site: HTTP use, a bearer header.
    fn made_by_copy_for_use(&self, id: &str) {
        let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
        let mut entry = registry.get(id).unwrap().unwrap();
        entry.allowed_uses = vec![crate::config::UserCredentialUse::Http];
        entry.injection = crate::config::UserCredentialInjectionConfig::Header {
            name: "Authorization".into(),
            value: "Bearer ${secret}".into(),
        };
        registry.upsert(entry).unwrap();
    }

    async fn start(&self, method: &str) -> String {
        let started = self
            .send(LocalDaemonRequest::StartProviderLogin(
                crate::local::StartProviderLoginRequest {
                    provider: "opencode".into(),
                    account_profile: self.profile.clone(),
                    method: Some(method.into()),
                },
            ))
            .await
            .unwrap();
        match started {
            LocalDaemonResponse::ProviderLoginStarted { login } => login.login_id.unwrap(),
            other => panic!("expected a login: {other:?}"),
        }
    }

    async fn vault_entry(
        &self,
        login_id: &str,
        credential_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.send(LocalDaemonRequest::SendProviderLoginVaultEntry(
            SendProviderLoginVaultEntryRequest {
                login_id: login_id.into(),
                credential_id: credential_id.into(),
            },
        ))
        .await
    }

    /// The screen a model could read, once the login has stopped changing.
    async fn status(&self, login_id: &str) -> crate::local::ProviderLoginStatus {
        let response =
            crate::runtime::provider_auth_control::execute_get_provider_login_status_request(
                &self.router.runtime_state,
                "local",
                crate::local::GetProviderLoginStatusRequest {
                    login_id: login_id.into(),
                },
            )
            .await
            .unwrap();
        match response {
            LocalDaemonResponse::ProviderLoginStatus { login } => login,
            other => panic!("expected status: {other:?}"),
        }
    }

    async fn until_finished(&self, login_id: &str) -> crate::local::ProviderLoginStatus {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let status = self.status(login_id).await;
                if status.state != ProviderLoginProcessState::Running {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the login should finish")
    }

    /// Lets the fragmenting login program go on once the kernel has had time
    /// to read what it printed.
    async fn release(&self, wait_for: &str, go: &str) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !self.root.join(wait_for).exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the login program should print its fragment");
        tokio::time::sleep(Duration::from_millis(400)).await;
        if !go.is_empty() {
            std::fs::write(self.root.join(go), "").unwrap();
        }
    }

    /// Polls the login a few times and fails if any answer shows the start of the key.
    async fn assert_no_key_start_shown(&self, login_id: &str) {
        for _ in 0..3 {
            let shown = screen(&self.status(login_id).await);
            assert!(
                !shown.contains(&KEY[..4]),
                "a poll showed the start of the key: {shown:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// What the fake login program was typed, kept beside it by the program itself.
    fn received(&self) -> Option<String> {
        std::fs::read_to_string(self.root.join("received")).ok()
    }
}

fn screen(status: &crate::local::ProviderLoginStatus) -> String {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&status.terminal_output_base64)
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn an_entry_bound_to_opencode_ai_signs_the_login_in_and_stays_off_the_screen() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::ProcessMemory);
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    for method in ["opencode_go_api_key", "opencode_zen_api_key"] {
        let login_id = fixture.start(method).await;
        let sent = fixture.vault_entry(&login_id, &entry).await.unwrap();
        let answer = format!("{sent:?}");
        assert!(matches!(
            sent,
            LocalDaemonResponse::ProviderLoginVaultEntrySent { .. }
        ));
        assert!(!answer.contains(KEY), "the answer must not carry the key");
        let status = fixture.until_finished(&login_id).await;
        assert_eq!(
            status.state,
            ProviderLoginProcessState::Succeeded,
            "{method}"
        );
        assert!(
            !screen(&status).contains(KEY) && !format!("{status:?}").contains(KEY),
            "the screen a model reads must not show the key"
        );
        assert!(
            fixture.received().is_some_and(|typed| typed == KEY),
            "the login program received the key"
        );
        let _ = std::fs::remove_file(fixture.root.join("received"));
    }
}

#[tokio::test]
async fn a_go_key_rejected_by_the_provider_does_not_complete_vault_login() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::ProcessMemory);
    std::env::set_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS", "401");
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    let status = fixture.until_finished(&login_id).await;
    assert_ne!(status.state, ProviderLoginProcessState::Succeeded);
    let profile = fixture
        .router
        .provider_account_profiles
        .get("local", "opencode", &fixture.profile)
        .unwrap();
    assert_eq!(
        profile.auth_state,
        crate::account_profile::ProviderAccountAuthState::Error
    );
    assert_eq!(
        profile.usage.management_url.as_deref(),
        Some("https://opencode.ai/console/go")
    );
    assert!(!screen(&status).contains(KEY));
}

#[tokio::test]
async fn an_entry_copy_for_use_made_for_opencode_ai_signs_the_login_in() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::ProcessMemory);
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    fixture.made_by_copy_for_use(&entry);
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    let status = fixture.until_finished(&login_id).await;
    assert_eq!(status.state, ProviderLoginProcessState::Succeeded);
    assert!(!screen(&status).contains(KEY));
    assert!(fixture.received().is_some_and(|typed| typed == KEY));
}

#[tokio::test]
async fn an_entry_for_another_site_is_refused_and_the_login_stays_open() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::ProcessMemory);
    let elsewhere = fixture.owner_entry("https://example.com").await;
    let login_id = fixture.start("opencode_go_api_key").await;
    let refused = fixture
        .vault_entry(&login_id, &elsewhere)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains(OPENCODE_SITE),
        "the reason names the site the entry must be bound to: {refused}"
    );
    assert!(!refused.to_string().contains(KEY));
    let status = fixture.status(&login_id).await;
    assert_eq!(status.state, ProviderLoginProcessState::Running);
    assert!(fixture.received().is_none(), "nothing was typed");
    fixture
        .send(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id },
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_login_without_an_api_key_and_a_second_entry_are_refused() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::ProcessMemory);
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let terminal = fixture.start("terminal").await;
    let refused = fixture.vault_entry(&terminal, &entry).await.unwrap_err();
    assert!(refused.to_string().contains("does not take an API key"));
    fixture
        .send(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id: terminal },
        ))
        .await
        .unwrap();
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    let second = fixture.vault_entry(&login_id, &entry).await.unwrap_err();
    assert!(second.to_string().contains("already typed"));
    fixture.until_finished(&login_id).await;
}

#[tokio::test]
async fn a_locked_vault_is_refused_as_locked_before_the_login_is_touched() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new(crate::config::CredentialVaultBackend::CharioxEncrypted);
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let vault = fixture.root.join("vault/vault.enc");
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    let login_id = fixture.start("opencode_go_api_key").await;
    let error = fixture.vault_entry(&login_id, &entry).await.unwrap_err();
    assert_eq!(
        crate::transport::kernel_protocol::map_kernel_error(&error).code,
        "vault_locked"
    );
    assert_eq!(
        fixture.status(&login_id).await.state,
        ProviderLoginProcessState::Running
    );
    fixture
        .send(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id },
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_key_echoed_in_fragments_never_shows_its_start_between_polls() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::with_program(
        crate::config::CredentialVaultBackend::ProcessMemory,
        FRAGMENTED_OPENCODE,
    );
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    fixture.release("sent1", "go2").await;
    fixture.assert_no_key_start_shown(&login_id).await;
    fixture.release("sent2", "go3").await;
    fixture.assert_no_key_start_shown(&login_id).await;
    let status = fixture.until_finished(&login_id).await;
    let shown = screen(&status);
    assert!(
        !shown.contains(&KEY[..4]) && !shown.contains(KEY),
        "{shown:?}"
    );
    assert!(
        shown.contains("echo: "),
        "the rest of the screen stays: {shown:?}"
    );
}

#[tokio::test]
async fn a_key_echo_cut_short_by_the_login_ending_is_masked_at_the_end() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::with_program(
        crate::config::CredentialVaultBackend::ProcessMemory,
        FRAGMENTED_OPENCODE,
    );
    std::fs::write(fixture.root.join("truncate"), "").unwrap();
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    fixture.release("sent1", "go2").await;
    fixture.release("sent2", "go3").await;
    let status = fixture.until_finished(&login_id).await;
    let shown = screen(&status);
    assert!(!shown.contains(&KEY[..4]), "{shown:?}");
    assert!(shown.contains("echo: **********"), "{shown:?}");
}

/// Starts the fragmenting login and stops it with part of the key echoed.
async fn fragmented_login_stopped_mid_echo(fixture: &Fixture) -> String {
    let entry = fixture.owner_entry(OPENCODE_SITE).await;
    let login_id = fixture.start("opencode_go_api_key").await;
    fixture.vault_entry(&login_id, &entry).await.unwrap();
    fixture.release("sent1", "go2").await;
    fixture.release("sent2", "").await;
    fixture.assert_no_key_start_shown(&login_id).await;
    login_id
}

#[tokio::test]
async fn a_key_echo_cut_short_by_the_login_timing_out_is_masked_before_the_timeout_note() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::with_program(
        crate::config::CredentialVaultBackend::ProcessMemory,
        FRAGMENTED_OPENCODE,
    );
    let login_id = fragmented_login_stopped_mid_echo(&fixture).await;
    fixture
        .router
        .runtime_state
        .provider_login_process_store()
        .backdate_start_for_test(&login_id, 11 * 60 * 1000);
    let status = fixture.status(&login_id).await;
    let shown = screen(&status);
    assert_eq!(status.state, ProviderLoginProcessState::Failed);
    assert!(shown.contains("timed out"), "{shown:?}");
    assert!(!shown.contains(&KEY[..4]), "{shown:?}");
}

#[tokio::test]
async fn a_key_echo_cut_short_by_a_failed_account_check_is_masked_before_the_failure_note() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::with_program(
        crate::config::CredentialVaultBackend::ProcessMemory,
        FRAGMENTED_OPENCODE,
    );
    std::fs::write(fixture.root.join("truncate"), "").unwrap();
    std::fs::write(fixture.root.join("listfail"), "").unwrap();
    let login_id = fragmented_login_stopped_mid_echo(&fixture).await;
    std::fs::write(fixture.root.join("go3"), "").unwrap();
    let status = fixture.until_finished(&login_id).await;
    let shown = screen(&status);
    assert_eq!(status.state, ProviderLoginProcessState::Failed);
    assert!(shown.contains("validation failed"), "{shown:?}");
    assert!(!shown.contains(&KEY[..4]), "{shown:?}");
}
