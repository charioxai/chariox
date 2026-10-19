//! MP-08/MP-10/MP-11 V03: revocable, in-memory consent for one login task.
use super::runtime_vault_unlock_state::expand_vault_path;
use super::*;
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct LoginScope {
    pub owner: String,
    pub session: String,
    pub agent: String,
    pub credential: String,
    pub app: String,
}
#[derive(Clone)]
pub(super) struct LoginGrant {
    id: String,
    pub scope: LoginScope,
    prompts: BTreeSet<String>,
    deadline: Instant,
}
#[derive(Clone, Default)]
pub(super) struct ComputerLoginGrants(Arc<std::sync::Mutex<BTreeMap<String, LoginGrant>>>);
impl ComputerLoginGrants {
    pub fn find(&self, scope: &LoginScope, prompt: &str) -> Option<LoginGrant> {
        self.0
            .lock()
            .unwrap()
            .values()
            .find(|g| {
                &g.scope == scope && g.prompts.contains(prompt) && Instant::now() < g.deadline
            })
            .cloned()
    }
    pub fn insert(&self, scope: LoginScope, prompt: String, seconds: u64) -> LoginGrant {
        let grant = LoginGrant {
            id: format!("login-{:032x}", rand::random::<u128>()),
            scope,
            prompts: BTreeSet::from([prompt]),
            deadline: Instant::now() + Duration::from_secs(seconds),
        };
        let mut grants = self.0.lock().unwrap();
        grants.retain(|_, g| Instant::now() < g.deadline);
        // Consent is never silently widened if many unfinished tasks accumulate.
        if grants.len() >= 256 {
            grants.clear();
        }
        grants.insert(grant.id.clone(), grant.clone());
        grant
    }
    pub fn live(&self, grant: &LoginGrant, prompt: &str) -> bool {
        self.0.lock().unwrap().get(&grant.id).is_some_and(|g| {
            g.scope == grant.scope && g.prompts.contains(prompt) && Instant::now() < g.deadline
        })
    }
    pub fn extend_catalog(&self, session: &str, agent: &str, source: &str, next: &str) {
        for grant in self.0.lock().unwrap().values_mut().filter(|g| {
            g.scope.session == session
                && g.scope.agent == agent
                && g.prompts.contains(source)
                && !g.scope.app.starts_with("room:")
                && Instant::now() < g.deadline
        }) {
            grant.prompts.insert(next.to_owned());
        }
    }
    pub fn reserves_prompt(&self, session: &str, agent: &str, prompt: &str) -> bool {
        self.0.lock().unwrap().values().any(|g| {
            g.scope.session == session
                && g.scope.agent == agent
                && g.prompts.contains(prompt)
                && Instant::now() < g.deadline
        })
    }
    pub fn revoke_all(&self) {
        self.0.lock().unwrap().clear();
    }
}

pub(super) struct ComputerSecretApproval {
    pub sudo_turn: Option<crate::local::KernelSudoTurn>,
    grant: Option<LoginGrant>,
    // Keep an operation-only unlock alive through physical input.
    _unlock: Option<runtime_vault_unlock_state::VaultUnlockGuard>,
}
impl ComputerSecretApproval {
    pub fn has_unlock(&self) -> bool {
        self._unlock.is_some()
    }
    pub fn authority(&self) -> &'static str {
        if self.sudo_turn.is_some() {
            "sudo"
        } else if self
            .grant
            .as_ref()
            .is_some_and(|g| g.scope.app.starts_with("room:"))
        {
            "user_interaction"
        } else {
            "login_grant"
        }
    }
}
impl KernelRuntimeState {
    pub(crate) fn revoke_computer_login_grants(&self) {
        self.owned.computer_login_grants.revoke_all();
    }
    pub(super) fn computer_secret_approval_live(&self, approval: &ComputerSecretApproval) -> bool {
        if let Some(turn) = &approval.sudo_turn {
            return self.sudo_turn_live(turn);
        }
        let Some(grant) = &approval.grant else {
            return false;
        };
        let Ok(session) = self.owned.session_store.get_session(&grant.scope.session) else {
            return false;
        };
        let Ok(agent) = self.owned.agent_store.get_agent(&grant.scope.agent) else {
            return false;
        };
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
        else {
            return false;
        };
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        let unlocked = config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted
            || crate::secret::chariox_encrypted_vault_status(expand_vault_path(&config.path))
                .is_ok_and(|s| s.unlocked);
        unlocked
            && session.status() != crate::session::SessionStatus::Ended
            && session.owner_user_id() == grant.scope.owner
            && agent.owner_user_id() == grant.scope.owner
            && agent.session_id() == session.id()
            && prompt.status() == crate::session::PromptStatus::Running
            && self.owned.computer_login_grants.live(grant, prompt.id())
    }

    pub(super) async fn ensure_computer_login_approved(
        &self,
        session_id: &str,
        agent_id: &str,
        credential: &str,
        app: &str,
        description: &str,
    ) -> Result<ComputerSecretApproval, DaemonError> {
        if let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        {
            if let Ok(turn) = self.sudo_for_provider_run(run.id()) {
                return Ok(ComputerSecretApproval {
                    sudo_turn: Some(turn),
                    grant: None,
                    _unlock: None,
                });
            }
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id || agent.owner_user_id() != session.owner_user_id() {
            return Err(login_error("login task owner changed"));
        }
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .filter(|p| p.status() == crate::session::PromptStatus::Running)
            .ok_or_else(|| login_error("an active login task is required"))?;
        let scope = LoginScope {
            owner: agent.owner_user_id().into(),
            session: session_id.into(),
            agent: agent_id.into(),
            credential: credential.into(),
            app: app.into(),
        };
        // Room's legacy target has XIDs, not a process/start identity. Never
        // widen one operation's consent across an app restart/XID reuse.
        let reusable_native_app = !app.starts_with("room:");
        if let Some(grant) = self
            .owned
            .computer_login_grants
            .find(&scope, prompt.id())
            .filter(|_| reusable_native_app)
        {
            let approval = ComputerSecretApproval {
                sudo_turn: None,
                grant: Some(grant),
                _unlock: None,
            };
            if self.computer_secret_approval_live(&approval) {
                return Ok(approval);
            }
        }
        let purpose = if reusable_native_app {
            format!("Allow agent `{agent_id}` to use credential `{credential}` for this login task in {description}. Further pastes for this task and app need no new consent. Revoke in Vault or lock it at any time.")
        } else {
            format!("Allow agent `{agent_id}` to paste credential `{credential}` into {description}. This consent covers this paste. The chosen unlock duration controls the Vault lease.")
        };
        let (unlock, unlocked_choice) = self
            .ensure_vault_unlocked_for_agent_with_purpose(
                session_id,
                agent_id,
                "computer_login",
                Some(&purpose),
            )
            .await?;
        let via_unlock = unlocked_choice.is_some();
        let choice = if let Some(choice) = unlocked_choice {
            choice
        } else {
            let choices = vec![
                crate::session::RuntimeInteractionChoice::new(
                    "unlock_default_ttl",
                    if reusable_native_app {
                        "Allow this login for 15 minutes"
                    } else {
                        "Allow this paste"
                    },
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "unlock_60m",
                    "Allow this login for 1 hour",
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "cancel",
                    "Cancel",
                    "cancel",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ];
            let choices = if reusable_native_app {
                choices
            } else {
                choices
                    .into_iter()
                    .filter(|c| c.id() != "unlock_60m")
                    .collect()
            };
            let interaction = crate::session::RuntimeInteraction::new(
                format!(
                    "computer-login-{agent_id}-{}",
                    crate::session::unix_epoch_ms()
                ),
                agent_id,
                crate::session::RuntimeInteractionKind::Choice,
                crate::session::RuntimeInteractionLevel::Critical,
                Some(
                    if reusable_native_app {
                        "Allow this login"
                    } else {
                        "Allow this paste"
                    }
                    .into(),
                ),
                purpose,
                choices,
                None,
                Some(300),
                Some("cancel".into()),
            );
            let result = self
                .await_vault_interaction(session_id, interaction, "computer_login", "login consent")
                .await?;
            if result.status != "answered"
                || !matches!(
                    result.choice_id.as_deref(),
                    Some("unlock_default_ttl" | "unlock_60m")
                )
            {
                return Err(login_error("login consent was cancelled or timed out"));
            }
            result.choice_id.unwrap()
        };
        // The trusted sheet can wait: never move its consent onto replacement work.
        let current = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&current, agent_id)
            .is_none_or(|p| {
                p.id() != prompt.id() || p.status() != crate::session::PromptStatus::Running
            })
            || !unlock.still_unlocked()
        {
            return Err(login_error(
                "login task or Vault unlock ended during consent",
            ));
        }
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        let minutes = match choice.as_str() {
            "unlock_60m" => 60,
            "unlock_kernel" => config.max_ttl_minutes,
            "unlock_operation" => 1,
            "passphrase" => config.default_ttl_minutes,
            "unlock_default_ttl" if via_unlock => config.default_ttl_minutes,
            _ => 15,
        }
        .min(config.max_ttl_minutes)
        .max(1);
        let grant = self.owned.computer_login_grants.insert(
            scope,
            prompt.id().into(),
            u64::from(minutes) * 60,
        );
        Ok(ComputerSecretApproval {
            sudo_turn: None,
            grant: Some(grant),
            _unlock: Some(unlock),
        })
    }
}
fn login_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "computer_secret_input.approval",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_grant_expiry_and_catalog_correlation_are_exact() {
        let grants = ComputerLoginGrants::default();
        let scope = LoginScope {
            owner: "owner".into(),
            session: "session".into(),
            agent: "agent".into(),
            credential: "handle".into(),
            app: "app-start".into(),
        };
        let expired = grants.insert(scope.clone(), "first".into(), 0);
        assert!(!grants.live(&expired, "first"));
        let grant = grants.insert(scope.clone(), "first".into(), 60);
        grants.extend_catalog("session", "agent", "unrelated", "other");
        assert!(!grants.live(&grant, "other"));
        grants.extend_catalog("session", "agent", "first", "continued");
        assert!(grants.live(&grant, "continued"));
        assert!(grants.find(&scope, "unrelated").is_none());
        let other = LoginScope {
            app: "restarted-app".into(),
            ..scope
        };
        assert!(grants.find(&other, "first").is_none());
        grants.revoke_all();
        assert!(!grants.live(&grant, "continued"));
    }
}
