//! Queue row 274: a Claude login reaches another machine only as the Vault setup token (carried by the
//! launch). A native-only login cannot cross, so a spawn on another machine with one is refused before
//! anything exists there, and the owner gets ONE card on this computer to create the setup token.
use super::*;
use crate::runtime::provider_auth_control::execute_start_provider_login_request;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

const SETUP_TOKEN_METHOD: &str = "setup_token";

/// What a spawn of a Claude agent on another machine needs from this kernel first.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum WorkerClaudeLogin {
    /// Not Claude, not yet configured, not signed in, or a Vault setup token (the launch carries it).
    Proceed,
    /// Only a native login exists: the owner creates the setup token once on this computer.
    NeedsSetupToken { profile_id: String, label: String },
}

pub(super) fn worker_claude_login(
    config: &crate::config::DaemonConfig,
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    request_owner: &str,
    provider: &str,
    reference: &str,
) -> Result<WorkerClaudeLogin, DaemonError> {
    if crate::provider::canonical_provider_family(provider) != Some("claude") {
        return Ok(WorkerClaudeLogin::Proceed);
    }
    let owner = crate::account_profile::provider_account_authority_owner_for_profile(
        config,
        profiles,
        request_owner,
        "claude",
        reference,
    )?;
    let Some(profile) = profiles.find(&owner, "claude", reference)? else {
        return Ok(WorkerClaudeLogin::Proceed);
    };
    if profile.auth_state != crate::account_profile::ProviderAccountAuthState::Authenticated {
        return Ok(WorkerClaudeLogin::Proceed);
    }
    let source = profiles.credential_source_profile_id(&owner, "claude", &profile.profile_id)?;
    if crate::provider::provider_account_credential_uses_vault(&owner, "claude", &source)? {
        Ok(WorkerClaudeLogin::Proceed)
    } else {
        Ok(WorkerClaudeLogin::NeedsSetupToken {
            profile_id: profile.profile_id,
            label: profile.label,
        })
    }
}

/// The account the spawn runs under. A spawn that names none keeps `None`, and the agent then
/// runs under its provider's default (`AgentInstance::provider_account_profile`), which is what
/// the worker is provisioned with. The Room's agent defaults are applied where a client builds
/// the request, never here, so the check must not read them.
fn spawned_account_reference(request: &crate::agent::CreateAgentRequest) -> &str {
    request
        .account_profile
        .as_deref()
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .unwrap_or("default")
}

fn worker_login_for_spawn(
    config: &crate::config::DaemonConfig,
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    request: &crate::agent::CreateAgentRequest,
) -> Result<WorkerClaudeLogin, DaemonError> {
    worker_claude_login(
        config,
        profiles,
        &request.owner_user_id,
        &request.provider,
        spawned_account_reference(request),
    )
}

/// The refusal names the one step and where it happens.
pub(super) fn setup_token_refusal(label: &str, profile_id: &str, card_raised: bool) -> DaemonError {
    let step = if card_raised {
        "A card is waiting on your computer: choose Create login and sign in to Claude there once"
    } else {
        "On your own computer sign in to Claude once to create the login (Provider Accounts: Create setup token, or /provider setup-token claude <account-profile> --run)"
    };
    DaemonError::LocalTransport {
        operation: "agent.spawn",
        message: format!(
            "Claude account {} ({profile_id}) is signed in only on your computer, so another machine cannot use it yet. {step}, then spawn the agent again. Nothing needs doing on the other machine.",
            if label.is_empty() { profile_id } else { label },
        ),
    }
}

impl KernelRuntimeState {
    /// The outcome of the sign-in a card started, as a notice to the requesting agent and its Room.
    fn notify_claude_login_outcome(&self, session_id: &str, agent_id: &str, message: String) {
        self.owned.record_notice_for_agent(
            session_id,
            None,
            Some(agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(session_id),
            message,
        );
    }

    /// Runs before a spawn on another machine. Errors refuse the spawn; nothing exists yet.
    pub(super) async fn prepare_claude_login_for_worker(
        &self,
        request: &crate::agent::CreateAgentRequest,
        worker_ref: &str,
    ) -> Result<(), DaemonError> {
        if crate::provider::canonical_provider_family(&request.provider) != Some("claude") {
            return Ok(());
        }
        // A refusal here is the existing spawn path's to report.
        let Ok(Some(_)) = self.resolve_peer_kernel(worker_ref).await else {
            return Ok(());
        };
        let config = self.config_snapshot().await;
        match worker_login_for_spawn(&config, &self.owned.provider_account_profiles, request)? {
            WorkerClaudeLogin::Proceed => Ok(()),
            WorkerClaudeLogin::NeedsSetupToken { profile_id, label } => {
                let card_raised = self
                    .raise_setup_token_card(request, &profile_id, &label)
                    .await;
                Err(setup_token_refusal(&label, &profile_id, card_raised))
            }
        }
    }

    /// ONE card per profile; false when no card could be raised (a spawn from a client with no agent).
    pub(super) async fn raise_setup_token_card(
        &self,
        request: &crate::agent::CreateAgentRequest,
        profile_id: &str,
        label: &str,
    ) -> bool {
        let Some(parent) = request.parent_agent_id.as_ref() else {
            return false;
        };
        let agent_id = parent.agent_id().to_string();
        let session_id = request.session_id.clone();
        let id = format!("claude-login-for-machines:{profile_id}");
        let Ok(session) = self.owned.session_store.get_session(&session_id) else {
            return false;
        };
        if session
            .active_interactions()
            .iter()
            .any(|interaction| interaction.id() == id)
        {
            return true;
        }
        if session.active_interaction_for_agent(&agent_id).is_some() {
            return false;
        }
        let name = if label.is_empty() { profile_id } else { label };
        let receiver = match self
            .create_runtime_interaction(
                &session_id,
                RuntimeInteraction::new(
                    &id,
                    &agent_id,
                    RuntimeInteractionKind::Choice,
                    RuntimeInteractionLevel::Warning,
                    Some("Create a Claude login your other machines can use".to_string()),
                    format!(
                        "Sign in to Claude once here to make {name} usable on your other machines."
                    ),
                    vec![
                        RuntimeInteractionChoice::new(
                            "create",
                            "Create login",
                            "create",
                            Some(RuntimeInteractionChoiceStyle::Primary),
                        ),
                        RuntimeInteractionChoice::new(
                            "cancel",
                            "Cancel",
                            "cancel",
                            Some(RuntimeInteractionChoiceStyle::Secondary),
                        ),
                    ],
                    None,
                    Some(600),
                    None,
                ),
            )
            .await
        {
            Ok(receiver) => receiver,
            Err(_) => return false,
        };
        let state = self.clone();
        let profile_id = profile_id.to_string();
        let owner = self.provider_account_authority_owner_user_id(&request.owner_user_id);
        tokio::spawn(async move {
            let resolution = tokio::time::timeout(Duration::from_secs(600), receiver).await;
            let _ = state.timeout_runtime_interaction(&session_id, &id).await;
            if !resolution.is_ok_and(|reply| {
                reply.is_ok_and(|reply| reply.choice_id.as_deref() == Some("create"))
            }) {
                return;
            }
            let started = execute_start_provider_login_request(
                &state,
                &owner,
                crate::local::StartProviderLoginRequest {
                    provider: "claude".to_string(),
                    account_profile: profile_id.clone(),
                    method: Some(SETUP_TOKEN_METHOD.to_string()),
                },
            )
            .await;
            let login = match started {
                Ok(crate::local::LocalDaemonResponse::ProviderLoginStarted { login })
                    if login.login_id.is_some() =>
                {
                    login
                }
                other => {
                    // The card is gone, so the agent and the Room hear it here. The cause goes to the log.
                    crate::logging::warn_with_fields(
                        "daemon.claude_login_for_machines",
                        "Claude sign-in could not start",
                        serde_json::json!({"profile_id": profile_id, "error": other.err().map(|error| error.to_string())}),
                    );
                    state.notify_claude_login_outcome(
                        &session_id,
                        &agent_id,
                        format!("Claude sign-in could not start for {profile_id}. Check that Claude is installed on this computer, then spawn the agent again to get the card back."),
                    );
                    return;
                }
            };
            let outcome = state
                .wait_for_account_login_state(
                    &session_id,
                    &agent_id,
                    &owner,
                    &format!("{id}:login"),
                    &login,
                )
                .await;
            let message = if outcome
                .is_ok_and(|state| state == crate::local::ProviderLoginProcessState::Succeeded)
            {
                format!("Claude login created for {profile_id}; other machines can use it now. Spawn the agent again.")
            } else {
                format!("Claude login for {profile_id} was not created; spawn the agent again to get the card back.")
            };
            state.notify_claude_login_outcome(&session_id, &agent_id, message);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CredentialVaultBackend;
    use crate::session::DEFAULT_LOCAL_USER_ID;

    fn fixture(name: &str) -> (std::path::PathBuf, crate::config::DaemonConfig) {
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-login-worker-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("CHARIOX_HOME", &root);
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("credentials.vault").display().to_string();
        crate::secret::unlock_chariox_encrypted_vault(
            root.join("credentials.vault"),
            "fixture passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        (root, config)
    }

    #[test]
    fn a_native_only_login_needs_the_setup_token_and_a_vault_login_proceeds() {
        let _env = crate::env_lock::lock();
        let (root, config) = fixture("decision");
        let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let profile = profiles
            .create_managed(DEFAULT_LOCAL_USER_ID, "claude", "Work")
            .unwrap();
        crate::test_support::authenticate_provider_account(
            &profiles,
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        let decide = |provider: &str, reference: &str| {
            worker_claude_login(
                &config,
                &profiles,
                DEFAULT_LOCAL_USER_ID,
                provider,
                reference,
            )
            .unwrap()
        };
        assert_eq!(
            decide("claude", &profile.profile_id),
            WorkerClaudeLogin::NeedsSetupToken {
                profile_id: profile.profile_id.clone(),
                label: "Work".into()
            }
        );
        assert_eq!(decide("codex", "x"), WorkerClaudeLogin::Proceed);
        assert_eq!(decide("claude", "missing"), WorkerClaudeLogin::Proceed);
        crate::provider::store_provider_account_credential(
            &config,
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "decision-token-canary",
            false,
        )
        .unwrap();
        // A login held as a Vault setup token is what the launch carries: no card, no refusal.
        assert_eq!(
            decide("claude", &profile.profile_id),
            WorkerClaudeLogin::Proceed
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_spawn_without_an_account_is_judged_by_the_account_it_will_run_under() {
        let _env = crate::env_lock::lock();
        let (root, config) = fixture("omitted-account");
        let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let make = |label: &str| {
            let profile = profiles
                .create_managed(DEFAULT_LOCAL_USER_ID, "claude", label)
                .unwrap();
            crate::test_support::authenticate_provider_account(
                &profiles,
                DEFAULT_LOCAL_USER_ID,
                "claude",
                &profile.profile_id,
            )
            .unwrap();
            profile
        };
        let native_only = make("Work");
        let with_token = make("Token");
        crate::provider::store_provider_account_credential(
            &config,
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &with_token.profile_id,
            "omitted-account-token-canary",
            false,
        )
        .unwrap();
        // A Room's agent defaults may name another account; they reach a spawn only through the
        // request a client builds, so an omitted account is the provider default.
        let request = crate::agent::CreateAgentRequest::new("room", "claude");
        assert!(request.account_profile.is_none());
        profiles
            .set_default(DEFAULT_LOCAL_USER_ID, "claude", &with_token.profile_id)
            .unwrap();
        assert_eq!(
            worker_login_for_spawn(&config, &profiles, &request).unwrap(),
            WorkerClaudeLogin::Proceed,
            "the worker is provisioned with the provider default, which carries its token"
        );
        profiles
            .set_default(DEFAULT_LOCAL_USER_ID, "claude", &native_only.profile_id)
            .unwrap();
        assert_eq!(
            worker_login_for_spawn(&config, &profiles, &request).unwrap(),
            WorkerClaudeLogin::NeedsSetupToken {
                profile_id: native_only.profile_id.clone(),
                label: "Work".into()
            },
            "a native-only default is the one that needs the card"
        );
        // An explicit account is the one that counts.
        let explicit = request
            .clone()
            .with_account_profile(with_token.profile_id.clone());
        assert_eq!(
            worker_login_for_spawn(&config, &profiles, &explicit).unwrap(),
            WorkerClaudeLogin::Proceed
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_profile_with_a_vault_login_no_longer_exports_its_native_credential() {
        let _env = crate::env_lock::lock();
        let (root, config) = fixture("export");
        let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let profile = profiles
            .create_managed(DEFAULT_LOCAL_USER_ID, "claude", "Work")
            .unwrap();
        let environment = profiles
            .resolve_environment(DEFAULT_LOCAL_USER_ID, "claude", &profile.profile_id)
            .unwrap();
        let dir = std::path::PathBuf::from(&environment["CLAUDE_CONFIG_DIR"]);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"native-access","refreshToken":"native-refresh","expiresAt":4102444800000}}"#,
        )
        .unwrap();
        let exported = |profiles: &crate::account_profile::ProviderAccountProfileRegistry| {
            profiles
                .export_materialization(DEFAULT_LOCAL_USER_ID, "claude", &profile.profile_id)
                .unwrap()
                .files
                .iter()
                .any(|file| file.relative_path == ".credentials.json")
        };
        // A native login alone is still exported: the worker refuses it by name.
        assert!(exported(&profiles));
        crate::provider::store_provider_account_credential(
            &config,
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "export-token-canary",
            false,
        )
        .unwrap();
        // With the setup token beside it, only the token crosses (carried by the launch).
        assert!(!exported(&profiles));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_refusal_names_the_one_step_and_where_it_happens() {
        for (card, expected) in [
            (true, "A card is waiting on your computer"),
            (false, "/provider setup-token claude"),
        ] {
            let text = setup_token_refusal("Work", "claude-1", card).to_string();
            assert!(text.contains(expected), "{text}");
            assert!(text.contains("on your computer") || text.contains("your own computer"));
            assert!(text.contains("Nothing needs doing on the other machine"));
            assert!(!text.contains("on that machine run"));
        }
    }
}
