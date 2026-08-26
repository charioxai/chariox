use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;

use crate::error::DaemonError;
use crate::local::{ProviderLoginProcessState, ProviderLoginStatus};
use crate::provider::ProviderLoginStart;

const MAX_PROVIDER_LOGIN_OUTPUT_BYTES: usize = 64 * 1024;
pub(in crate::runtime) const PROVIDER_LOGIN_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum ProviderLoginProcessBackend {
    CodexAppServer,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum ProviderAuthProcessOperation {
    Login,
    Logout,
}

#[derive(Clone)]
pub(in crate::runtime) struct ProviderLoginProcessRecord {
    pub owner_user_id: String,
    pub provider: String,
    pub account_profile: String,
    pub credential_scope: String,
    pub login_id: String,
    pub start: ProviderLoginStart,
    pub state: ProviderLoginProcessState,
    pub backend: ProviderLoginProcessBackend,
    pub operation: ProviderAuthProcessOperation,
    pub output: Vec<u8>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}

impl ProviderLoginProcessRecord {
    pub fn status(&self) -> ProviderLoginStatus {
        ProviderLoginStatus {
            provider: self.provider.clone(),
            account_profile: self.account_profile.clone(),
            login_id: self.login_id.clone(),
            state: self.state,
            interaction: self.interaction(),
            terminal_output_base64: base64::engine::general_purpose::STANDARD.encode(&self.output),
            started_at_ms: self.started_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }

    fn interaction(&self) -> Option<crate::session::RuntimeInteraction> {
        if self.state != ProviderLoginProcessState::Running {
            return None;
        }
        let title = match self.operation {
            ProviderAuthProcessOperation::Login => "Authenticate provider account",
            ProviderAuthProcessOperation::Logout => "Log out provider account",
        };
        let message = if self.backend == ProviderLoginProcessBackend::Terminal {
            "Complete the provider-native terminal workflow. Its output is projected separately and responses are treated as secrets."
        } else {
            "Complete the provider-native browser authorization flow."
        };
        Some(crate::session::RuntimeInteraction::new(
            &self.login_id,
            format!(
                "provider-account:{}:{}",
                self.provider, self.account_profile
            ),
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Warning,
            Some(title.to_string()),
            message,
            vec![crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            )],
            (self.backend == ProviderLoginProcessBackend::Terminal).then(|| {
                crate::session::RuntimeInteractionCustomChoice::secret(
                    "provider-response",
                    "Send response",
                    Some("Enter the response requested by the provider CLI".to_string()),
                    Some(1),
                    Some(8 * 1024),
                )
            }),
            Some(10 * 60),
            None,
        ))
    }
}

#[derive(Clone, Default)]
pub(in crate::runtime) struct ProviderLoginProcessStore {
    inner: Arc<Mutex<BTreeMap<String, ProviderLoginProcessRecord>>>,
}

impl std::fmt::Debug for ProviderLoginProcessStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let process_count = self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        formatter
            .debug_struct("ProviderLoginProcessStore")
            .field("process_count", &process_count)
            .finish()
    }
}

impl ProviderLoginProcessStore {
    pub fn insert(&self, record: ProviderLoginProcessRecord) -> Result<(), DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let now_ms = crate::session::unix_epoch_ms();
        for existing in records.values_mut() {
            if existing.state == ProviderLoginProcessState::Running
                && now_ms.saturating_sub(existing.started_at_ms) >= PROVIDER_LOGIN_TIMEOUT_MS
            {
                existing.state = ProviderLoginProcessState::Failed;
                existing.updated_at_ms = now_ms;
            }
        }
        if records.values().any(|existing| {
            existing.provider == record.provider
                && existing.state == ProviderLoginProcessState::Running
                && if record.provider == "claude" {
                    existing.credential_scope == record.credential_scope
                } else {
                    existing.owner_user_id == record.owner_user_id
                        && existing.account_profile == record.account_profile
                }
        }) {
            return Err(login_error(if record.provider == "claude" {
                "Claude authentication is temporarily busy; retry after the running login finishes or expires"
            } else {
                "a provider login is already running for this account profile"
            }));
        }
        records.insert(record.login_id.clone(), record);
        Ok(())
    }

    pub fn record_for_owner(
        &self,
        owner_user_id: &str,
        login_id: &str,
    ) -> Result<ProviderLoginProcessRecord, DaemonError> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(login_id)
            .filter(|record| record.owner_user_id == owner_user_id)
            .cloned()
            .ok_or_else(|| login_error("provider login was not found"))
    }

    pub fn has_running_for_profile(
        &self,
        owner_user_id: &str,
        provider: &str,
        account_profile: &str,
    ) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .any(|record| {
                record.owner_user_id == owner_user_id
                    && record.provider == provider
                    && record.account_profile == account_profile
                    && record.state == ProviderLoginProcessState::Running
            })
    }

    pub fn running_start_for_profile(
        &self,
        owner_user_id: &str,
        provider: &str,
        account_profile: &str,
    ) -> Option<ProviderLoginStart> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|record| {
                record.owner_user_id == owner_user_id
                    && record.provider == provider
                    && record.account_profile == account_profile
                    && record.state == ProviderLoginProcessState::Running
            })
            .map(|record| record.start.clone())
    }

    pub fn running_for_owner_provider(
        &self,
        owner_user_id: &str,
        provider: &str,
    ) -> Vec<ProviderLoginProcessRecord> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|record| {
                record.owner_user_id == owner_user_id
                    && record.provider == provider
                    && record.state == ProviderLoginProcessState::Running
            })
            .cloned()
            .collect()
    }

    pub fn remove(&self, login_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(login_id);
    }

    pub fn append_output(
        &self,
        owner_user_id: &str,
        login_id: &str,
        chunks: impl IntoIterator<Item = Vec<u8>>,
        now_ms: u64,
    ) -> Result<ProviderLoginStatus, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        for chunk in chunks {
            record.output.extend_from_slice(&chunk);
        }
        if record.output.len() > MAX_PROVIDER_LOGIN_OUTPUT_BYTES {
            let overflow = record.output.len() - MAX_PROVIDER_LOGIN_OUTPUT_BYTES;
            record.output.drain(..overflow);
        }
        record.updated_at_ms = now_ms;
        Ok(record.status())
    }

    pub fn set_state(
        &self,
        owner_user_id: &str,
        login_id: &str,
        state: ProviderLoginProcessState,
        now_ms: u64,
    ) -> Result<ProviderLoginStatus, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        record.state = state;
        record.updated_at_ms = now_ms;
        Ok(record.status())
    }
}

fn owned_record_mut<'a>(
    records: &'a mut BTreeMap<String, ProviderLoginProcessRecord>,
    owner_user_id: &str,
    login_id: &str,
) -> Result<&'a mut ProviderLoginProcessRecord, DaemonError> {
    records
        .get_mut(login_id)
        .filter(|record| record.owner_user_id == owner_user_id)
        .ok_or_else(|| login_error("provider login was not found"))
}

fn login_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider login",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(owner: &str, login_id: &str) -> ProviderLoginProcessRecord {
        let now_ms = crate::session::unix_epoch_ms();
        ProviderLoginProcessRecord {
            owner_user_id: owner.to_string(),
            provider: "claude".to_string(),
            account_profile: "work".to_string(),
            credential_scope: "claude-ambient".to_string(),
            login_id: login_id.to_string(),
            start: crate::provider::ProviderLoginStart {
                provider: "claude".to_string(),
                account_profile: "work".to_string(),
                login_kind: "terminal".to_string(),
                login_id: Some(login_id.to_string()),
                auth_url: None,
                verification_url: None,
                user_code: None,
            },
            state: ProviderLoginProcessState::Running,
            backend: ProviderLoginProcessBackend::Terminal,
            operation: ProviderAuthProcessOperation::Login,
            output: Vec::new(),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    #[test]
    fn login_process_queries_are_owner_scoped() {
        let store = ProviderLoginProcessStore::default();
        store.insert(record("owner-a", "login-a")).unwrap();
        assert!(store.has_running_for_profile("owner-a", "claude", "work"));
        assert_eq!(
            store
                .running_start_for_profile("owner-a", "claude", "work")
                .and_then(|start| start.login_id),
            Some("login-a".to_string())
        );
        assert!(!store.has_running_for_profile("owner-b", "claude", "work"));
        assert!(store.insert(record("owner-a", "login-b")).is_err());
        assert!(store.record_for_owner("owner-b", "login-a").is_err());
        let mut owner_b = record("owner-b", "login-b");
        owner_b.credential_scope = "claude-config:/profiles/owner-b".to_string();
        store.insert(owner_b).unwrap();
        store
            .set_state(
                "owner-a",
                "login-a",
                ProviderLoginProcessState::Succeeded,
                2,
            )
            .unwrap();
        assert!(!store.has_running_for_profile("owner-a", "claude", "work"));
    }

    #[test]
    fn ambient_claude_logins_are_single_flight_per_owner_across_profiles() {
        let store = ProviderLoginProcessStore::default();
        store.insert(record("owner-a", "login-work")).unwrap();

        let mut personal = record("owner-a", "login-personal");
        personal.account_profile = "personal".to_string();

        assert!(store.insert(personal).is_err());
    }

    #[test]
    fn claude_logins_with_distinct_explicit_scopes_can_run_in_parallel() {
        let store = ProviderLoginProcessStore::default();
        let mut work = record("owner-a", "login-work");
        work.credential_scope = "claude-config:/profiles/work".to_string();
        store.insert(work).unwrap();

        let mut personal = record("owner-a", "login-personal");
        personal.account_profile = "personal".to_string();
        personal.credential_scope = "claude-config:/profiles/personal".to_string();

        store.insert(personal).unwrap();
    }

    #[test]
    fn claude_logins_sharing_an_explicit_scope_are_single_flight() {
        let store = ProviderLoginProcessStore::default();
        let mut work = record("owner-a", "login-work");
        work.credential_scope = "claude-config:/profiles/shared".to_string();
        store.insert(work).unwrap();

        let mut alias = record("owner-a", "login-alias");
        alias.account_profile = "work-alias".to_string();
        alias.credential_scope = "claude-config:/profiles/shared".to_string();

        assert!(store.insert(alias).is_err());
    }

    #[test]
    fn claude_logins_sharing_a_scope_are_single_flight_across_owners() {
        let store = ProviderLoginProcessStore::default();
        let mut owner_a = record("owner-a", "login-owner-a");
        owner_a.credential_scope = "claude-config:/profiles/shared".to_string();
        store.insert(owner_a).unwrap();

        let mut owner_b = record("owner-b", "login-owner-b");
        owner_b.credential_scope = "claude-config:/profiles/shared".to_string();

        assert!(store.insert(owner_b).is_err());
    }

    #[test]
    fn claude_logins_with_distinct_scopes_remain_independent_across_owners() {
        let store = ProviderLoginProcessStore::default();
        let mut owner_a = record("owner-a", "login-owner-a");
        owner_a.credential_scope = "claude-config:/profiles/owner-a".to_string();
        store.insert(owner_a).unwrap();

        let mut owner_b = record("owner-b", "login-owner-b");
        owner_b.credential_scope = "claude-config:/profiles/owner-b".to_string();

        store.insert(owner_b).unwrap();
    }

    #[test]
    fn expired_claude_login_does_not_block_another_profile() {
        let store = ProviderLoginProcessStore::default();
        let mut expired = record("owner-a", "login-work");
        expired.started_at_ms = expired
            .started_at_ms
            .saturating_sub(PROVIDER_LOGIN_TIMEOUT_MS);
        store.insert(expired).unwrap();

        let mut personal = record("owner-a", "login-personal");
        personal.account_profile = "personal".to_string();
        store.insert(personal).unwrap();

        assert_eq!(
            store
                .record_for_owner("owner-a", "login-work")
                .unwrap()
                .state,
            ProviderLoginProcessState::Failed
        );
    }

    #[test]
    fn opencode_logins_remain_independent_across_profiles() {
        let store = ProviderLoginProcessStore::default();
        let mut work = record("owner-a", "login-work");
        work.provider = "opencode".to_string();
        store.insert(work).unwrap();

        let mut personal = record("owner-a", "login-personal");
        personal.provider = "opencode".to_string();
        personal.account_profile = "personal".to_string();

        store.insert(personal).unwrap();
    }

    #[test]
    fn terminal_output_is_bounded_and_store_debug_is_redacted() {
        let store = ProviderLoginProcessStore::default();
        store.insert(record("owner-a", "login-a")).unwrap();
        let secret = vec![b's'; MAX_PROVIDER_LOGIN_OUTPUT_BYTES + 32];
        let status = store
            .append_output("owner-a", "login-a", [secret], 2)
            .unwrap();
        let output = base64::engine::general_purpose::STANDARD
            .decode(&status.terminal_output_base64)
            .unwrap();
        assert_eq!(output.len(), MAX_PROVIDER_LOGIN_OUTPUT_BYTES);
        assert_eq!(
            format!("{store:?}"),
            "ProviderLoginProcessStore { process_count: 1 }"
        );
        assert!(!format!("{status:?}").contains("c3Nz"));
    }

    #[test]
    fn running_terminal_auth_projects_one_secret_runtime_interaction() {
        let store = ProviderLoginProcessStore::default();
        store.insert(record("owner-a", "login-a")).unwrap();

        let running = store
            .record_for_owner("owner-a", "login-a")
            .unwrap()
            .status();
        let interaction = running.interaction.expect("running workflow interaction");
        assert_eq!(interaction.id(), "login-a");
        assert_eq!(interaction.choices().len(), 1);
        assert_eq!(interaction.choices()[0].id(), "cancel");
        assert_eq!(
            interaction
                .custom_choice()
                .expect("terminal response")
                .input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );

        let completed = store
            .set_state(
                "owner-a",
                "login-a",
                ProviderLoginProcessState::Succeeded,
                2,
            )
            .unwrap();
        assert!(completed.interaction.is_none());
    }
}
