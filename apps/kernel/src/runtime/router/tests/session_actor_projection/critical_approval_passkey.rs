//! Protocol 392: a critical approval needs the Chariox passkey (the vault
//! passphrase) or the owner's remember window; deny and routine decisions
//! need neither, and agent or Meta answers stay refused. Protocol 402: only
//! terminals may submit the passkey, and audits name the connection class.
use super::*;
use crate::durable_state::DurableKernelStateStore;
use crate::local::KernelConnectionClass;
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};
use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::RelayCallerIdentity;

const PASSKEY: &str = "correct horse battery";
const NEW_PASSKEY: &str = "Correct Horse Battery \u{c9}!";

struct Fixture {
    router: CommandRouter,
    durable: DurableKernelStateStore,
    session: String,
    /// The agent `/credential vault manage` runs for.
    agent: String,
    vault: std::path::PathBuf,
}

/// Pending interaction ids are process-wide: each test's ids are its own.
fn id(name: &str) -> String {
    format!(
        "{name}-{}",
        std::thread::current()
            .name()
            .unwrap_or("test")
            .rsplit("::")
            .next()
            .unwrap()
    )
}

impl Fixture {
    /// With `vault`, the encrypted vault exists (locked) with `PASSKEY`.
    fn new(vault: bool) -> Self {
        Self::build(vault, None)
    }

    /// The local owner hosts a session shared with `guest`.
    fn shared_with(guest: &str) -> Self {
        Self::build(true, Some(guest))
    }

    fn build(vault: bool, guest: Option<&str>) -> Self {
        let root = std::env::temp_dir().join(format!("passkey-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let vault_path = root.join("vault.json");
        if vault {
            crate::secret::create_chariox_encrypted_vault_for_test(&vault_path, PASSKEY).unwrap();
        }
        let mut config = DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config_path = root.join("config.toml");
        let app = DaemonApp::bootstrap(config).unwrap();
        let mut session = RuntimeSession::new(
            format!("passkey-{:016x}", rand::random::<u64>()),
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        if let Some(guest) = guest {
            session.add_member(
                guest,
                Some(DEFAULT_LOCAL_USER_ID.to_owned()),
                crate::session::CollaborationLevel::Full,
            );
        }
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
        let durable = app.durable_state_store();
        Self {
            router: CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1),
            durable,
            session: session_id,
            agent,
            vault: vault_path,
        }
    }

    /// A critical-action validation, as the App validation pump raises it.
    async fn critical(
        &self,
        id: &str,
    ) -> tokio::sync::oneshot::Receiver<crate::runtime::state::PendingInteractionResolution> {
        self.router
            .runtime_state
            .create_kernel_operation_interaction(
                &self.session,
                DEFAULT_LOCAL_USER_ID,
                RuntimeInteraction::for_kernel_operation(
                    self::id(id),
                    format!("validation:{}", self::id(id)),
                    "Approve App action",
                    "An App asks to perform a protected action.",
                    vec![
                        RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                        RuntimeInteractionChoice::new("approve", "Approve", "allow", None)
                            .requiring_passkey(),
                    ],
                ),
            )
            .await
            .unwrap()
    }

    async fn answer(
        &self,
        id: &str,
        choice: &str,
        passkey: Option<&str>,
        remember: Option<u32>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.answer_as(
            id,
            choice,
            passkey,
            remember,
            KernelCommandSource::LocalCli,
            local_caller(KernelConnectionClass::Terminal),
        )
        .await
    }

    /// An answer from a caller admitted on a given connection.
    async fn answer_as(
        &self,
        id: &str,
        choice: &str,
        passkey: Option<&str>,
        remember: Option<u32>,
        source: KernelCommandSource,
        caller: KernelCaller,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let request =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: self.session.clone(),
                interaction_id: self::id(id),
                choice_id: choice.into(),
                custom_reply: None,
                passkey: passkey.map(crate::local::ApprovalPasskey::new),
                passkey_remember_minutes: remember,
            });
        let command = KernelCommand::from_local_request_with_caller(
            format!("answer-{id}-{choice}-{}", rand::random::<u64>()),
            source,
            caller,
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await
    }

    /// Runs `/credential vault manage`, chooses Change passphrase and
    /// answers its three secret prompts through `RespondToInteraction`, as a
    /// client does.
    async fn change_passphrase(
        &self,
        entries: [&str; 3],
    ) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
        let agent = self.agent.clone();
        let runtime = self.router.runtime_state.clone();
        let session = self.session.clone();
        let manage_agent = agent.clone();
        let manage = tokio::spawn(async move {
            runtime
                .manage_credential_vault_unlock(&session, &manage_agent)
                .await
        });
        let answers = std::iter::once(("change_passphrase", None))
            .chain(entries.map(|entry| ("passphrase", Some(entry))));
        let mut answered = std::collections::BTreeSet::new();
        for (choice, reply) in answers {
            let prompt = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let session = self
                        .router
                        .runtime_state
                        .session_snapshot(&self.session)
                        .await;
                    if let Some(prompt) = session
                        .unwrap()
                        .active_interaction_for_agent(&agent)
                        .filter(|prompt| !answered.contains(prompt.id()))
                    {
                        break Some(prompt.id().to_owned());
                    }
                    if manage.is_finished() {
                        break None;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("the next vault prompt should appear");
            let Some(prompt) = prompt else { break };
            answered.insert(prompt.clone());
            let request = LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: self.session.clone(),
                    interaction_id: prompt.clone(),
                    choice_id: choice.into(),
                    custom_reply: reply.map(str::to_owned),
                    passkey: None,
                    passkey_remember_minutes: None,
                },
            );
            let command = KernelCommand::from_local_request_with_caller(
                format!("answer-{prompt}"),
                KernelCommandSource::LocalCli,
                local_caller(KernelConnectionClass::Terminal),
                None,
                None,
                &request,
            );
            self.router.dispatch(command, request).await.unwrap();
        }
        manage.await.unwrap()
    }

    fn rotations(&self) -> Vec<String> {
        self.durable
            .load_subject_events_by_kind("chariox-vault", "critical_approval.passkey_rotation", 50)
            .unwrap()
            .into_iter()
            .map(|event| {
                assert!(!event.payload.to_string().contains(PASSKEY));
                assert!(!event.payload.to_string().contains(NEW_PASSKEY));
                event.payload["outcome"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn refused_with(result: Result<LocalDaemonResponse, DaemonError>, code: &str) {
        let error = result.expect_err("answer should be refused").to_string();
        assert!(error.contains(code), "{error}");
    }

    fn outcomes(&self, id: &str) -> Vec<String> {
        self.audits(id)
            .into_iter()
            .map(|(outcome, _)| outcome)
            .collect()
    }

    /// Each audit's outcome and connection class; no audit holds a passkey.
    fn audits(&self, id: &str) -> Vec<(String, serde_json::Value)> {
        let events = self
            .durable
            .load_subject_events_by_kind(
                &format!("validation:{}", self::id(id)),
                "critical_approval.passkey",
                50,
            )
            .unwrap();
        events
            .into_iter()
            .map(|event| {
                // The outcome and attribution only: never a passkey.
                let mut keys = event
                    .payload
                    .as_object()
                    .unwrap()
                    .keys()
                    .collect::<Vec<_>>();
                keys.sort();
                assert_eq!(
                    keys,
                    [
                        "connection_class",
                        "interaction_id",
                        "outcome",
                        "owner",
                        "remember_minutes"
                    ]
                );
                assert!(!event.payload.to_string().contains(PASSKEY));
                (
                    event.payload["outcome"].as_str().unwrap().to_owned(),
                    event.payload["connection_class"].clone(),
                )
            })
            .collect()
    }

    async fn active(&self) -> usize {
        self.router
            .runtime_state
            .session_snapshot(&self.session)
            .await
            .unwrap()
            .active_interactions()
            .len()
    }
}

#[tokio::test]
async fn critical_approval_needs_the_verified_passkey_and_deny_needs_none() {
    let f = Fixture::new(true);
    let approved = f.critical("first").await;
    let denied = f.critical("second").await;
    Fixture::refused_with(
        f.answer("first", "approve", None, None).await,
        "PASSKEY_REQUIRED",
    );
    Fixture::refused_with(
        f.answer("first", "approve", Some("wrong"), None).await,
        "PASSKEY_REJECTED",
    );
    assert_eq!(f.active().await, 2);
    assert!(matches!(
        f.answer("first", "approve", Some(PASSKEY), None)
            .await
            .unwrap(),
        LocalDaemonResponse::InteractionResponded { .. }
    ));
    assert_eq!(
        approved.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    f.answer("second", "deny", None, None).await.unwrap();
    assert_eq!(denied.await.unwrap().choice_id.as_deref(), Some("deny"));
    assert_eq!(f.outcomes("first"), ["missing", "rejected", "verified"]);
    assert!(f.outcomes("second").is_empty());
    // The passkey never unlocked the vault.
    assert!(
        !crate::secret::chariox_encrypted_vault_status(&f.vault)
            .unwrap()
            .unlocked
    );
}

#[tokio::test]
async fn wrong_passkeys_are_rate_limited_until_the_lockout_ends() {
    let f = Fixture::new(true);
    let receiver = f.critical("guessed").await;
    for _ in 0..5 {
        Fixture::refused_with(
            f.answer("guessed", "approve", Some("guess"), None).await,
            "PASSKEY_REJECTED",
        );
    }
    // Locked out: even the right passkey is not checked.
    Fixture::refused_with(
        f.answer("guessed", "approve", Some(PASSKEY), None).await,
        "PASSKEY_RATE_LIMITED",
    );
    f.router
        .runtime_state
        .expire_critical_approval_presence_for_test(DEFAULT_LOCAL_USER_ID);
    f.answer("guessed", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    assert_eq!(
        receiver.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    assert_eq!(f.outcomes("guessed")[5..], ["rate_limited", "verified"]);
}

#[tokio::test]
async fn host_owner_routing_cannot_use_the_terminals_remember_window() {
    let f = Fixture::new(true);
    let first = f.critical("first").await;
    let second = f.critical("second").await;
    let third = f.critical("third").await;
    f.answer("first", "approve", Some(PASSKEY), Some(5))
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("approve"));
    Fixture::refused_with(
        f.answer_as(
            "second",
            "approve",
            None,
            None,
            KernelCommandSource::LocalCli,
            local_caller(KernelConnectionClass::Host),
        )
        .await,
        "PASSKEY_REQUIRED",
    );
    assert!(f.audits("second").is_empty());
    f.answer_as(
        "second",
        "deny",
        None,
        None,
        KernelCommandSource::LocalCli,
        local_caller(KernelConnectionClass::Host),
    )
    .await
    .unwrap();
    assert_eq!(second.await.unwrap().choice_id.as_deref(), Some("deny"));
    f.answer("third", "approve", None, None).await.unwrap();
    assert_eq!(third.await.unwrap().choice_id.as_deref(), Some("approve"));
    assert_eq!(
        f.audits("third"),
        [("remembered".into(), serde_json::json!("terminal"))]
    );
}

#[tokio::test]
async fn remember_window_is_opt_in_bounded_and_expires() {
    let f = Fixture::new(true);
    let _first = f.critical("first").await;
    let second = f.critical("second").await;
    let _third = f.critical("third").await;
    for minutes in [0, 16] {
        assert!(f
            .answer("first", "approve", Some(PASSKEY), Some(minutes))
            .await
            .is_err());
    }
    assert!(f.answer("first", "approve", None, Some(5)).await.is_err());
    // A verified passkey without the option opens no window.
    f.answer("first", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    Fixture::refused_with(
        f.answer("second", "approve", None, None).await,
        "PASSKEY_REQUIRED",
    );
    // With it, the owner's next critical approvals need no passkey...
    let fourth = f.critical("fourth").await;
    f.answer("fourth", "approve", Some(PASSKEY), Some(5))
        .await
        .unwrap();
    assert_eq!(fourth.await.unwrap().choice_id.as_deref(), Some("approve"));
    f.answer("second", "approve", None, None).await.unwrap();
    assert_eq!(second.await.unwrap().choice_id.as_deref(), Some("approve"));
    assert_eq!(f.outcomes("second"), ["missing", "remembered"]);
    // ...until the window ends.
    f.router
        .runtime_state
        .expire_critical_approval_presence_for_test(DEFAULT_LOCAL_USER_ID);
    Fixture::refused_with(
        f.answer("third", "approve", None, None).await,
        "PASSKEY_REQUIRED",
    );
}

#[tokio::test]
async fn routine_decisions_agent_meta_and_other_users_are_unchanged() {
    let f = Fixture::new(true);
    // A routine kernel decision (no passkey choice) is answered as before.
    let routine = f
        .router
        .runtime_state
        .create_kernel_operation_interaction(
            &f.session,
            DEFAULT_LOCAL_USER_ID,
            RuntimeInteraction::for_kernel_operation(
                id("routine"),
                format!("install:{}", id("routine")),
                "Install App?",
                "Review this release",
                vec![RuntimeInteractionChoice::new(
                    "allow", "Install", "allow", None,
                )],
            ),
        )
        .await
        .unwrap();
    f.answer("routine", "allow", None, None).await.unwrap();
    assert_eq!(routine.await.unwrap().choice_id.as_deref(), Some("allow"));
    // The Meta tool and other non-terminal answers stay refused, and another
    // user holding the passkey cannot answer either.
    let _critical = f.critical("critical").await;
    assert!(f
        .router
        .runtime_state
        .resolve_runtime_interaction(&f.session, &id("critical"), "approve", None)
        .await
        .is_err());
    assert!(f
        .router
        .runtime_state
        .answer_terminal_runtime_interaction(
            &f.session,
            &id("critical"),
            "approve",
            None,
            Some("other-user"),
            Some(&crate::local::ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .is_err());
    assert!(f.outcomes("critical").is_empty());
    assert_eq!(f.active().await, 1);
    // An agent's own question may not ask for the passkey.
    let agent_question = RuntimeInteraction::new(
        "agent-question",
        "agent-1",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Warning,
        None,
        "Enter your passkey",
        vec![RuntimeInteractionChoice::new("ok", "OK", "ok", None).requiring_passkey()],
        None,
        None,
        None,
    );
    let refused = f
        .router
        .runtime_state
        .create_runtime_interaction(&f.session, agent_question)
        .await
        .expect_err("an agent question may not require the passkey");
    assert!(refused.to_string().contains("passkey"), "{refused}");
}

#[tokio::test]
async fn critical_approval_fails_closed_without_the_vault() {
    let f = Fixture::new(false);
    let _receiver = f.critical("no-vault").await;
    Fixture::refused_with(
        f.answer("no-vault", "approve", Some(PASSKEY), None).await,
        "PASSKEY_UNAVAILABLE",
    );
    assert_eq!(f.outcomes("no-vault"), ["unavailable"]);
    f.answer("no-vault", "deny", None, None).await.unwrap();
}

#[tokio::test]
async fn a_swapped_vault_path_or_file_cannot_supply_the_passkey() {
    let f = Fixture::new(true);
    let forged = f.vault.with_file_name("forged.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&forged, "agent passphrase").unwrap();
    let first = f.critical("first").await;
    let second = f.critical("second").await;
    // A local caller points the live vault config at a vault it made...
    let request = LocalDaemonRequest::SetUserConfigValue(crate::local::SetUserConfigValueRequest {
        path: "credential_vault.path".into(),
        value: forged.display().to_string(),
    });
    let command = KernelCommand::from_local_request("forge-vault-path", None, None, &request);
    f.router.dispatch(command, request).await.unwrap();
    Fixture::refused_with(
        f.answer("first", "approve", Some("agent passphrase"), None)
            .await,
        "PASSKEY_REJECTED",
    );
    // The boot vault still decides, and the right passkey pins it.
    f.answer("first", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("approve"));
    // ...or replaces the vault file itself: the pin does not move.
    std::fs::copy(&forged, &f.vault).unwrap();
    Fixture::refused_with(
        f.answer("second", "approve", Some("agent passphrase"), None)
            .await,
        "PASSKEY_REJECTED",
    );
    f.answer("second", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    assert_eq!(second.await.unwrap().choice_id.as_deref(), Some("approve"));
}

#[tokio::test]
async fn a_vault_passphrase_change_rotates_the_passkey_and_ends_remember_windows() {
    let f = Fixture::new(true);
    crate::secret::unlock_chariox_encrypted_vault(
        &f.vault,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    // The passkey is pinned, and a remember window is open.
    let first = f.critical("first").await;
    f.answer("first", "approve", Some(PASSKEY), Some(5))
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("approve"));

    let differ = f.change_passphrase([PASSKEY, NEW_PASSKEY, "typo"]).await;
    let differ = differ.unwrap_err().to_string();
    assert!(differ.contains("differ"), "{differ}");
    let wrong = f
        .change_passphrase([NEW_PASSKEY, NEW_PASSKEY, NEW_PASSKEY])
        .await;
    let wrong = wrong.unwrap_err().to_string();
    assert!(wrong.contains("incorrect"), "{wrong}");
    let (status, action) = f
        .change_passphrase([PASSKEY, NEW_PASSKEY, NEW_PASSKEY])
        .await
        .unwrap();
    assert_eq!(action, "passphrase_changed");
    assert!(status.unlocked, "the vault stays unlocked");

    // The remember window ended; the old passphrase is no longer the passkey.
    let second = f.critical("second").await;
    Fixture::refused_with(
        f.answer("second", "approve", None, None).await,
        "PASSKEY_REQUIRED",
    );
    Fixture::refused_with(
        f.answer("second", "approve", Some(PASSKEY), None).await,
        "PASSKEY_REJECTED",
    );
    f.answer("second", "approve", Some(NEW_PASSKEY), None)
        .await
        .unwrap();
    assert_eq!(second.await.unwrap().choice_id.as_deref(), Some("approve"));
    assert_eq!(f.rotations(), ["rejected", "changed"]);
    // And the vault opens with the new passphrase only.
    crate::secret::lock_chariox_encrypted_vault(&f.vault).unwrap();
    let unlock = |passphrase| {
        crate::secret::unlock_chariox_encrypted_vault(
            &f.vault,
            passphrase,
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
    };
    assert!(unlock(PASSKEY).is_err());
    unlock(NEW_PASSKEY).unwrap();
    crate::secret::lock_chariox_encrypted_vault(&f.vault).unwrap();
}

#[tokio::test]
async fn wrong_current_passphrases_count_against_the_passkey_limit() {
    let f = Fixture::new(true);
    crate::secret::unlock_chariox_encrypted_vault(
        &f.vault,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    for _ in 0..5 {
        let wrong = f
            .change_passphrase(["guess", NEW_PASSKEY, NEW_PASSKEY])
            .await;
        let wrong = wrong.unwrap_err().to_string();
        assert!(wrong.contains("incorrect"), "{wrong}");
    }
    // Locked out: neither a change nor a critical approval checks the passkey.
    let limited = f
        .change_passphrase([PASSKEY, NEW_PASSKEY, NEW_PASSKEY])
        .await;
    let limited = limited.unwrap_err().to_string();
    assert!(limited.contains("PASSKEY_RATE_LIMITED"), "{limited}");
    let _receiver = f.critical("locked").await;
    Fixture::refused_with(
        f.answer("locked", "approve", Some(PASSKEY), None).await,
        "PASSKEY_RATE_LIMITED",
    );
    assert_eq!(f.rotations()[5..], ["rate_limited"]);
    crate::secret::lock_chariox_encrypted_vault(&f.vault).unwrap();
}

fn local_caller(class: KernelConnectionClass) -> KernelCaller {
    KernelCaller::for_source(&KernelCommandSource::LocalCli).with_connection_class(class)
}

fn relay_caller(subject_kind: RelaySubjectKind) -> KernelCaller {
    KernelCaller::from_relay_identity(RelayCallerIdentity {
        realm_id: "realm-1".into(),
        subject: "subject-1".into(),
        subject_kind,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(DEFAULT_LOCAL_USER_ID.into()),
        public_key_thumbprint: None,
    })
}

#[tokio::test]
async fn critical_approval_audits_name_the_answering_connection_class() {
    let f = Fixture::new(true);
    let local = f.critical("local").await;
    let relayed = f.critical("relayed").await;
    let terminal = || local_caller(KernelConnectionClass::Terminal);
    let cli = KernelCommandSource::LocalCli;
    Fixture::refused_with(
        f.answer_as("local", "approve", None, None, cli.clone(), terminal())
            .await,
        "PASSKEY_REQUIRED",
    );
    Fixture::refused_with(
        f.answer_as(
            "local",
            "approve",
            Some("guess"),
            None,
            cli.clone(),
            terminal(),
        )
        .await,
        "PASSKEY_REJECTED",
    );
    // A terminal presenting the local token.
    f.answer_as(
        "local",
        "approve",
        Some(PASSKEY),
        None,
        cli,
        local_caller(KernelConnectionClass::Terminal),
    )
    .await
    .unwrap();
    assert_eq!(local.await.unwrap().choice_id.as_deref(), Some("approve"));
    // A relay client with the owner's user id (web, remote TUI).
    f.answer_as(
        "relayed",
        "approve",
        Some(PASSKEY),
        None,
        KernelCommandSource::RelayClient,
        relay_caller(RelaySubjectKind::Client),
    )
    .await
    .unwrap();
    assert_eq!(relayed.await.unwrap().choice_id.as_deref(), Some("approve"));

    let audit = |outcome: &str, class: &str| (outcome.to_owned(), serde_json::json!(class));
    assert_eq!(
        f.audits("local"),
        [
            audit("missing", "terminal"),
            audit("rejected", "terminal"),
            audit("verified", "terminal"),
        ]
    );
    assert_eq!(f.audits("relayed"), [audit("verified", "terminal")]);
}

#[tokio::test]
async fn a_passkey_from_a_refused_class_is_neither_verified_nor_counted() {
    let f = Fixture::new(true);
    let receiver = f.critical("guarded").await;
    let cli = KernelCommandSource::LocalCli;
    // A04: kernel agents are refused before any passkey handling.
    let refused = [
        (
            cli.clone(),
            local_caller(KernelConnectionClass::Unauthenticated),
            "PASSKEY_NOT_ACCEPTED",
        ),
        (
            cli.clone(),
            local_caller(KernelConnectionClass::Host),
            "PASSKEY_NOT_ACCEPTED",
        ),
        (
            cli.clone(),
            local_caller(KernelConnectionClass::KernelAgent),
            "agents cannot answer approvals",
        ),
        (
            cli.clone(),
            local_caller(KernelConnectionClass::ExternalAgent),
            "PASSKEY_NOT_ACCEPTED",
        ),
        (
            KernelCommandSource::RelayClient,
            relay_caller(RelaySubjectKind::Kernel),
            "PASSKEY_NOT_ACCEPTED",
        ),
    ];
    // Far more wrong passkeys than the owner's free failures, and the right
    // one: none is verified, audited or counted.
    for _ in 0..2 {
        for (source, caller, code) in &refused {
            for passkey in ["guess", PASSKEY] {
                Fixture::refused_with(
                    f.answer_as(
                        "guarded",
                        "approve",
                        Some(passkey),
                        None,
                        source.clone(),
                        caller.clone(),
                    )
                    .await,
                    code,
                );
            }
        }
    }
    // A stray passkey on a deny from these classes is refused as well.
    Fixture::refused_with(
        f.answer_as(
            "guarded",
            "deny",
            Some("guess"),
            None,
            cli.clone(),
            local_caller(KernelConnectionClass::Host),
        )
        .await,
        "PASSKEY_NOT_ACCEPTED",
    );
    assert!(f.outcomes("guarded").is_empty());
    assert_eq!(f.active().await, 1);
    // Protocol 403: the popup stays open on the owner's terminals.
    assert_eq!(f.prompt_ids(), [id("guarded")]);
    // The owner is not locked out: a terminal's passkey is verified at once.
    f.answer_as(
        "guarded",
        "approve",
        Some(PASSKEY),
        None,
        cli,
        local_caller(KernelConnectionClass::Terminal),
    )
    .await
    .unwrap();
    assert_eq!(
        receiver.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    assert_eq!(
        f.audits("guarded"),
        [("verified".to_owned(), serde_json::json!("terminal"))]
    );
}

mod passkey_prompts;
