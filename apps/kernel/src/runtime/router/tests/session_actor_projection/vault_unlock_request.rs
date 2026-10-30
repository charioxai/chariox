//! `UnlockCredentialVault`: the owner's client unlocks the kernel's Vault with
//! no session or agent, inside the budget every passphrase check shares.
use super::*;
use crate::config::{CredentialVaultBackend, CredentialVaultUnlockPolicy};
use crate::local::{KernelConnectionClass, UnlockCredentialVaultRequest, VaultPassphrase};
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

const PASSPHRASE: &str = "correct horse battery";
const WRONG: &str = "tr0ub4dor&3";
const SENTINEL: &str = "sentinel-passphrase-5f1c";

struct Fixture {
    router: CommandRouter,
    vault: std::path::PathBuf,
    session: String,
    agent: String,
}

impl Fixture {
    fn new(policy: CredentialVaultUnlockPolicy, create_vault: bool) -> Self {
        Self::build(
            CredentialVaultBackend::CharioxEncrypted,
            policy,
            create_vault,
        )
    }

    fn build(
        backend: CredentialVaultBackend,
        policy: CredentialVaultUnlockPolicy,
        create_vault: bool,
    ) -> Self {
        let root =
            std::env::temp_dir().join(format!("vault-unlock-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault.json");
        if create_vault {
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSPHRASE).unwrap();
        }
        let mut config = DaemonConfig::for_tests();
        config.user_config.credential_vault.backend = backend;
        config.user_config.credential_vault.path = vault.display().to_string();
        config.user_config.credential_vault.unlock_policy = policy;
        config.user_config_path = root.join("config.toml");
        let app = DaemonApp::bootstrap(config).unwrap();
        let session = RuntimeSession::new(
            format!("vault-unlock-{:016x}", rand::random::<u64>()),
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        let session_id = session.id().to_owned();
        app.sessions_mut().restore_session(session);
        let agent = format!("vault-agent-{:016x}", rand::random::<u64>());
        app.agents_mut()
            .restore_agent(crate::agent::AgentInstance::new(
                agent.clone(),
                "vault-agent",
                session_id.clone(),
                None,
                "dev-stub",
                None,
                None,
                None,
                crate::agent::GridPosition::new(0, 0, 1, 1),
            ));
        Self {
            router: CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1),
            vault,
            session: session_id,
            agent,
        }
    }

    fn locked() -> Self {
        Self::new(CredentialVaultUnlockPolicy::Ttl, true)
    }

    async fn send(&self, request: LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
            .with_connection_class(KernelConnectionClass::Terminal);
        let command = KernelCommand::from_local_request_with_caller(
            format!("vault-unlock-{}", rand::random::<u64>()),
            KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await
    }

    async fn unlock(&self, passphrase: &str) -> Result<LocalDaemonResponse, DaemonError> {
        self.send(LocalDaemonRequest::UnlockCredentialVault(
            UnlockCredentialVaultRequest {
                passphrase: VaultPassphrase::new(passphrase),
            },
        ))
        .await
    }

    async fn status(&self) -> crate::secret::CharioxVaultUnlockStatus {
        match self
            .send(LocalDaemonRequest::GetCredentialVaultStatus(
                crate::local::GetCredentialVaultStatusRequest,
            ))
            .await
            .unwrap()
        {
            LocalDaemonResponse::CredentialVaultStatus { status } => status,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The interaction path: an agent's operation asks for the passphrase and
    /// the person answers it, as a terminal does.
    async fn unlock_through_the_agent_prompt(&self, passphrase: &str) -> Result<(), DaemonError> {
        let runtime = self.router.runtime_state.clone();
        let (session, agent) = (self.session.clone(), self.agent.clone());
        let operation = tokio::spawn(async move {
            runtime
                .manage_credential_vault_unlock(&session, &agent)
                .await
        });
        let prompt = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let snapshot = self
                    .router
                    .runtime_state
                    .session_snapshot(&self.session)
                    .await
                    .unwrap();
                if let Some(prompt) = snapshot.active_interaction_for_agent(&self.agent) {
                    break prompt.id().to_owned();
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the unlock prompt should appear");
        let answer =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: self.session.clone(),
                interaction_id: prompt,
                choice_id: "passphrase".into(),
                custom_reply: Some(passphrase.to_owned()),
                passkey: None,
                passkey_remember_minutes: None,
            });
        self.send(answer).await.unwrap();
        operation.await.unwrap().map(|_| ())
    }
}

fn code_of(result: Result<LocalDaemonResponse, DaemonError>) -> (String, String) {
    let error = result.expect_err("the request should be refused");
    let mapped = crate::transport::kernel_protocol::map_kernel_error(&error);
    (mapped.code, mapped.message)
}

fn seconds_in(message: &str) -> u64 {
    message
        .split_whitespace()
        .rev()
        .nth(1)
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("no seconds in `{message}`"))
}

#[tokio::test]
async fn the_owner_unlocks_the_vault_with_no_session() {
    let f = Fixture::locked();
    assert!(!f.status().await.unlocked);
    let LocalDaemonResponse::CredentialVaultUnlocked { status, action } =
        f.unlock(PASSPHRASE).await.unwrap()
    else {
        panic!("expected CredentialVaultUnlocked");
    };
    assert!(status.unlocked);
    assert_eq!(action, "unlocked");
    assert!(f.status().await.unlocked);
    let LocalDaemonResponse::CredentialVaultUnlocked { action, .. } =
        f.unlock(PASSPHRASE).await.unwrap()
    else {
        panic!("expected CredentialVaultUnlocked");
    };
    assert_eq!(action, "already_unlocked");
}

#[tokio::test]
async fn a_wrong_passphrase_says_so_in_plain_words_and_leaves_the_vault_locked() {
    let f = Fixture::locked();
    let (code, message) = code_of(f.unlock(WRONG).await);
    assert_eq!(code, "vault_passphrase_wrong");
    assert_eq!(message, "That is not the Vault passphrase. Try again.");
    assert!(!f.status().await.unlocked);
}

#[tokio::test]
async fn five_wrong_passphrases_lock_out_even_the_right_one_and_the_wait_doubles_to_the_cap() {
    let f = Fixture::locked();
    for _ in 0..5 {
        assert_eq!(code_of(f.unlock(WRONG).await).0, "vault_passphrase_wrong");
    }
    let (code, message) = code_of(f.unlock(PASSPHRASE).await);
    assert_eq!(code, "vault_unlock_rate_limited");
    assert!((29..=30).contains(&seconds_in(&message)), "{message}");
    assert!(!f.status().await.unlocked);
    let mut waits = Vec::new();
    for _ in 0..12 {
        f.router
            .runtime_state
            .expire_critical_approval_presence_for_test(DEFAULT_LOCAL_USER_ID);
        assert_eq!(code_of(f.unlock(WRONG).await).0, "vault_passphrase_wrong");
        waits.push(seconds_in(&code_of(f.unlock(PASSPHRASE).await).1));
    }
    assert!((59..=60).contains(&waits[0]), "{waits:?}");
    assert!((119..=120).contains(&waits[1]), "{waits:?}");
    assert!(waits.iter().all(|wait| *wait <= 900), "{waits:?}");
    assert!((899..=900).contains(waits.last().unwrap()), "{waits:?}");
}

#[tokio::test]
async fn parallel_guesses_are_checked_one_at_a_time_inside_the_budget() {
    let f = Arc::new(Fixture::locked());
    let guesses: Vec<_> = (0..20)
        .map(|_| {
            let f = Arc::clone(&f);
            tokio::spawn(async move { code_of(f.unlock(WRONG).await).0 })
        })
        .collect();
    let mut codes = Vec::new();
    for guess in guesses {
        codes.push(guess.await);
    }
    let count = |wanted: &str| {
        codes
            .iter()
            .filter(|code| code.as_ref().unwrap() == wanted)
            .count()
    };
    assert_eq!(count("vault_passphrase_wrong"), 5);
    assert_eq!(count("vault_unlock_rate_limited"), 15);
}

#[tokio::test]
async fn the_budget_is_shared_with_the_interaction_unlock() {
    let f = Fixture::locked();
    for _ in 0..5 {
        assert!(f.unlock_through_the_agent_prompt(WRONG).await.is_err());
    }
    let (code, _) = code_of(f.unlock(PASSPHRASE).await);
    assert_eq!(code, "vault_unlock_rate_limited");
    let error = f
        .unlock_through_the_agent_prompt(PASSPHRASE)
        .await
        .expect_err("a locked-out owner cannot unlock through the prompt either");
    assert!(
        error.to_string().contains("Too many wrong passphrases"),
        "{error}"
    );
    assert!(!f.status().await.unlocked);
}

#[tokio::test]
async fn wrong_unlocks_count_against_the_interaction_unlock() {
    let f = Fixture::locked();
    for _ in 0..5 {
        assert_eq!(code_of(f.unlock(WRONG).await).0, "vault_passphrase_wrong");
    }
    let error = f
        .unlock_through_the_agent_prompt(PASSPHRASE)
        .await
        .expect_err("the budget is spent");
    assert!(
        error.to_string().contains("Too many wrong passphrases"),
        "{error}"
    );
}

#[tokio::test]
async fn a_missing_vault_is_refused_and_not_created() {
    let f = Fixture::new(CredentialVaultUnlockPolicy::Ttl, false);
    let (code, message) = code_of(f.unlock(PASSPHRASE).await);
    assert_eq!(code, "vault_missing");
    assert_eq!(
        message,
        "This kernel has no Vault yet. Create it on that machine first."
    );
    assert!(!f.vault.exists());
}

#[tokio::test]
async fn a_plain_backend_needs_no_unlock() {
    let f = Fixture::build(
        CredentialVaultBackend::ProcessMemory,
        CredentialVaultUnlockPolicy::Ttl,
        true,
    );
    let LocalDaemonResponse::CredentialVaultUnlocked { action, .. } =
        f.unlock(PASSPHRASE).await.unwrap()
    else {
        panic!("expected CredentialVaultUnlocked");
    };
    assert_eq!(action, "not_required");
}

#[tokio::test]
async fn the_passphrase_is_in_no_record_debug_or_reply() {
    let f = Fixture::locked();
    let request = LocalDaemonRequest::UnlockCredentialVault(UnlockCredentialVaultRequest {
        passphrase: VaultPassphrase::new(SENTINEL),
    });
    assert!(format!("{request:?}").contains("[REDACTED]"));
    assert!(!format!("{request:?}").contains(SENTINEL));
    let command = KernelCommand::from_local_request_with_caller(
        "sentinel".to_string(),
        KernelCommandSource::LocalCli,
        KernelCaller::for_source(&KernelCommandSource::LocalCli),
        None,
        None,
        &request,
    );
    assert!(!serde_json::to_string(&command).unwrap().contains(SENTINEL));
    let (_, message) = code_of(f.send(request).await);
    assert!(!message.contains(SENTINEL));
}

#[tokio::test]
async fn an_operation_without_a_session_asks_the_client_to_unlock_first() {
    let f = Fixture::locked();
    let (code, message) = code_of(
        f.send(LocalDaemonRequest::SetCredentialSecret(
            crate::local::SetCredentialSecretRequest {
                session_id: None,
                agent_id: None,
                key: "login".into(),
                value: "value".into(),
            },
        ))
        .await,
    );
    assert_eq!(code, "vault_locked");
    assert_eq!(message, "The Vault is locked. Unlock it, then try again.");
}

#[tokio::test]
async fn the_lease_follows_the_unlock_policy() {
    let now_ms = crate::session::unix_epoch_ms();
    let kernel_init = Fixture::new(CredentialVaultUnlockPolicy::KernelInit, true);
    kernel_init.unlock(PASSPHRASE).await.unwrap();
    assert_eq!(kernel_init.status().await.expires_at_ms, None);

    let ttl = Fixture::new(CredentialVaultUnlockPolicy::Ttl, true);
    ttl.unlock(PASSPHRASE).await.unwrap();
    let config = ttl
        .router
        .runtime_state
        .config_snapshot()
        .await
        .user_config
        .credential_vault;
    let minutes = config
        .default_ttl_minutes
        .min(config.max_ttl_minutes)
        .max(1);
    let expires = ttl.status().await.expires_at_ms.unwrap();
    assert!(expires.abs_diff(now_ms + minutes * 60_000) < 5_000);

    let always = Fixture::new(CredentialVaultUnlockPolicy::Always, true);
    always.unlock(PASSPHRASE).await.unwrap();
    let expires = always.status().await.expires_at_ms.unwrap();
    assert!(
        expires.abs_diff(now_ms + 60_000) < 5_000,
        "single use lasts a minute"
    );
}
