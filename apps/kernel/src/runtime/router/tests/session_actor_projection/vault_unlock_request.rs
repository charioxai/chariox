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
        self.send_as(caller, request).await
    }

    async fn send_as(
        &self,
        caller: KernelCaller,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
        self.answer_the_unlock_prompt(passphrase).await;
        operation.await.unwrap().map(|_| ())
    }

    /// The person answers the unlock prompt that appeared in the agent's terminal.
    async fn answer_the_unlock_prompt(&self, passphrase: &str) {
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

fn unlock_request(passphrase: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::UnlockCredentialVault(UnlockCredentialVaultRequest {
        passphrase: VaultPassphrase::new(passphrase),
    })
}

#[tokio::test]
async fn only_the_owners_own_client_may_unlock() {
    let f = Fixture::locked();
    // A member of a shared session is a terminal with a user id, not the owner.
    let mut member = KernelCaller::for_source(&KernelCommandSource::RelayClient)
        .with_connection_class(KernelConnectionClass::Terminal);
    member.user_id = Some("session-member".into());
    // An agent the kernel launched, and a Metaagent, are not the owner's client.
    let agent = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    let mut metaagent = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(KernelConnectionClass::Terminal);
    metaagent.metaagent_id = Some("metaagent".into());
    for caller in [member, agent, metaagent] {
        let (code, message) = code_of(f.send_as(caller, unlock_request(PASSPHRASE)).await);
        assert_eq!(code, "vault_unlock_owner_only");
        assert_eq!(
            message,
            "Only the owner of this kernel can unlock its Vault."
        );
        assert!(
            !f.status().await.unlocked,
            "a refused caller unlocks nothing"
        );
    }
    // The refusal is before the passphrase check: it costs the owner no attempt.
    for _ in 0..6 {
        let mut member = KernelCaller::for_source(&KernelCommandSource::RelayClient)
            .with_connection_class(KernelConnectionClass::Terminal);
        member.user_id = Some("session-member".into());
        f.send_as(member, unlock_request(WRONG)).await.unwrap_err();
    }
    f.unlock(PASSPHRASE).await.unwrap();
    assert!(f.status().await.unlocked);
}

#[tokio::test]
async fn the_vault_menu_on_a_locked_vault_asks_the_client_to_unlock_and_raises_no_prompt() {
    let f = Fixture::locked();
    let manage =
        LocalDaemonRequest::ManageCredentialVault(crate::local::ManageCredentialVaultRequest {
            session_id: f.session.clone(),
            agent_id: Some(f.agent.clone()),
        });
    let (code, message) = code_of(
        tokio::time::timeout(std::time::Duration::from_secs(10), f.send(manage))
            .await
            .expect("a locked Vault must not wait for an unlock prompt"),
    );
    assert_eq!(code, "vault_locked");
    assert_eq!(message, "The Vault is locked. Unlock it, then try again.");
    let snapshot = f
        .router
        .runtime_state
        .session_snapshot(&f.session)
        .await
        .unwrap();
    assert!(
        snapshot.active_interaction_for_agent(&f.agent).is_none(),
        "no unlock prompt is raised"
    );
}

#[tokio::test]
async fn the_vault_menu_on_a_kernel_with_no_vault_still_makes_the_first_one() {
    let f = Fixture::new(CredentialVaultUnlockPolicy::Ttl, false);
    let runtime = f.router.runtime_state.clone();
    let request = crate::local::ManageCredentialVaultRequest {
        session_id: f.session.clone(),
        agent_id: Some(f.agent.clone()),
    };
    let operation = tokio::spawn(async move {
        crate::runtime::user_config_executor::execute_manage_credential_vault_request(
            &runtime, request,
        )
        .await
    });
    f.answer_the_unlock_prompt(PASSPHRASE).await;
    operation.await.unwrap().unwrap();
    assert!(f.vault.exists(), "the first-time prompt created the Vault");
    assert!(f.status().await.unlocked);
}

fn caller_of(class: Option<KernelConnectionClass>, user: Option<&str>) -> KernelCaller {
    let source = match class {
        Some(KernelConnectionClass::Terminal) if user.is_some() => KernelCommandSource::RelayClient,
        Some(KernelConnectionClass::RelayPeer) => KernelCommandSource::RelayPeer,
        _ => KernelCommandSource::LocalCli,
    };
    let mut caller = KernelCaller::for_source(&source);
    if let Some(class) = class {
        caller = caller.with_connection_class(class);
    }
    caller.user_id = user.map(str::to_owned);
    caller
}

fn vault_changes() -> Vec<(&'static str, LocalDaemonRequest)> {
    let credential = crate::config::UserCredentialConfig {
        id: "login".into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: "login".into(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: Vec::new(),
        injection: crate::config::UserCredentialInjectionConfig::Provider,
        metadata: None,
    };
    vec![
        (
            "lock",
            LocalDaemonRequest::LockCredentialVault(crate::local::LockCredentialVaultRequest),
        ),
        (
            "set",
            LocalDaemonRequest::SetCredentialSecret(crate::local::SetCredentialSecretRequest {
                session_id: None,
                agent_id: None,
                key: "login".into(),
                value: "value".into(),
            }),
        ),
        (
            "delete",
            LocalDaemonRequest::DeleteCredentialSecret(
                crate::local::DeleteCredentialSecretRequest {
                    session_id: None,
                    agent_id: None,
                    key: "login".into(),
                },
            ),
        ),
        (
            "upsert",
            LocalDaemonRequest::UpsertCredential(crate::local::UpsertCredentialRequest {
                credential,
            }),
        ),
        (
            "remove",
            LocalDaemonRequest::RemoveCredential(crate::local::RemoveCredentialRequest {
                id: "login".into(),
            }),
        ),
        (
            "register",
            LocalDaemonRequest::RegisterCredential(crate::local::RegisterCredentialRequest {
                source_path: "/nonexistent/credential.yaml".into(),
            }),
        ),
        (
            "sign-in",
            LocalDaemonRequest::SetProviderAccountCredential(
                crate::local::SetProviderAccountCredentialRequest {
                    session_id: None,
                    agent_id: None,
                    provider: "claude".into(),
                    account_profile: "none".into(),
                    value: "value".into(),
                    run: false,
                    overwrite: false,
                },
            ),
        ),
    ]
}

/// The refusal is the first thing a caller meets, so it must not depend on
/// what else is wrong with the request.
fn refused_as_owner_only(result: Result<LocalDaemonResponse, DaemonError>) -> bool {
    matches!(
        result,
        Err(DaemonError::VaultUnlock { code, .. }) if code.ends_with("owner_only") || code.ends_with("person_only")
    )
}

#[tokio::test]
async fn only_the_owner_may_lock_the_vault_or_change_what_it_stores() {
    let f = Fixture::locked();
    f.unlock(PASSPHRASE).await.unwrap();
    let mut metaagent = caller_of(Some(KernelConnectionClass::Terminal), None);
    metaagent.metaagent_id = Some("metaagent".into());
    // Callers that are neither the owner's client nor an agent of the owner. A
    // member is a person at a client, so signing in their own account is theirs.
    let strangers = [
        (
            "member",
            caller_of(
                Some(KernelConnectionClass::Terminal),
                Some("session-member"),
            ),
            "sign-in",
        ),
        (
            "relay peer",
            caller_of(
                Some(KernelConnectionClass::RelayPeer),
                Some("remote-kernel"),
            ),
            "",
        ),
        (
            "unauthenticated",
            caller_of(Some(KernelConnectionClass::Unauthenticated), None),
            "",
        ),
    ];
    for (who, caller, allowed) in strangers {
        for (name, request) in vault_changes() {
            let refused = refused_as_owner_only(f.send_as(caller.clone(), request).await);
            assert_eq!(refused, name != allowed, "{who} {name}");
        }
    }
    // Agents may do what their tools are for, and nothing else: a token they
    // were handed can be saved, but locking the Vault is the owner's.
    let agents = [
        (
            "kernel agent",
            caller_of(Some(KernelConnectionClass::KernelAgent), None),
        ),
        ("metaagent", metaagent),
    ];
    for (who, caller) in agents {
        for (name, request) in vault_changes() {
            let refused = refused_as_owner_only(f.send_as(caller.clone(), request).await);
            assert_eq!(refused, name == "lock", "{who} {name}");
        }
    }
    // A local grant holder is stopped before the gate unless its grant is live.
    let (_, lock) = vault_changes().remove(0);
    let grant_holder = caller_of(Some(KernelConnectionClass::ExternalAgent), None);
    assert!(f.send_as(grant_holder, lock).await.is_err());
    assert!(
        f.status().await.unlocked,
        "no refused caller locked the Vault"
    );
    // The owner's own client is never refused.
    for (name, request) in vault_changes() {
        assert!(
            !refused_as_owner_only(f.send(request).await),
            "owner {name}"
        );
    }
}

#[tokio::test]
async fn an_agent_still_reaches_the_vault_menu_and_its_own_secret_writes() {
    let f = Fixture::locked();
    f.unlock(PASSPHRASE).await.unwrap();
    let agent = caller_of(Some(KernelConnectionClass::KernelAgent), None);
    let (_, set) = vault_changes().remove(1);
    f.send_as(agent.clone(), set).await.unwrap();
    let manage =
        LocalDaemonRequest::ManageCredentialVault(crate::local::ManageCredentialVaultRequest {
            session_id: f.session.clone(),
            agent_id: Some(f.agent.clone()),
        });
    // The menu waits for the owner's answer; it is not refused.
    let answer =
        tokio::time::timeout(std::time::Duration::from_secs(2), f.send_as(agent, manage)).await;
    assert!(answer.is_err() || !refused_as_owner_only(answer.unwrap()));
}

fn open_the_vault_menu(f: &Fixture) -> LocalDaemonRequest {
    LocalDaemonRequest::ManageCredentialVault(crate::local::ManageCredentialVaultRequest {
        session_id: f.session.clone(),
        agent_id: Some(f.agent.clone()),
    })
}

#[tokio::test]
async fn only_the_owner_or_its_agents_may_open_the_vault_menu() {
    let f = Fixture::locked();
    f.unlock(PASSPHRASE).await.unwrap();
    let strangers = [
        (
            "member",
            caller_of(
                Some(KernelConnectionClass::Terminal),
                Some("session-member"),
            ),
        ),
        (
            "relay peer",
            caller_of(
                Some(KernelConnectionClass::RelayPeer),
                Some("remote-kernel"),
            ),
        ),
        (
            "unauthenticated",
            caller_of(Some(KernelConnectionClass::Unauthenticated), None),
        ),
    ];
    for (who, caller) in strangers {
        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            f.send_as(caller, open_the_vault_menu(&f)),
        )
        .await;
        let refused = answer.is_ok_and(refused_as_owner_only);
        assert!(refused, "{who} may not open the Vault menu");
    }
    let pending = f
        .router
        .runtime_state
        .session_snapshot(&f.session)
        .await
        .unwrap()
        .active_interaction_for_agent(&f.agent)
        .is_some();
    assert!(!pending, "a refused caller raised no menu in the session");
}

#[tokio::test]
async fn a_member_cannot_answer_the_vault_menu() {
    let f = Fixture::locked();
    f.unlock(PASSPHRASE).await.unwrap();
    let runtime = f.router.runtime_state.clone();
    let (session, agent) = (f.session.clone(), f.agent.clone());
    let menu = tokio::spawn(async move {
        runtime
            .manage_credential_vault_unlock(&session, &agent)
            .await
    });
    let prompt = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let snapshot = f.router.runtime_state.session_snapshot(&f.session).await;
            if let Some(prompt) = snapshot
                .unwrap()
                .active_interaction_for_agent(&f.agent)
                .map(|prompt| prompt.id().to_owned())
            {
                break prompt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the Vault menu should appear");
    let lock_now =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: f.session.clone(),
            interaction_id: prompt,
            choice_id: "lock_now".into(),
            custom_reply: None,
            passkey: None,
            passkey_remember_minutes: None,
        });
    let member = caller_of(
        Some(KernelConnectionClass::Terminal),
        Some("session-member"),
    );
    assert!(f.send_as(member, lock_now).await.is_err());
    assert!(
        f.status().await.unlocked,
        "the member did not lock the Vault"
    );
    menu.abort();
}

fn sign_in(run: bool) -> LocalDaemonRequest {
    LocalDaemonRequest::SetProviderAccountCredential(
        crate::local::SetProviderAccountCredentialRequest {
            session_id: None,
            agent_id: None,
            provider: "claude".into(),
            account_profile: "none".into(),
            value: "value".into(),
            run,
            overwrite: false,
        },
    )
}

#[tokio::test]
async fn an_agent_may_save_a_token_it_was_given_but_not_start_the_login() {
    let f = Fixture::locked();
    f.unlock(PASSPHRASE).await.unwrap();
    let mut metaagent = caller_of(Some(KernelConnectionClass::Terminal), None);
    metaagent.metaagent_id = Some("metaagent".into());
    for agent in [
        caller_of(Some(KernelConnectionClass::KernelAgent), None),
        metaagent,
    ] {
        assert!(!refused_as_owner_only(
            f.send_as(agent.clone(), sign_in(false)).await
        ));
        assert!(refused_as_owner_only(f.send_as(agent, sign_in(true)).await));
    }
}

fn set_secret(value: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::SetCredentialSecret(crate::local::SetCredentialSecretRequest {
        session_id: None,
        agent_id: None,
        key: "login".into(),
        value: value.into(),
    })
}

#[tokio::test]
async fn under_ask_every_time_the_client_unlock_covers_the_next_write_only() {
    let f = Fixture::new(CredentialVaultUnlockPolicy::Always, true);
    f.unlock(PASSPHRASE).await.unwrap();
    f.send(set_secret("first")).await.unwrap();
    assert!(!f.status().await.unlocked, "the write used the unlock up");
    let (code, _) = code_of(f.send(set_secret("second")).await);
    assert_eq!(code, "vault_locked");

    f.unlock(PASSPHRASE).await.unwrap();
    f.send(LocalDaemonRequest::DeleteCredentialSecret(
        crate::local::DeleteCredentialSecretRequest {
            session_id: None,
            agent_id: None,
            key: "login".into(),
        },
    ))
    .await
    .unwrap();
    assert!(!f.status().await.unlocked);
}
