//! `UnlockCredentialVault`: the owner's client unlocks the kernel's Vault with
//! no session or agent, inside the budget every passphrase check shares.
use super::*;
use crate::config::{CredentialVaultBackend, CredentialVaultUnlockPolicy};
use crate::local::{
    KernelConnectionClass, UnlockCredentialVaultRequest, VaultPassphrase, VaultUnlockDuration,
    VaultUnlockSurface,
};
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

pub(super) const PASSPHRASE: &str = "correct horse battery";
const WRONG: &str = "tr0ub4dor&3";
const SENTINEL: &str = "sentinel-passphrase-5f1c";

pub(super) struct Fixture {
    pub(super) router: CommandRouter,
    pub(super) vault: std::path::PathBuf,
    session: String,
    agent: String,
    pub(super) kernel_id: String,
    pub(super) relay_public_key: String,
}

impl Fixture {
    pub(super) fn new(policy: CredentialVaultUnlockPolicy, create_vault: bool) -> Self {
        Self::build(
            CredentialVaultBackend::CharioxEncrypted,
            policy,
            create_vault,
        )
    }

    /// A kernel enrolled for `owner`, whose own client reaches it over the relay.
    pub(super) fn enrolled(owner: &str, create_vault: bool) -> Self {
        Self::build_for(
            CredentialVaultBackend::CharioxEncrypted,
            CredentialVaultUnlockPolicy::Ttl,
            create_vault,
            Some(owner),
        )
    }

    fn build(
        backend: CredentialVaultBackend,
        policy: CredentialVaultUnlockPolicy,
        create_vault: bool,
    ) -> Self {
        Self::build_for(backend, policy, create_vault, None)
    }

    fn build_for(
        backend: CredentialVaultBackend,
        policy: CredentialVaultUnlockPolicy,
        create_vault: bool,
        owner: Option<&str>,
    ) -> Self {
        let root =
            std::env::temp_dir().join(format!("vault-unlock-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault.json");
        if create_vault {
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSPHRASE).unwrap();
        }
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = format!("vault-kernel-{:016x}", rand::random::<u64>());
        let (kernel_id, relay_public_key) =
            (config.daemon_id.clone(), config.relay_public_key.clone());
        config.user_config.credential_vault.backend = backend;
        config.user_config.credential_vault.path = vault.display().to_string();
        config.user_config.credential_vault.unlock_policy = policy;
        config.user_config_path = root.join("config.toml");
        if let Some(owner) = owner {
            config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
                user_id: owner.to_owned(),
                ..Default::default()
            });
        }
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
            kernel_id,
            relay_public_key,
        }
    }

    pub(super) fn locked() -> Self {
        Self::new(CredentialVaultUnlockPolicy::Ttl, true)
    }

    pub(super) async fn send(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
            .with_connection_class(KernelConnectionClass::Terminal);
        self.send_as(caller, request).await
    }

    pub(super) async fn send_as(
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

    pub(super) async fn unlock(
        &self,
        passphrase: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.unlock_for(passphrase, VaultUnlockDuration::Minutes { minutes: 30 })
            .await
    }

    async fn unlock_for(
        &self,
        passphrase: &str,
        duration: VaultUnlockDuration,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.send(unlock_request(passphrase, duration)).await
    }

    pub(super) async fn status(&self) -> crate::secret::CharioxVaultUnlockStatus {
        match self
            .send(LocalDaemonRequest::GetCredentialVaultStatus(
                crate::local::GetCredentialVaultStatusRequest,
            ))
            .await
            .unwrap()
        {
            LocalDaemonResponse::CredentialVaultStatus { status, .. } => status,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The id of the unlock prompt that appears in the agent's terminal.
    async fn the_open_unlock_prompt(&self) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
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
        .expect("the unlock prompt should appear")
    }

    async fn no_prompt_is_open(&self) -> bool {
        self.router
            .runtime_state
            .session_snapshot(&self.session)
            .await
            .unwrap()
            .active_interaction_for_agent(&self.agent)
            .is_none()
    }

    /// The open unlock prompt asked after the question stamped `after`, with its message.
    /// A refusal arrives in the same card, so the stamp tells the questions apart.
    async fn next_prompt(&self, after: Option<u64>) -> (String, String, u64) {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let snapshot = self
                    .router
                    .runtime_state
                    .session_snapshot(&self.session)
                    .await
                    .unwrap();
                if let Some(prompt) = snapshot
                    .active_interaction_for_agent(&self.agent)
                    .filter(|prompt| after.is_none_or(|after| prompt.requested_at_ms() > after))
                {
                    break (
                        prompt.id().to_owned(),
                        prompt.message().to_owned(),
                        prompt.requested_at_ms(),
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("an unlock prompt should appear")
    }

    async fn answer_prompt(&self, prompt: &str, choice: &str, reply: Option<&str>) {
        let answer =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: self.session.clone(),
                interaction_id: prompt.to_owned(),
                choice_id: choice.into(),
                custom_reply: reply.map(str::to_owned),
                passkey: None,
                passkey_remember_minutes: None,
            });
        self.send(answer).await.unwrap();
    }

    fn the_agents_operation(
        &self,
    ) -> tokio::task::JoinHandle<
        Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError>,
    > {
        let runtime = self.router.runtime_state.clone();
        let (session, agent) = (self.session.clone(), self.agent.clone());
        tokio::spawn(async move {
            runtime
                .manage_credential_vault_unlock(&session, &agent)
                .await
                .map(|managed| (managed.status, managed.action))
        })
    }
}

// VUI-2: whatever window the owner unlocks from, the open unlock card closes and the agent's operation goes on.
#[tokio::test]
async fn an_unlock_from_the_vault_window_closes_the_open_card_and_the_agent_goes_on() {
    for duration in [
        VaultUnlockDuration::Once,
        VaultUnlockDuration::Minutes { minutes: 30 },
        VaultUnlockDuration::UntilKernelStops,
    ] {
        let f = Fixture::locked();
        let runtime = f.router.runtime_state.clone();
        let (session, agent) = (f.session.clone(), f.agent.clone());
        let operation = tokio::spawn(async move {
            runtime
                .manage_credential_vault_unlock(&session, &agent)
                .await
        });
        f.the_open_unlock_prompt().await;
        f.unlock_for(PASSPHRASE, duration).await.unwrap();
        let managed = tokio::time::timeout(std::time::Duration::from_secs(30), operation)
            .await
            .expect("the agent's operation is woken by the unlock")
            .unwrap()
            .expect("the operation succeeds");
        assert_eq!(managed.action, "unlocked", "{duration:?}");
        assert_eq!(
            managed.unlocked_via,
            Some(crate::local::VaultUnlockSurface::VaultWindow),
            "{duration:?}: the agent is told where the person unlocked"
        );
        assert!(managed.status.unlocked);
        assert!(
            f.no_prompt_is_open().await,
            "{duration:?}: the card closed by itself"
        );
    }
}

#[tokio::test]
async fn an_unlock_request_cannot_claim_to_come_from_a_card() {
    let f = Fixture::locked();
    let (code, _) = code_of(
        f.send(LocalDaemonRequest::UnlockCredentialVault(
            UnlockCredentialVaultRequest {
                passphrase: VaultPassphrase::new(PASSPHRASE),
                duration: VaultUnlockDuration::Once,
                surface: VaultUnlockSurface::Card,
            },
        ))
        .await,
    );
    assert_eq!(code, "vault_unlock_card_surface");
    assert!(!f.status().await.unlocked);
}

pub(super) fn code_of(result: Result<LocalDaemonResponse, DaemonError>) -> (String, String) {
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

// PCD-04: a wrong passphrase in the card asks again in a card that says so; the budget is still the owner's one.
#[tokio::test]
async fn the_budget_is_shared_with_the_interaction_unlock() {
    let f = Fixture::locked();
    let operation = f.the_agents_operation();
    let mut previous: Option<u64> = None;
    for _ in 0..5 {
        let (id, _, stamp) = f.next_prompt(previous).await;
        f.answer_prompt(&id, "unlock_kernel", Some(WRONG)).await;
        previous = Some(stamp);
    }
    let (id, message, stamp) = f.next_prompt(previous).await;
    assert!(
        message.starts_with("That is not the Vault passphrase"),
        "{message}"
    );
    let (code, _) = code_of(f.unlock(PASSPHRASE).await);
    assert_eq!(code, "vault_unlock_rate_limited");
    f.answer_prompt(&id, "unlock_kernel", Some(PASSPHRASE))
        .await;
    let (id, message, _) = f.next_prompt(Some(stamp)).await;
    assert!(
        message.contains("Too many wrong passphrases"),
        "a locked-out owner cannot unlock through the prompt either: {message}"
    );
    f.answer_prompt(&id, "cancel", None).await;
    operation
        .await
        .unwrap()
        .expect_err("the cancelled card ends the operation");
    assert!(!f.status().await.unlocked);
}

#[tokio::test]
async fn wrong_unlocks_count_against_the_interaction_unlock() {
    let f = Fixture::locked();
    for _ in 0..5 {
        assert_eq!(code_of(f.unlock(WRONG).await).0, "vault_passphrase_wrong");
    }
    let operation = f.the_agents_operation();
    let (id, _, stamp) = f.next_prompt(None).await;
    f.answer_prompt(&id, "unlock_kernel", Some(PASSPHRASE))
        .await;
    let (id, message, _) = f.next_prompt(Some(stamp)).await;
    assert!(
        message.contains("Too many wrong passphrases"),
        "the budget is spent: {message}"
    );
    f.answer_prompt(&id, "cancel", None).await;
    operation
        .await
        .unwrap()
        .expect_err("the cancelled card ends the operation");
}

#[tokio::test]
async fn unlocking_a_kernel_with_no_vault_asks_for_a_passphrase_and_creates_nothing() {
    let f = Fixture::new(CredentialVaultUnlockPolicy::Ttl, false);
    let (code, message) = code_of(f.unlock(PASSPHRASE).await);
    assert_eq!(code, "vault_awaiting_passphrase");
    assert_eq!(
        message,
        "This kernel's Vault has no passphrase yet. Choose one to create it."
    );
    assert!(!f.vault.exists());
}

#[tokio::test]
async fn a_plain_backend_needs_no_unlock() {
    let f = Fixture::build(
        CredentialVaultBackend::ProcessMemory,
        CredentialVaultUnlockPolicy::Ttl,
        false,
    );
    assert!(
        !f.status().await.awaiting_passphrase,
        "a plain backend has no Vault to create"
    );
    let LocalDaemonResponse::CredentialVaultUnlocked { action, status } =
        f.unlock(PASSPHRASE).await.unwrap()
    else {
        panic!("expected CredentialVaultUnlocked");
    };
    assert_eq!(action, "not_required");
    assert!(!status.awaiting_passphrase);
}

#[tokio::test]
async fn the_passphrase_is_in_no_record_debug_or_reply() {
    let f = Fixture::locked();
    let request = unlock_request(SENTINEL, VaultUnlockDuration::Once);
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
async fn the_lease_follows_the_chosen_duration_and_the_policy_only_preselects() {
    let now_ms = crate::session::unix_epoch_ms();
    for policy in [
        CredentialVaultUnlockPolicy::KernelInit,
        CredentialVaultUnlockPolicy::Ttl,
        CredentialVaultUnlockPolicy::Always,
    ] {
        let kernel = Fixture::new(policy, true);
        kernel
            .unlock_for(PASSPHRASE, VaultUnlockDuration::UntilKernelStops)
            .await
            .unwrap();
        assert_eq!(kernel.status().await.expires_at_ms, None, "{policy:?}");
    }

    let timed = Fixture::new(CredentialVaultUnlockPolicy::KernelInit, true);
    timed
        .unlock_for(PASSPHRASE, VaultUnlockDuration::Minutes { minutes: 45 })
        .await
        .unwrap();
    let expires = timed.status().await.expires_at_ms.unwrap();
    assert!(expires.abs_diff(now_ms + 45 * 60_000) < 5_000);

    let once = Fixture::new(CredentialVaultUnlockPolicy::Ttl, true);
    once.unlock_for(PASSPHRASE, VaultUnlockDuration::Once)
        .await
        .unwrap();
    let expires = once.status().await.expires_at_ms.unwrap();
    assert!(
        expires.abs_diff(now_ms + 60_000) < 5_000,
        "single use lasts a minute"
    );
}

#[tokio::test]
async fn the_status_offers_the_durations_and_preselects_the_policys_own() {
    for (policy, preselected) in [
        (
            CredentialVaultUnlockPolicy::KernelInit,
            VaultUnlockDuration::UntilKernelStops,
        ),
        (
            CredentialVaultUnlockPolicy::Always,
            VaultUnlockDuration::Once,
        ),
    ] {
        let f = Fixture::new(policy, true);
        let LocalDaemonResponse::CredentialVaultStatus { unlock, .. } = f
            .send(LocalDaemonRequest::GetCredentialVaultStatus(
                crate::local::GetCredentialVaultStatusRequest,
            ))
            .await
            .unwrap()
        else {
            panic!("expected CredentialVaultStatus");
        };
        assert_eq!(unlock.default, preselected, "{policy:?}");
        assert_eq!(unlock.durations.first(), Some(&VaultUnlockDuration::Once));
        assert_eq!(
            unlock.durations.last(),
            Some(&VaultUnlockDuration::UntilKernelStops)
        );
    }
}

fn unlock_request(passphrase: &str, duration: VaultUnlockDuration) -> LocalDaemonRequest {
    LocalDaemonRequest::UnlockCredentialVault(UnlockCredentialVaultRequest {
        passphrase: VaultPassphrase::new(passphrase),
        duration,
        surface: VaultUnlockSurface::VaultWindow,
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
        let (code, message) = code_of(
            f.send_as(
                caller,
                unlock_request(PASSPHRASE, VaultUnlockDuration::Once),
            )
            .await,
        );
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
        f.send_as(member, unlock_request(WRONG, VaultUnlockDuration::Once))
            .await
            .unwrap_err();
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
async fn the_vault_menu_on_a_kernel_with_no_vault_is_refused_like_a_locked_one() {
    let missing = Fixture::new(CredentialVaultUnlockPolicy::Ttl, false);
    let locked = Fixture::locked();
    let mut refusals = Vec::new();
    for f in [&missing, &locked] {
        let runtime = f.router.runtime_state.clone();
        let request = crate::local::ManageCredentialVaultRequest {
            session_id: f.session.clone(),
            agent_id: Some(f.agent.clone()),
        };
        let answer = crate::runtime::user_config_executor::execute_manage_credential_vault_request(
            &runtime, request,
        )
        .await;
        refusals.push(code_of(answer));
        assert!(f.no_prompt_is_open().await, "no prompt creates a Vault");
    }
    assert_eq!(refusals[0], refusals[1]);
    assert_eq!(refusals[0].0, "vault_locked");
    assert!(!missing.vault.exists(), "an agent never creates the Vault");
}

#[tokio::test]
async fn a_session_less_write_on_a_kernel_with_no_vault_asks_to_unlock_like_a_locked_one() {
    let missing = Fixture::new(CredentialVaultUnlockPolicy::Ttl, false);
    let (code, message) = code_of(missing.send(set_secret("value")).await);
    assert_eq!(code, "vault_locked");
    assert!(!message.contains("machine"), "{message}");
    assert!(!missing.vault.exists());
}

pub(super) fn caller_of(class: Option<KernelConnectionClass>, user: Option<&str>) -> KernelCaller {
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
        Err(DaemonError::VaultUnlock { code, .. }) if code.ends_with("owner_only")
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
async fn an_owners_agent_may_save_a_token_and_start_the_login() {
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
        assert!(!refused_as_owner_only(
            f.send_as(agent, sign_in(true)).await
        ));
    }
    // Other callers stay refused: a relay peer is nobody's person or agent.
    let peer = caller_of(
        Some(KernelConnectionClass::RelayPeer),
        Some("remote-kernel"),
    );
    assert!(refused_as_owner_only(f.send_as(peer, sign_in(true)).await));
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
async fn a_once_unlock_covers_the_next_write_only() {
    let f = Fixture::locked();
    f.unlock_for(PASSPHRASE, VaultUnlockDuration::Once)
        .await
        .unwrap();
    f.send(set_secret("first")).await.unwrap();
    assert!(!f.status().await.unlocked, "the write used the unlock up");
    let (code, _) = code_of(f.send(set_secret("second")).await);
    assert_eq!(code, "vault_locked");

    f.unlock_for(PASSPHRASE, VaultUnlockDuration::Once)
        .await
        .unwrap();
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

// MP-08 / MP-10 / MP-11: owner operations use the real admission/router path.
#[tokio::test]
async fn owner_vault_roundtrip_preserves_identity_and_audits_reveal() {
    crate::test_support::isolated_env_test!();
    use crate::local::{OwnerVaultRequest as Request, OwnerVaultResult as Result};
    let fixture = Fixture::locked();
    fixture.unlock(PASSPHRASE).await.unwrap();
    let send = |request| fixture.send(LocalDaemonRequest::OwnerVault(request));
    let response = send(Request::Create {
        name: "GitHub".into(),
        kind: "Password".into(),
        use_binding: crate::config::VaultUseBinding {
            target: crate::config::VaultUseTarget::Site {
                origin: "https://github.com".into(),
            },
            value_kind: Default::default(),
        },
        value: VaultPassphrase::new(SENTINEL),
    })
    .await
    .unwrap();
    let LocalDaemonResponse::OwnerVault {
        result: Result::Saved { id },
    } = response
    else {
        panic!("save not confirmed")
    };
    let list = fixture
        .send(LocalDaemonRequest::ListCredentials(
            crate::local::ListCredentialsRequest,
        ))
        .await
        .unwrap();
    assert!(!serde_json::to_string(&list).unwrap().contains(SENTINEL));
    let LocalDaemonResponse::CredentialsListed {
        credentials: entries,
    } = list
    else {
        panic!("list not confirmed")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]
            .metadata
            .as_ref()
            .unwrap()
            .use_binding
            .as_ref()
            .unwrap()
            .target,
        crate::config::VaultUseTarget::Site {
            origin: "https://github.com".into()
        }
    );
    assert_eq!(
        entries[0]
            .metadata
            .as_ref()
            .unwrap()
            .created_by_id
            .as_deref(),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    let reveal = send(Request::Reveal { id: id.clone() }).await.unwrap();
    assert!(!format!("{reveal:?}").contains(SENTINEL));
    let LocalDaemonResponse::OwnerVault {
        result: Result::Revealed { value, .. },
    } = reveal
    else {
        panic!("reveal not confirmed")
    };
    assert!(value.expose_secret() == SENTINEL);
    send(Request::Replace {
        id: id.clone(),
        value: VaultPassphrase::new("replacement-sentinel"),
    })
    .await
    .unwrap();
    let LocalDaemonResponse::CredentialsListed { credentials: after } = fixture
        .send(LocalDaemonRequest::ListCredentials(
            crate::local::ListCredentialsRequest,
        ))
        .await
        .unwrap()
    else {
        panic!("list not confirmed")
    };
    assert_eq!(after[0].id, entries[0].id);
    assert_eq!(after[0].description, entries[0].description);
    assert_eq!(
        after[0].metadata.as_ref().unwrap().use_binding,
        entries[0].metadata.as_ref().unwrap().use_binding
    );
    assert_eq!(
        after[0].metadata.as_ref().unwrap().created_at_ms,
        entries[0].metadata.as_ref().unwrap().created_at_ms
    );
    let LocalDaemonResponse::OwnerVault {
        result: Result::Activity { events, .. },
    } = send(Request::Activity { id: id.clone() }).await.unwrap()
    else {
        panic!("activity not confirmed")
    };
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.as_str())
            .collect::<Vec<_>>(),
        ["Created by owner", "Viewed by owner", "Replaced by owner"]
    );
    send(Request::Delete { id }).await.unwrap();
    let LocalDaemonResponse::CredentialsListed {
        credentials: entries,
    } = fixture
        .send(LocalDaemonRequest::ListCredentials(
            crate::local::ListCredentialsRequest,
        ))
        .await
        .unwrap()
    else {
        panic!("list not confirmed")
    };
    assert!(entries.is_empty());
}

// MP-08 / MP-09 / MP-10 / MP-11: a payment card is one structured Vault value. The owner's
// list carries its non-secret summary; number and security code stay in the Vault.
#[tokio::test]
async fn owner_vault_saves_a_payment_card_and_lists_only_its_summary() {
    crate::test_support::isolated_env_test!();
    use crate::local::{OwnerVaultRequest as Request, OwnerVaultResult as Result};
    // Published payment-provider test cards, never real ones.
    const NUMBER: &str = "4242424242424242";
    const OTHER: &str = "5555555555554444";
    const CODE: &str = "7316";
    fn no_card_data(value: &serde_json::Value) {
        match value {
            serde_json::Value::String(text) => assert!(
                !text.contains(NUMBER) && !text.contains(OTHER) && text != CODE,
                "card data in an output"
            ),
            serde_json::Value::Array(items) => items.iter().for_each(no_card_data),
            serde_json::Value::Object(fields) => {
                assert!(!fields.contains_key("security_code"));
                fields.values().for_each(no_card_data);
            }
            _ => {}
        }
    }
    let fixture = Fixture::locked();
    fixture.unlock(PASSPHRASE).await.unwrap();
    let send = |request| fixture.send(LocalDaemonRequest::OwnerVault(request));
    let card = |number: &str, year: u16| {
        serde_json::json!({"number": number, "expiry_month": 12, "expiry_year": year,
            "security_code": CODE, "name": "Ada Lovelace"})
        .to_string()
    };
    // One card serves every site: it is saved with no site, and each payment asks the owner.
    let any_site: crate::config::VaultUseBinding = serde_json::from_value(
        serde_json::json!({"target": {"kind": "any_site"}, "value_kind": "payment_card"}),
    )
    .unwrap();
    let create_for =
        |kind: &str, use_binding: crate::config::VaultUseBinding, value: String| Request::Create {
            name: "Travel card".into(),
            kind: kind.into(),
            use_binding,
            value: VaultPassphrase::new(value),
        };
    let create = |kind: &str, value: String| create_for(kind, any_site.clone(), value);
    // A card is not tied to one merchant, and only a card is usable at any site.
    let site_card = crate::config::VaultUseBinding {
        target: crate::config::VaultUseTarget::Site {
            origin: "https://shop.example".into(),
        },
        value_kind: crate::config::VaultValueKind::PaymentCard,
    };
    assert!(
        send(create_for("Payment card", site_card, card(NUMBER, 2099)))
            .await
            .is_err()
    );
    let any_site_secret: crate::config::VaultUseBinding = serde_json::from_value(
        serde_json::json!({"target": {"kind": "any_site"}, "value_kind": "secret"}),
    )
    .unwrap();
    assert!(
        send(create_for("Password", any_site_secret, "fixture".into()))
            .await
            .is_err()
    );
    // Refusals are calm fixed sentences that never echo what was typed.
    for bad in [
        card("4242424242424241", 2099),
        card("4242 4242 4242 4242", 2099),
        card(NUMBER, 2001),
        card(NUMBER, 2099).replace(CODE, "73a6"),
        "4242424242424242|12/99|7316".into(),
    ] {
        let error = send(create("Payment card", bad)).await.unwrap_err();
        let text = format!("{error} {error:?}");
        assert!(!text.contains("4242") && !text.contains(CODE), "{text}");
    }
    // A card is never saved under another kind.
    assert!(send(create("API key", card(NUMBER, 2099))).await.is_err());
    let LocalDaemonResponse::OwnerVault {
        result: Result::Saved { id },
    } = send(create("Payment card", card(NUMBER, 2099)))
        .await
        .unwrap()
    else {
        panic!("save not confirmed")
    };
    let list = || async {
        serde_json::to_value(
            fixture
                .send(LocalDaemonRequest::ListCredentials(
                    crate::local::ListCredentialsRequest,
                ))
                .await
                .unwrap(),
        )
        .unwrap()
    };
    let listed = list().await;
    no_card_data(&listed);
    let entry = &listed["CredentialsListed"]["credentials"][0];
    assert_eq!(
        entry["metadata"]["payment_card"],
        serde_json::json!({"last4": "4242", "expiry_month": 12, "expiry_year": 2099,
            "name": "Ada Lovelace", "brand": "Visa"})
    );
    assert_eq!(
        entry["metadata"]["use_binding"],
        serde_json::json!({"target": {"kind": "any_site"}, "value_kind": "payment_card"})
    );
    assert!(entry
        .get("allowed_hosts")
        .is_none_or(|hosts| hosts == &serde_json::json!([])));
    assert_eq!(entry["metadata"]["created_by_kind"], "owner_payment_card");
    // Agents see the entry, never its card details.
    let handles = serde_json::to_value(
        crate::secret::RuntimeSecretService::new(
            crate::credential::load_user_credentials().unwrap(),
        )
        .list_handles(),
    )
    .unwrap();
    no_card_data(&handles);
    assert!(!handles.to_string().contains("payment_card\":{"));
    assert!(!handles.to_string().contains("expiry"));
    no_card_data(&serde_json::to_value(fixture.status().await).unwrap());
    let activity = send(Request::Activity { id: id.clone() }).await.unwrap();
    no_card_data(&serde_json::to_value(&activity).unwrap());
    // The owner's own view is not masked: Reveal returns the canonical card.
    let LocalDaemonResponse::OwnerVault {
        result: Result::Revealed { value, .. },
    } = send(Request::Reveal { id: id.clone() }).await.unwrap()
    else {
        panic!("reveal not confirmed")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(value.expose_secret()).unwrap(),
        serde_json::from_str::<serde_json::Value>(&card(NUMBER, 2099)).unwrap()
    );
    // Replacing keeps the card structure and refreshes the summary.
    assert!(send(Request::Replace {
        id: id.clone(),
        value: VaultPassphrase::new("plain value"),
    })
    .await
    .is_err());
    // The name on the card is optional.
    send(Request::Replace {
        id,
        value: VaultPassphrase::new(
            serde_json::json!({"number": OTHER, "expiry_month": 12, "expiry_year": 2098,
                "security_code": CODE})
            .to_string(),
        ),
    })
    .await
    .unwrap();
    let listed = list().await;
    no_card_data(&listed);
    let summary = &listed["CredentialsListed"]["credentials"][0]["metadata"]["payment_card"];
    assert_eq!(
        summary,
        &serde_json::json!({"last4": "4444", "expiry_month": 12, "expiry_year": 2098,
            "brand": "Mastercard"})
    );
}

#[tokio::test]
async fn owner_vault_rejects_agents_other_users_and_locked_values() {
    use crate::local::OwnerVaultRequest;
    let fixture = Fixture::locked();
    assert!(fixture
        .send(LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Reveal {
            id: "entry".into()
        }))
        .await
        .is_err());
    fixture.unlock(PASSPHRASE).await.unwrap();
    for class in [
        KernelConnectionClass::KernelAgent,
        KernelConnectionClass::ExternalAgent,
        KernelConnectionClass::Host,
    ] {
        let caller =
            KernelCaller::for_source(&KernelCommandSource::LocalCli).with_connection_class(class);
        assert!(fixture
            .send_as(
                caller,
                LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Generate {
                    name: "Test".into(),
                    kind: "Password".into(),
                    use_binding: crate::config::VaultUseBinding {
                        target: crate::config::VaultUseTarget::Site {
                            origin: "https://example.org".into()
                        },
                        value_kind: Default::default()
                    }
                })
            )
            .await
            .is_err());
    }
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(KernelConnectionClass::Terminal);
    caller.user_id = Some("other-user".into());
    assert!(fixture
        .send_as(
            caller,
            LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Reveal { id: "entry".into() })
        )
        .await
        .is_err());
}

#[tokio::test]
async fn owner_vault_generates_without_returning_value_and_validates_inputs() {
    crate::test_support::isolated_env_test!();
    use crate::local::{OwnerVaultRequest as Request, OwnerVaultResult as Result};
    let fixture = Fixture::locked();
    fixture.unlock(PASSPHRASE).await.unwrap();
    let response = fixture
        .send(LocalDaemonRequest::OwnerVault(Request::Generate {
            name: "Generated".into(),
            kind: "Password".into(),
            use_binding: crate::config::VaultUseBinding {
                target: crate::config::VaultUseTarget::Site {
                    origin: "https://example.org".into(),
                },
                value_kind: Default::default(),
            },
        }))
        .await
        .unwrap();
    let LocalDaemonResponse::OwnerVault {
        result: Result::Saved { id },
    } = response
    else {
        panic!("save not confirmed")
    };
    let reveal = fixture
        .send(LocalDaemonRequest::OwnerVault(Request::Reveal { id }))
        .await
        .unwrap();
    let LocalDaemonResponse::OwnerVault {
        result: Result::Revealed { value, .. },
    } = reveal
    else {
        panic!("reveal not confirmed")
    };
    assert_eq!(value.expose_secret().len(), 32);
    for (name, usage) in [("", "example.org"), ("Name", "")] {
        assert!(fixture
            .send(LocalDaemonRequest::OwnerVault(Request::Create {
                name: name.into(),
                kind: "Password".into(),
                use_binding: crate::config::VaultUseBinding {
                    target: crate::config::VaultUseTarget::Site {
                        origin: usage.into()
                    },
                    value_kind: Default::default()
                },
                value: VaultPassphrase::new(SENTINEL)
            }))
            .await
            .is_err());
    }
}

#[test]
fn owner_vault_command_payload_and_debug_redact_values() {
    let request = LocalDaemonRequest::OwnerVault(crate::local::OwnerVaultRequest::Create {
        name: "Name".into(),
        kind: "Password".into(),
        use_binding: crate::config::VaultUseBinding {
            target: crate::config::VaultUseTarget::Site {
                origin: "https://example.org".into(),
            },
            value_kind: Default::default(),
        },
        value: VaultPassphrase::new(SENTINEL),
    });
    let command = KernelCommand::from_local_request("owner-vault-redaction", None, None, &request);
    assert!(!serde_json::to_string(&command.payload)
        .unwrap()
        .contains(SENTINEL));
    assert!(!format!("{request:?}").contains(SENTINEL));
}

#[test]
fn owner_vault_response_is_withheld_from_every_model_bound_projection() {
    let response = LocalDaemonResponse::OwnerVault {
        result: crate::local::OwnerVaultResult::Revealed {
            id: "entry".into(),
            value: VaultPassphrase::new(SENTINEL),
        },
    };
    let mut wire = serde_json::to_value(response).unwrap();
    let refusal = crate::runtime::external_response::project_response_value(&mut wire).unwrap_err();
    assert!(!refusal.to_string().contains(SENTINEL));
    let request = LocalDaemonRequest::OwnerVault(crate::local::OwnerVaultRequest::Reveal {
        id: "entry".into(),
    });
    // The payload used by history/traces never serializes the response value.
    let command = KernelCommand::from_local_request("owner-vault-reveal", None, None, &request);
    assert!(!serde_json::to_string(&command.payload)
        .unwrap()
        .contains(SENTINEL));
}
