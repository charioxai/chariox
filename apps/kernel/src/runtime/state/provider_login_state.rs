use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;

use zeroize::Zeroizing;

use super::claude_setup_token_capture::{ClaudeSetupTokenScreen, SetupTokenScan};
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

/// Where the Chariox Vault prompt of a `claude setup-token` login points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum ClaudeSetupTokenVaultPrompt {
    Unlock,
    Create,
    /// A new vault passphrase was entered once and must be repeated.
    ConfirmCreate,
}

/// Secret-bearing state of one `claude setup-token` login. It is shared by
/// record clones so reading a record never copies the screen or the token.
#[derive(Default)]
struct ClaudeSetupTokenSecrets {
    screen: Option<ClaudeSetupTokenScreen>,
    screen_text: String,
    notes: String,
    token: Option<Zeroizing<String>>,
    new_vault_passphrase: Option<Zeroizing<String>>,
    hidden_inputs: Vec<Zeroizing<String>>,
    reader_finished: bool,
    submission_requested: bool,
    failure_notice: Option<&'static str>,
}

impl ClaudeSetupTokenSecrets {
    fn projection(&self) -> Vec<u8> {
        format!("{}{}", self.screen_text, self.notes).into_bytes()
    }
}

#[derive(Clone)]
pub(in crate::runtime) struct ClaudeSetupTokenLogin {
    secrets: Arc<Mutex<ClaudeSetupTokenSecrets>>,
    /// Serializes PTY draining, rendering, input, cancel, and completion so
    /// concurrent status polls cannot interleave output or race the store.
    serial: Arc<tokio::sync::Mutex<()>>,
    pub vault_prompt: Option<ClaudeSetupTokenVaultPrompt>,
    pub overwrite: bool,
}

impl Default for ClaudeSetupTokenLogin {
    fn default() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(ClaudeSetupTokenSecrets {
                screen: Some(ClaudeSetupTokenScreen::default()),
                ..ClaudeSetupTokenSecrets::default()
            })),
            serial: Arc::new(tokio::sync::Mutex::new(())),
            vault_prompt: None,
            overwrite: true,
        }
    }
}

impl ClaudeSetupTokenLogin {
    pub async fn serialize(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.serial).lock_owned().await
    }

    fn secrets(&self) -> std::sync::MutexGuard<'_, ClaudeSetupTokenSecrets> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn process(&self, bytes: &[u8]) -> Vec<u8> {
        let mut secrets = self.secrets();
        let ClaudeSetupTokenSecrets {
            screen,
            screen_text,
            hidden_inputs,
            ..
        } = &mut *secrets;
        if let Some(screen) = screen.as_mut() {
            screen.process(bytes);
            *screen_text = screen.redacted_text_with_inputs(hidden_inputs);
        }
        secrets.projection()
    }

    /// The shared vt100 screen receives bytes before PTY queues/diagnostics.
    /// Status reads project that screen; raw bytes never leave the reader.
    pub fn read_private_output(&self, bytes: &[u8]) -> Vec<u8> {
        if bytes.is_empty() {
            self.secrets().reader_finished = true;
        } else {
            self.process(bytes);
        }
        Vec::new()
    }

    pub fn failure_notice(&self) -> Option<&'static str> {
        self.secrets().failure_notice
    }

    pub fn set_failure_notice(&self, notice: &'static str) {
        self.secrets().failure_notice = Some(notice);
    }

    fn authorization_retry_required(&self) -> bool {
        self.secrets()
            .screen
            .as_ref()
            .is_some_and(ClaudeSetupTokenScreen::authorization_retry_required)
    }

    fn code_sent(&self) -> bool {
        self.secrets().submission_requested
    }

    fn authorization_checking(&self) -> bool {
        let secrets = self.secrets();
        secrets.submission_requested
            && secrets.screen.as_ref().is_some_and(|screen| {
                screen.authorization_checking() && !screen.authorization_retry_required()
            })
    }

    pub fn reader_finished(&self) -> bool {
        self.secrets().reader_finished
    }

    pub fn hide_input(&self, input: &[u8]) -> Result<(), DaemonError> {
        let input = std::str::from_utf8(input)
            .map_err(|_| login_error("Claude authorization response must be UTF-8"))?
            .trim_end_matches(['\r', '\n']);
        if input.is_empty() && self.authorization_retry_required() {
            self.secrets().submission_requested = false;
        }
        if !input.is_empty() {
            let mut secrets = self.secrets();
            let retained_bytes: usize = secrets.hidden_inputs.iter().map(|value| value.len()).sum();
            if retained_bytes + input.len() > MAX_PROVIDER_LOGIN_OUTPUT_BYTES {
                return Err(login_error(
                    "Claude authorization responses exceed the bounded login limit",
                ));
            }
            secrets.submission_requested = true;
            secrets
                .hidden_inputs
                .push(Zeroizing::new(input.to_string()));
        }
        Ok(())
    }

    fn note(&self, note: &str) -> Vec<u8> {
        let mut secrets = self.secrets();
        secrets.notes.push('\n');
        secrets.notes.push_str(note);
        secrets.projection()
    }

    fn capture(&self, exited: bool) -> SetupTokenScan {
        let mut secrets = self.secrets();
        let scan = secrets
            .screen
            .as_ref()
            .map_or(SetupTokenScan::Pending, |screen| screen.scan(exited));
        if let SetupTokenScan::Found(token) = &scan {
            secrets.token = Some(token.clone());
        }
        scan
    }

    fn token(&self) -> Option<Zeroizing<String>> {
        self.secrets().token.clone()
    }

    /// Drops the rendered screen, which still holds the token, together with
    /// every captured secret. The redacted projection is kept.
    fn discard_secrets(&self) {
        let mut secrets = self.secrets();
        secrets.screen = None;
        secrets.token = None;
        secrets.new_vault_passphrase = None;
        secrets.hidden_inputs.clear();
    }
}

#[derive(Clone)]
pub(in crate::runtime) struct ProviderLoginProcessRecord {
    pub owner_user_id: String,
    pub kernel_id: String,
    pub provider: String,
    pub account_profile: String,
    pub credential_scope: String,
    pub login_id: String,
    pub start: ProviderLoginStart,
    pub state: ProviderLoginProcessState,
    pub backend: ProviderLoginProcessBackend,
    pub operation: ProviderAuthProcessOperation,
    pub setup_token: Option<ClaudeSetupTokenLogin>,
    pub output: Vec<u8>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}

impl ProviderLoginProcessRecord {
    pub fn awaits_vault_passphrase(&self) -> bool {
        self.state == ProviderLoginProcessState::Running
            && self
                .setup_token
                .as_ref()
                .is_some_and(|login| login.vault_prompt.is_some())
    }

    fn append_projected_output(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
        if self.output.len() > MAX_PROVIDER_LOGIN_OUTPUT_BYTES {
            let overflow = self.output.len() - MAX_PROVIDER_LOGIN_OUTPUT_BYTES;
            self.output.drain(..overflow);
        }
    }

    fn set_state(&mut self, state: ProviderLoginProcessState, now_ms: u64) {
        self.state = state;
        self.updated_at_ms = now_ms;
        if state != ProviderLoginProcessState::Running {
            if let Some(login) = self.setup_token.as_mut() {
                login.discard_secrets();
                login.vault_prompt = None;
            }
        }
    }

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
        if let Some(prompt) = self
            .setup_token
            .as_ref()
            .and_then(|login| login.vault_prompt)
        {
            return Some(self.vault_passphrase_interaction(prompt));
        }
        let retry_required = self
            .setup_token
            .as_ref()
            .is_some_and(ClaudeSetupTokenLogin::authorization_retry_required);
        let code_sent = self
            .setup_token
            .as_ref()
            .is_some_and(ClaudeSetupTokenLogin::code_sent);
        let checking = self
            .setup_token
            .as_ref()
            .is_some_and(ClaudeSetupTokenLogin::authorization_checking);
        let awaiting_submission = code_sent && !retry_required && !checking;
        let title = if awaiting_submission {
            "Code not submitted yet — press Enter"
        } else if checking {
            "Checking Claude authorization"
        } else if retry_required {
            "Retry Claude authorization"
        } else {
            match self.operation {
                ProviderAuthProcessOperation::Login => "Authenticate provider account",
                ProviderAuthProcessOperation::Logout => "Log out provider account",
            }
        };
        let authorization_url = self.setup_token.as_ref().and_then(|login| {
            login
                .secrets()
                .screen
                .as_ref()
                .and_then(ClaudeSetupTokenScreen::authorization_url)
        });
        let message = if awaiting_submission {
            "Code not submitted yet — press Enter. Choose Submit code to retry Enter without pasting the code again."
        } else if checking {
            "Checking the code…"
        } else if retry_required {
            "Claude could not accept the authorization response. Nothing was stored. Choose Retry authorization to open a fresh link, then authorize and paste the new code."
        } else if self.setup_token.is_some() {
            "Open the Claude authorization link to sign in. If Claude gives you a code, paste it below."
        } else if self.backend == ProviderLoginProcessBackend::Terminal {
            "Complete the provider-native terminal workflow. Its output is projected separately and responses are treated as secrets."
        } else {
            "Complete the provider-native browser authorization flow."
        };
        let mut interaction = crate::session::RuntimeInteraction::new(
            &self.login_id,
            format!(
                "provider-account:{}:{}",
                self.provider, self.account_profile
            ),
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Warning,
            Some(title.to_string()),
            message,
            if awaiting_submission {
                vec![
                    crate::session::RuntimeInteractionChoice::new(
                        "submit",
                        "Submit code (Enter)",
                        "submit",
                        Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                    ),
                    crate::session::RuntimeInteractionChoice::new(
                        "cancel",
                        "Cancel",
                        "cancel",
                        Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
                    ),
                ]
            } else if retry_required {
                vec![
                    crate::session::RuntimeInteractionChoice::new(
                        "retry",
                        "Retry authorization",
                        "retry",
                        Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                    ),
                    crate::session::RuntimeInteractionChoice::new(
                        "cancel",
                        "Cancel",
                        "cancel",
                        Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
                    ),
                ]
            } else {
                vec![crate::session::RuntimeInteractionChoice::new(
                    "cancel",
                    "Cancel",
                    "cancel",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
                )]
            },
            (self.backend == ProviderLoginProcessBackend::Terminal
                && !retry_required
                && !code_sent)
                .then(|| {
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
        );
        if self.setup_token.is_some() {
            let mut start = self.start.clone();
            start.auth_url = if retry_required {
                None
            } else {
                authorization_url
            };
            interaction = interaction.with_provider_login(crate::session::RuntimeProviderLogin {
                kernel_id: self.kernel_id.clone(),
                login: start,
                terminal_output_base64: String::new(),
            });
        }
        Some(interaction)
    }

    fn vault_passphrase_interaction(
        &self,
        prompt: ClaudeSetupTokenVaultPrompt,
    ) -> crate::session::RuntimeInteraction {
        let (title, message) = match prompt {
            ClaudeSetupTokenVaultPrompt::Unlock => (
                "Unlock Chariox Vault",
                "Enter the Chariox Vault passphrase to store the Claude setup token.",
            ),
            ClaudeSetupTokenVaultPrompt::Create => (
                "Create Chariox Vault",
                "Choose a Chariox Vault passphrase. The vault is created with it and stores the Claude setup token.",
            ),
            ClaudeSetupTokenVaultPrompt::ConfirmCreate => (
                "Confirm Chariox Vault passphrase",
                "Enter the new Chariox Vault passphrase again.",
            ),
        };
        let retry_note = self.setup_token.as_ref().and_then(|login| {
            login
                .secrets()
                .notes
                .lines()
                .last()
                .filter(|note| note.starts_with("Chariox Vault unlock failed:"))
                .map(str::to_string)
        });
        let message = retry_note
            .map(|note| format!("{message}\n{note}"))
            .unwrap_or_else(|| message.to_string());
        let label = if prompt == ClaudeSetupTokenVaultPrompt::ConfirmCreate {
            "Confirm vault passphrase"
        } else {
            "Vault passphrase"
        };
        crate::session::RuntimeInteraction::new(
            &self.login_id,
            format!(
                "provider-account:{}:{}",
                self.provider, self.account_profile
            ),
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Critical,
            Some(title.to_string()),
            message,
            vec![crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            )],
            Some(crate::session::RuntimeInteractionCustomChoice::secret(
                "passphrase",
                label,
                Some("Passphrase".to_string()),
                Some(1),
                Some(512),
            )),
            Some(10 * 60),
            None,
        )
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
                existing.set_state(ProviderLoginProcessState::Failed, now_ms);
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
        if let Some(login) = record.setup_token.clone() {
            let mut projection = None;
            for chunk in chunks {
                projection = Some(login.process(&chunk));
            }
            let projection = projection.unwrap_or_else(|| login.secrets().projection());
            record.output.clear();
            record.append_projected_output(&projection);
        } else {
            for chunk in chunks {
                record.append_projected_output(&chunk);
            }
        }
        record.updated_at_ms = now_ms;
        Ok(record.status())
    }

    /// Adds a kernel message below the rendered `claude setup-token` screen.
    pub fn append_setup_token_note(
        &self,
        owner_user_id: &str,
        login_id: &str,
        note: &str,
        now_ms: u64,
    ) -> Result<ProviderLoginStatus, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        let projection = setup_token_login(record)?.note(note);
        record.output.clear();
        record.append_projected_output(&projection);
        record.updated_at_ms = now_ms;
        Ok(record.status())
    }

    pub fn capture_setup_token(
        &self,
        owner_user_id: &str,
        login_id: &str,
        exited: bool,
    ) -> Result<SetupTokenScan, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        Ok(setup_token_login(record)?.capture(exited))
    }

    pub fn setup_token_secret(
        &self,
        owner_user_id: &str,
        login_id: &str,
    ) -> Result<Option<Zeroizing<String>>, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        Ok(setup_token_login(record)?.token())
    }

    pub fn await_vault_passphrase(
        &self,
        owner_user_id: &str,
        login_id: &str,
        prompt: ClaudeSetupTokenVaultPrompt,
        note: &str,
        now_ms: u64,
    ) -> Result<ProviderLoginStatus, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        let login = setup_token_login(record)?;
        let projection = login.note(note);
        record
            .setup_token
            .as_mut()
            .expect("setup-token login was checked")
            .vault_prompt = Some(prompt);
        record.output.clear();
        record.append_projected_output(&projection);
        record.updated_at_ms = now_ms;
        Ok(record.status())
    }

    /// Keeps the first entry of a new vault passphrase until it is confirmed.
    pub fn exchange_new_vault_passphrase(
        &self,
        owner_user_id: &str,
        login_id: &str,
        entered: Option<Zeroizing<String>>,
    ) -> Result<Option<Zeroizing<String>>, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        let login = setup_token_login(record)?;
        let mut secrets = login.secrets();
        Ok(std::mem::replace(
            &mut secrets.new_vault_passphrase,
            entered,
        ))
    }

    /// Ends a workflow only if it is still running, so a cancel or timeout
    /// that won the race is never overwritten by a late completion.
    pub fn finish_if_running(
        &self,
        owner_user_id: &str,
        login_id: &str,
        state: ProviderLoginProcessState,
        now_ms: u64,
    ) -> Result<Option<ProviderLoginStatus>, DaemonError> {
        let mut records = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let record = owned_record_mut(&mut records, owner_user_id, login_id)?;
        if record.state != ProviderLoginProcessState::Running {
            return Ok(None);
        }
        record.set_state(state, now_ms);
        Ok(Some(record.status()))
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
        if record.state != ProviderLoginProcessState::Running && record.state != state {
            return Ok(record.status());
        }
        record.set_state(state, now_ms);
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

fn setup_token_login(
    record: &ProviderLoginProcessRecord,
) -> Result<ClaudeSetupTokenLogin, DaemonError> {
    record
        .setup_token
        .clone()
        .ok_or_else(|| login_error("provider login is not a Claude setup-token login"))
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
            kernel_id: "test-kernel".into(),
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
            setup_token: None,
            output: Vec::new(),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    // MP-08/MP-10/MP-11: OAuth links belong to the kernel interaction.
    #[test]
    fn setup_token_projects_oauth_link_and_secret_code_without_command_instructions() {
        let store = ProviderLoginProcessStore::default();
        let mut login = record("owner-a", "login-a");
        login.setup_token = Some(ClaudeSetupTokenLogin::default());
        login.start.login_kind = "terminal_setup_token".into();
        store.insert(login).unwrap();
        let url = "https://claude.com/cai/oauth/authorize?code=true&client_id=fixture";
        let status = store
            .append_output(
                "owner-a",
                "login-a",
                [format!("{url}\r\nPaste code here if prompted > ").into_bytes()],
                2,
            )
            .unwrap();
        let interaction = status.interaction.unwrap();
        assert_eq!(
            interaction
                .provider_login()
                .unwrap()
                .login
                .auth_url
                .as_deref(),
            Some(url)
        );
        assert!(interaction.message().contains("authorization link"));
        assert!(!interaction.message().contains("terminal workflow"));
        assert_eq!(
            interaction.custom_choice().unwrap().input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );
    }

    #[test]
    fn setup_token_echo_does_not_confirm_submission_and_enter_retry_is_secret_free() {
        let mut record = record("owner-a", "login-a");
        let login = ClaudeSetupTokenLogin::default();
        record.setup_token = Some(login.clone());
        login.read_private_output(b"Paste code here if prompted > ");
        login.hide_input(b"FAKECODE#FAKESTATE\r").unwrap();
        login.read_private_output(b"************");
        let interaction = record.status().interaction.unwrap();
        assert_eq!(
            interaction.title(),
            Some("Code not submitted yet — press Enter")
        );
        assert_eq!(interaction.choices()[0].id(), "submit");
        assert!(interaction.custom_choice().is_none());
        assert!(!interaction.message().contains("FAKECODE"));
        login.read_private_output(b"\x1b[2J\x1b[H");
        assert_eq!(
            record.status().interaction.unwrap().title(),
            Some("Code not submitted yet — press Enter")
        );
        login.read_private_output(b"Exchanging code for token...\r\n");
        assert_eq!(
            record.status().interaction.unwrap().title(),
            Some("Checking Claude authorization")
        );
        login.read_private_output(
            b"\x1b[2J\x1b[HOAuth error: Invalid code\r\nPress Enter to retry.\r\n",
        );
        assert_eq!(
            record.status().interaction.unwrap().title(),
            Some("Retry Claude authorization")
        );
        login.hide_input(b"\r").unwrap();
        login.read_private_output(b"\x1b[2J\x1b[HPaste code here if prompted > ");
        assert_eq!(
            record.status().interaction.unwrap().title(),
            Some("Authenticate provider account")
        );
        assert!(record
            .status()
            .interaction
            .unwrap()
            .custom_choice()
            .is_some());
    }

    // MP-08/MP-10/MP-11: a running official OAuth refusal needs a safe retry phase.
    #[test]
    fn setup_token_rejected_oauth_code_projects_retry_without_secrets() {
        let store = ProviderLoginProcessStore::default();
        let mut login = record("owner-a", "login-a");
        login.setup_token = Some(ClaudeSetupTokenLogin::default());
        login.start.login_kind = "terminal_setup_token".into();
        store.insert(login).unwrap();
        let status = store.append_output("owner-a", "login-a", [
            b"https://claude.com/cai/oauth/authorize?code=true&client_id=fixture\r\nPaste code here if prompted > ".to_vec(),
            b"\r\nOAuth error: Request failed with status code 400 secret-provider-detail\r\nPress Enter to retry.\r\n".to_vec(),
        ], 2).unwrap();
        assert_eq!(status.state, ProviderLoginProcessState::Running);
        let interaction = status.interaction.unwrap();
        assert_eq!(
            interaction.title(),
            Some("Retry Claude authorization"),
            "MP-08/MP-10/MP-11 must surface the running OAuth refusal"
        );
        assert!(interaction
            .message()
            .contains("Claude could not accept the authorization response"));
        assert!(!interaction.message().contains("secret-provider-detail"));
        assert!(interaction
            .choices()
            .iter()
            .any(|choice| choice.id() == "retry"));
        assert!(interaction.custom_choice().is_none());
        assert!(interaction
            .provider_login()
            .unwrap()
            .login
            .auth_url
            .is_none());
        assert!(interaction
            .provider_login()
            .unwrap()
            .terminal_output_base64
            .is_empty());
        let status = store.append_output("owner-a", "login-a", [b"\x1b[2J\x1b[Hhttps://claude.com/cai/oauth/authorize?code=true&client_id=next\r\nPaste code here if prompted > ".to_vec()], 3).unwrap();
        let interaction = status.interaction.unwrap();
        assert_eq!(interaction.title(), Some("Authenticate provider account"));
        assert!(interaction.custom_choice().is_some());
        assert!(interaction
            .provider_login()
            .unwrap()
            .login
            .auth_url
            .as_deref()
            .unwrap()
            .contains("client_id=next"));
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
    fn setup_token_never_reaches_the_projected_login_status() {
        const TOKEN: &str = "sk-ant-oat01-TESTONLYzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-9";
        let store = ProviderLoginProcessStore::default();
        let mut login = record("owner-a", "login-a");
        login.setup_token = Some(ClaudeSetupTokenLogin::default());
        store.insert(login).unwrap();

        let (head, tail) = TOKEN.split_at(20);
        let mut statuses = vec![store
            .append_output(
                "owner-a",
                "login-a",
                [
                    b"\x1b[2GYour\x1b[7GOAuth\x1b[13Gtoken:\r\r\n\x1b[1m".to_vec(),
                    head.as_bytes().to_vec(),
                ],
                2,
            )
            .unwrap()];
        assert_eq!(
            store
                .capture_setup_token("owner-a", "login-a", false)
                .unwrap(),
            SetupTokenScan::Pending
        );
        statuses.push(
            store
                .append_output(
                    "owner-a",
                    "login-a",
                    [format!("\x1b[33m{tail}\x1b[22m\r\r\n\x1b[2GStore\x1b[8Gthis").into_bytes()],
                    3,
                )
                .unwrap(),
        );
        assert!(matches!(
            store
                .capture_setup_token("owner-a", "login-a", false)
                .unwrap(),
            SetupTokenScan::Found(_)
        ));
        statuses.push(
            store
                .await_vault_passphrase(
                    "owner-a",
                    "login-a",
                    ClaudeSetupTokenVaultPrompt::Unlock,
                    "Enter your Chariox Vault passphrase.",
                    5,
                )
                .unwrap(),
        );

        for status in &statuses {
            let output = base64::engine::general_purpose::STANDARD
                .decode(&status.terminal_output_base64)
                .unwrap();
            let output = String::from_utf8_lossy(&output);
            assert!(!output.contains("sk-ant"), "{output}");
            assert!(!output.contains("TESTONLY"), "{output}");
            assert!(!output.contains('\x1b'), "{output}");
            let wire =
                serde_json::to_string(&crate::local::LocalDaemonResponse::ProviderLoginStatus {
                    login: status.clone(),
                })
                .unwrap();
            assert!(!wire.contains("TESTONLY"));
            assert!(!wire.contains(&base64::engine::general_purpose::STANDARD.encode(TOKEN)));
        }
        let output = base64::engine::general_purpose::STANDARD
            .decode(&statuses[2].terminal_output_base64)
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            " Your OAuth token:\n[setup token captured; hidden]\n Store this\nEnter your Chariox Vault passphrase."
        );
        assert_eq!(
            store
                .setup_token_secret("owner-a", "login-a")
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some(TOKEN)
        );

        let interaction = statuses[2].interaction.as_ref().expect("vault prompt");
        assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
        assert_eq!(
            interaction
                .custom_choice()
                .expect("passphrase")
                .input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );
        assert!(store
            .record_for_owner("owner-a", "login-a")
            .unwrap()
            .awaits_vault_passphrase());

        store
            .set_state(
                "owner-a",
                "login-a",
                ProviderLoginProcessState::Cancelled,
                6,
            )
            .unwrap();
        assert!(store
            .setup_token_secret("owner-a", "login-a")
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .capture_setup_token("owner-a", "login-a", true)
                .unwrap(),
            SetupTokenScan::Pending,
            "a cancelled login keeps no screen that still holds the token"
        );
        assert_eq!(
            store
                .finish_if_running(
                    "owner-a",
                    "login-a",
                    ProviderLoginProcessState::Succeeded,
                    7
                )
                .unwrap(),
            None,
            "a late completion cannot overwrite the cancel"
        );
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
