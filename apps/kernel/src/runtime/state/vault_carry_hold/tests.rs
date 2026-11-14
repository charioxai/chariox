use super::super::provider_launch_owned_state::owned_runtime_state;
use super::*;
use crate::secret::{
    chariox_encrypted_vault_status, lock_chariox_encrypted_vault, unlock_chariox_encrypted_vault,
    VaultUnlockLease,
};
use std::sync::Arc;
use tokio::sync::Mutex;

const PASSPHRASE: &str = "correct horse battery staple";

struct Fixture {
    root: PathBuf,
    vault_path: PathBuf,
    config: crate::config::DaemonConfig,
    registry: crate::account_profile::ProviderAccountProfileRegistry,
    profile_id: String,
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    other_session_id: String,
    other_agent_id: String,
    request: crate::local::PreflightProviderAccountPortabilityRequest,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-vault-carry-hold-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        let vault_path = root.join("credentials.vault");
        let mut config = crate::config::DaemonConfig::for_tests()
            .with_session_history_root(root.join("session-history"));
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Always;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let (other_session, other_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("second session should create");
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Vaulted Claude",
            )
            .expect("managed Claude profile should create");
        unlock_chariox_encrypted_vault(&vault_path, PASSPHRASE, VaultUnlockLease::KernelShutdown)
            .expect("vault should initialize");
        crate::provider::store_provider_account_credential(
            &config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "FAKE-hold-token-0123456789",
            false,
        )
        .expect("provider credential should store");
        crate::test_support::authenticate_provider_account(
            &registry,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        lock_chariox_encrypted_vault(&vault_path).expect("vault should lock");
        crate::secret::clear_vault_secret_process_cache().expect("secret cache should clear");
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::local::PreflightProviderAccountPortabilityRequest {
            provider_accounts: crate::local::ProviderAccountTransferSelection::Selected {
                accounts: vec![crate::local::ProviderAccountTransferProfile {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                }],
            },
            launch_id: None,
        };
        Self {
            root,
            vault_path,
            config,
            registry,
            profile_id: profile.profile_id,
            runtime,
            session_id: session.id().to_owned(),
            agent_id: agent.id().to_owned(),
            other_session_id: other_session.id().to_owned(),
            other_agent_id: other_agent.id().to_owned(),
            request,
        }
    }

    fn command(&self) -> crate::runtime::command::KernelCommand {
        self.command_in(&self.session_id)
    }

    fn command_in(&self, session_id: &str) -> crate::runtime::command::KernelCommand {
        let mut command = crate::runtime::command::KernelCommand::from_local_request(
            "preflight",
            None,
            None,
            &crate::local::LocalDaemonRequest::PreflightProviderAccountPortability(
                self.request.clone(),
            ),
        );
        command.session_id = Some(session_id.to_owned());
        command
    }

    async fn preflight(&self) {
        self.preflight_in(&self.session_id).await;
    }

    async fn preflight_in(&self, session_id: &str) {
        self.launch_in(session_id, None).await;
    }

    /// Preflight of one launch, named by `launch_id`, from the room `session_id`.
    async fn launch_in(&self, session_id: &str, launch_id: Option<&str>) {
        let request = crate::local::PreflightProviderAccountPortabilityRequest {
            launch_id: launch_id.map(str::to_owned),
            ..self.request.clone()
        };
        crate::runtime::provider_account_portability::execute_preflight(
            &self.runtime,
            &self.config,
            &self.registry,
            &self.command_in(session_id),
            &request,
        )
        .await
        .expect("preflight passes");
    }

    fn unlocked(&self) -> bool {
        chariox_encrypted_vault_status(&self.vault_path)
            .expect("vault status should resolve")
            .unlocked
    }

    /// What the export reads: the login through the Vault, which fails while it is locked.
    fn login_readable(&self) -> bool {
        crate::provider::resolve_provider_account_credentials(
            &self.config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &self.profile_id,
        )
        .is_ok()
    }

    fn card(&self) -> Option<crate::session::RuntimeInteraction> {
        self.runtime
            .owned
            .session_store
            .get_session(&self.session_id)
            .ok()?
            .active_interaction_for_agent(&self.agent_id)
            .cloned()
    }

    /// Drives `future` to its end, answering the Vault unlock card it shows with "once".
    async fn answering_the_card<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        let card = loop {
            tokio::select! {
                _ = &mut future => panic!("finished before the Vault unlock card was answered"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            }
            if let Some(card) = self.card() {
                break card;
            }
        };
        assert_eq!(card.title(), Some("Unlock Chariox Vault"));
        self.runtime
            .resolve_terminal_runtime_interaction(
                &self.session_id,
                card.id(),
                "unlock_once",
                Some(PASSPHRASE),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .expect("vault passphrase interaction should resolve");
        future.await
    }

    async fn preflight_answering_the_card(&self) {
        self.answering_the_card(self.preflight()).await;
    }

    fn card_in(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<crate::session::RuntimeInteraction> {
        self.runtime
            .owned
            .session_store
            .get_session(session_id)
            .ok()?
            .active_interaction_for_agent(agent_id)
            .cloned()
    }

    /// Drives `future` until its unlock card shows; the card must be in `room` and not in
    /// `other`. Answers it there with "once".
    async fn answering_in<T>(
        &self,
        room: (&str, &str),
        other: (&str, &str),
        future: impl std::future::Future<Output = T>,
    ) -> T {
        let mut future = Box::pin(future);
        let card = loop {
            tokio::select! {
                _ = &mut future => panic!("finished before the Vault unlock card was answered"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            }
            assert!(
                self.card_in(other.0, other.1).is_none(),
                "the unlock card is shown in the room of another launch"
            );
            if let Some(card) = self.card_in(room.0, room.1) {
                break card;
            }
        };
        self.runtime
            .resolve_terminal_runtime_interaction(
                room.0,
                card.id(),
                "unlock_once",
                Some(PASSPHRASE),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .expect("vault passphrase interaction should resolve");
        future.await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = lock_chariox_encrypted_vault(&self.vault_path);
        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn the_unlock_taken_at_preflight_is_held_for_the_export_and_used_once() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;

    fx.preflight_answering_the_card().await;
    assert!(
        fx.unlocked() && fx.login_readable(),
        "the unlock outlives preflight until the export"
    );

    let export = fx
        .runtime
        .vault_for_login_export(Some(&fx.session_id), None)
        .await
        .expect("the export takes the held unlock without asking again");
    assert!(fx.login_readable(), "the export reads the login");
    drop(export);
    assert!(!fx.unlocked(), "the one use ends with the export");
}

#[tokio::test]
async fn a_second_export_asks_again_instead_of_reusing_a_used_unlock() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    fx.preflight_answering_the_card().await;
    drop(
        fx.runtime
            .vault_for_login_export(Some(&fx.session_id), None)
            .await
            .unwrap(),
    );
    assert!(!fx.unlocked());

    let export = fx
        .answering_the_card(
            fx.runtime
                .vault_for_login_export(Some(&fx.session_id), None),
        )
        .await
        .expect("a used unlock is asked for again through the unlock card");
    assert!(fx.login_readable());
    drop(export);
    assert!(!fx.unlocked());
}

#[tokio::test]
async fn a_lapsed_hold_ends_by_itself_and_the_export_asks_again() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    let command = fx.command();
    let unlocked = fx
        .answering_the_card(fx.runtime.ensure_vault_unlocked_for_command_context(
            &command,
            None,
            None,
            "provider_account_portability",
        ))
        .await
        .expect("unlocked once");
    fx.runtime.hold_vault_until(
        unlocked,
        &command,
        None,
        Instant::now() + std::time::Duration::from_millis(150),
    );
    assert!(fx.unlocked(), "held while the hold lives");

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        !fx.unlocked(),
        "the held unlock does not outlive its bounded lifetime"
    );

    let export = fx
        .answering_the_card(
            fx.runtime
                .vault_for_login_export(Some(&fx.session_id), None),
        )
        .await
        .expect("a lapsed hold asks again through the unlock card");
    assert!(fx.login_readable());
    drop(export);
    assert!(!fx.unlocked());
}

#[tokio::test]
async fn a_client_ask_every_time_unlock_is_held_past_its_minute() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    unlock_chariox_encrypted_vault(&fx.vault_path, PASSPHRASE, VaultUnlockLease::SingleUse)
        .expect("the client's single-use unlock");

    fx.preflight().await;
    let expires_at_ms = chariox_encrypted_vault_status(&fx.vault_path)
        .unwrap()
        .expires_at_ms
        .expect("the held unlock is bounded");
    assert!(
        expires_at_ms > crate::session::unix_epoch_ms() + 5 * 60_000,
        "the minute of a single-use unlock does not cover machine creation"
    );
    assert!(
        expires_at_ms <= crate::session::unix_epoch_ms() + 11 * 60_000,
        "the hold is bounded"
    );
    let export = fx
        .runtime
        .vault_for_login_export(Some(&fx.session_id), None)
        .await
        .unwrap();
    assert!(fx.login_readable());
    drop(export);
    assert!(!fx.unlocked());
}

#[tokio::test]
async fn a_normal_lease_is_neither_extended_nor_closed_by_the_export() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    unlock_chariox_encrypted_vault(&fx.vault_path, PASSPHRASE, VaultUnlockLease::TtlMinutes(30))
        .expect("a 30 minute unlock");
    let before = chariox_encrypted_vault_status(&fx.vault_path)
        .unwrap()
        .expires_at_ms;

    fx.preflight().await;
    let export = fx
        .runtime
        .vault_for_login_export(Some(&fx.session_id), None)
        .await
        .unwrap();
    assert!(fx.login_readable());
    drop(export);

    assert!(fx.unlocked(), "the lease the owner chose stays open");
    assert!(fx.card().is_none(), "nothing was asked");
    assert_eq!(
        chariox_encrypted_vault_status(&fx.vault_path)
            .unwrap()
            .expires_at_ms,
        before,
        "the expiry is the lease's own"
    );
}

// MP-10 / MP-11: a later launch in another session must not take over the first launch's ask.
#[tokio::test]
async fn two_launches_in_two_sessions_each_keep_their_own_ask() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    fx.preflight_answering_the_card().await;
    // The Vault is open on the first hold, so the second launch shows no card.
    fx.preflight_in(&fx.other_session_id).await;
    drop(
        fx.runtime
            .vault_for_login_export(Some(&fx.session_id), None)
            .await
            .unwrap(),
    );
    assert!(!fx.unlocked());

    let asked = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fx.answering_the_card(
            fx.runtime
                .vault_for_login_export(Some(&fx.session_id), None),
        ),
    )
    .await
    .expect("the first launch's export asks in the first launch's session");
    drop(asked.expect("the unlock card opens the Vault"));
    let other = fx
        .runtime
        .owned
        .session_store
        .get_session(&fx.other_session_id)
        .unwrap();
    assert!(other.active_interactions().is_empty());
}

// MP-10 / MP-11: a restart drops the in-memory holds; the export must end, not wait for a card.
#[tokio::test]
async fn an_export_after_a_restart_ends_with_the_locked_refusal_instead_of_waiting() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fx = Fixture::new().await;
    fx.preflight_answering_the_card().await;
    drop(
        fx.runtime
            .vault_for_login_export(Some(&fx.session_id), None)
            .await
            .unwrap(),
    );
    locked_entries().clear();
    assert!(!fx.unlocked());
    let ended = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fx.runtime
            .vault_for_login_export(Some(&fx.session_id), None),
    )
    .await
    .expect("no unlock card is awaited after a restart");
    assert!(matches!(
        ended,
        Err(DaemonError::VaultUnlock {
            retryable: true,
            ..
        })
    ));
    assert!(fx.card().is_none());
}

// MP-10 / MP-11: two launches in flight, each export named only by its launch key. Whichever
// machine is ready first, the unlock card of each export is shown in its own launch's room.
#[tokio::test]
async fn each_export_asks_in_the_room_of_its_launch_in_either_ready_order() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    for a_ready_first in [true, false] {
        let fx = Fixture::new().await;
        let first = (fx.session_id.as_str(), fx.agent_id.as_str());
        let second = (fx.other_session_id.as_str(), fx.other_agent_id.as_str());
        fx.answering_the_card(fx.launch_in(first.0, Some("launch-a")))
            .await;
        // The Vault is open on the first hold, so the second launch shows no card.
        fx.launch_in(second.0, Some("launch-b")).await;
        // Both holds lapse before either machine is ready, so each export has to ask.
        let lapsed: Vec<Held> = locked_entries()
            .values_mut()
            .filter_map(|entry| entry.held.take())
            .collect();
        drop(lapsed);
        assert!(!fx.unlocked());
        let exports = if a_ready_first {
            [("launch-a", first, second), ("launch-b", second, first)]
        } else {
            [("launch-b", second, first), ("launch-a", first, second)]
        };
        for (launch, room, other) in exports {
            let export = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                fx.answering_in(
                    room,
                    other,
                    fx.runtime.vault_for_login_export(None, Some(launch)),
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("{launch} asks in its own room"));
            drop(export.expect("the unlock card opens the Vault"));
            assert!(!fx.unlocked());
        }
    }
}
