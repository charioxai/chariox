//! MP-08/MP-11: official Claude utility on the profile-owning ordinary kernel.
//! Raw output stays in a bounded, zeroizing capture at the PTY reader boundary.
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

use super::command::{command_caller_user_id, KernelCommand};
use super::projection::DaemonConfigProjectionStore;
use super::state::{
    KernelRuntimeState, ProviderAuthProcessOperation, ProviderLoginProcessBackend,
    ProviderLoginProcessRecord,
};
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, ProviderLoginProcessState, SetProviderAccountCredentialRequest,
};
use crate::pty::{PtyProcessState, PtySpawnRequest};

const CAPTURE_LIMIT: usize = 128 * 1024;
const TOKEN_PREFIX: &[u8] = b"sk-ant-oat01-";

#[derive(Default)]
struct Capture {
    text: Zeroizing<Vec<u8>>,
    line_start: usize,
    escape: u8,
    overflow: bool,
    finished: bool,
    prompted: bool,
}

impl Capture {
    fn read(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut projected = Vec::new();
        if bytes.is_empty() {
            self.finished = true;
        }
        for &byte in bytes {
            if self.text.len() >= CAPTURE_LIMIT {
                self.overflow = true;
                break;
            }
            // Strip CSI/OSC decoration before parsing, including chunk-split sequences.
            match self.escape {
                1 => {
                    self.escape = match byte {
                        b'[' => 2,
                        b']' => 3,
                        _ => 0,
                    };
                    continue;
                }
                2 => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = 0;
                    }
                    continue;
                }
                3 => {
                    if byte == 7 {
                        self.escape = 0;
                    } else if byte == 27 {
                        self.escape = 4;
                    }
                    continue;
                }
                4 => {
                    self.escape = if byte == b'\\' { 0 } else { 3 };
                    continue;
                }
                _ => {}
            }
            if byte == 27 {
                self.escape = 1;
                continue;
            }
            if byte == b'\r' {
                continue;
            }
            self.text.push(byte);
            if byte == b'\n' {
                if !self.prompted
                    && self.text[self.line_start..]
                        .windows(4)
                        .any(|word| word.eq_ignore_ascii_case(b"code"))
                {
                    self.prompted = true;
                    projected.extend_from_slice(b"If Claude requests an authorization code, send it using the hidden provider response.\n");
                }
                projected.extend(self.project_line(&self.text[self.line_start..]));
                self.line_start = self.text.len();
            }
        }
        // Native prompts can omit a newline. Emit only a fixed prompt, never partial bytes.
        if !self.prompted
            && self.text[self.line_start..]
                .windows(4)
                .any(|word| word.eq_ignore_ascii_case(b"code"))
        {
            self.prompted = true;
            projected.extend_from_slice(b"If Claude requests an authorization code, send it using the hidden provider response.\n");
        }
        projected
    }

    fn project_line(&self, line: &[u8]) -> Vec<u8> {
        let mut projected = Vec::new();
        if line
            .windows(TOKEN_PREFIX.len())
            .any(|part| part == TOKEN_PREFIX)
        {
            return projected;
        }
        // All other native text is suppressed: errors may echo credentials or hidden input.
        for word in line.split(|byte| byte.is_ascii_whitespace()) {
            if !word.starts_with(b"https://") {
                continue;
            }
            let Ok(word) = std::str::from_utf8(word) else {
                continue;
            };
            let Ok(url) = url::Url::parse(word) else {
                continue;
            };
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || url.fragment().is_some()
                || !matches!(
                    url.host_str(),
                    Some("claude.ai" | "console.anthropic.com" | "platform.claude.com")
                )
                || url.path() != "/oauth/authorize"
                || url.query_pairs().any(|(name, value)| {
                    !matches!(
                        name.as_ref(),
                        "client_id"
                            | "response_type"
                            | "redirect_uri"
                            | "scope"
                            | "code_challenge"
                            | "code_challenge_method"
                            | "state"
                            | "code"
                    ) || value.contains("sk-ant-")
                        || value.chars().any(char::is_control)
                        || (name == "code" && value != "true")
                })
            {
                continue;
            }
            projected.extend_from_slice(b"Authorize Claude in your browser: ");
            projected.extend_from_slice(url.as_str().as_bytes());
            projected.push(b'\n');
        }
        projected
    }

    fn token(&self) -> Result<Zeroizing<String>, DaemonError> {
        if !self.finished || self.overflow || self.escape != 0 {
            return Err(capture_error());
        }
        let candidates: Vec<_> = self
            .text
            .windows(TOKEN_PREFIX.len())
            .enumerate()
            .filter(|(_, word)| *word == TOKEN_PREFIX)
            .map(|(offset, _)| offset)
            .collect();
        if candidates.len() != 1 {
            return Err(capture_error());
        }
        let start = candidates[0];
        if start > 0 && token_char(self.text[start - 1]) {
            return Err(capture_error());
        }
        let end = self.text[start..]
            .iter()
            .position(|byte| !token_char(*byte))
            .map(|offset| start + offset)
            .unwrap_or(self.text.len());
        let body_len = end - start - TOKEN_PREFIX.len();
        if !(32..=1024).contains(&body_len) {
            return Err(capture_error());
        }
        // Reject punctuation/unknown format adjoining a candidate rather than truncate it.
        if end < self.text.len()
            && !self.text[end].is_ascii_whitespace()
            && !matches!(self.text[end], b'\'' | b'"' | b'`')
        {
            return Err(capture_error());
        }
        Ok(Zeroizing::new(
            std::str::from_utf8(&self.text[start..end])
                .map_err(|_| capture_error())?
                .to_string(),
        ))
    }
}

fn token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}
fn capture_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Claude setup token",
        message:
            "Claude setup token capture failed (missing, ambiguous, malformed or incomplete output)"
                .to_string(),
    }
}

pub(super) async fn start(
    config_projection: &DaemonConfigProjectionStore,
    runtime: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetProviderAccountCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.provider != "claude" || !request.value.is_empty() {
        return Err(capture_error());
    }
    let owner = runtime.provider_account_authority_owner_user_id(&command_caller_user_id(command));
    let registry = runtime.provider_account_profile_registry();
    let profile = registry.get(&owner, "claude", &request.account_profile)?;
    let credential_id =
        crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
    if !request.overwrite
        && crate::credential::CharioxCredentialRegistry::user()?
            .get(&credential_id)?
            .is_some()
    {
        return Err(DaemonError::LocalTransport {
            operation: "Claude setup token",
            message: "A setup token already exists for this profile; use --replace to authorize replacement".to_string(),
        });
    }
    let guard = runtime
        .ensure_vault_unlocked_for_command_context(
            command,
            request.session_id.as_deref(),
            request.agent_id.as_deref(),
            "provider_account_credential_set",
        )
        .await?;
    let environment = registry.resolve_environment(&owner, "claude", &profile.profile_id)?;
    let credential_scope = environment
        .get("CLAUDE_CONFIG_DIR")
        .map(|path| format!("claude-config:{path}"))
        .unwrap_or_else(|| "claude-ambient".to_string());
    let program = crate::provider::resolve_claude_executable()?;
    let launch = crate::provider::managed_isolated_utility_launch(
        program.to_string_lossy().to_string(),
        vec!["setup-token".to_string()],
        environment,
        None,
        "claude:setup-token",
    )?;
    let mut env_remove = launch.pty_env_remove;
    env_remove.extend(
        crate::account_profile::provider_auth_env_vars("claude")
            .iter()
            .map(|name| (*name).to_string()),
    );
    let login_id = format!("provider-login-{}", rand::random::<u128>());
    let workflow = crate::provider::ProviderLoginStart {
        provider: "claude".to_string(),
        account_profile: profile.profile_id.clone(),
        login_kind: "terminal".to_string(),
        login_id: Some(login_id.clone()),
        auth_url: None,
        verification_url: None,
        user_code: None,
    };
    let now = crate::session::unix_epoch_ms();
    let record = ProviderLoginProcessRecord {
        owner_user_id: owner.clone(),
        provider: "claude".to_string(),
        account_profile: profile.profile_id.clone(),
        credential_scope,
        login_id: login_id.clone(),
        start: workflow.clone(),
        state: ProviderLoginProcessState::Running,
        backend: ProviderLoginProcessBackend::Terminal,
        operation: ProviderAuthProcessOperation::SetupToken,
        setup_token: None,
        output:
            b"Complete Claude browser authorization; the setup token will be captured privately.\n"
                .to_vec(),
        started_at_ms: now,
        updated_at_ms: now,
    };
    runtime.provider_login_process_store().insert(record)?;
    let capture = Arc::new(Mutex::new(Capture::default()));
    let reader_capture = capture.clone();
    let spawned = runtime
        .with_app_side_effect(|app| {
            app.pty_mut().spawn_with_output_filter(
                PtySpawnRequest {
                    process_key: login_id.clone(),
                    provider_run_id: login_id.clone(),
                    program: launch.pty_program.ok_or_else(capture_error)?,
                    args: launch.pty_args,
                    env: launch.pty_env,
                    env_remove,
                    working_directory: launch.working_directory,
                    cols: 120,
                    rows: 40,
                },
                &crate::provider::ProviderCredentialEnvironment::default(),
                true,
                Some(Box::new(move |bytes| {
                    reader_capture.lock().unwrap().read(bytes)
                })),
            )
        })
        .await;
    if let Err(error) = spawned {
        runtime.provider_login_process_store().remove(&login_id);
        return Err(error);
    }
    let registry = registry.clone();
    let runtime = runtime.clone();
    let config = config_projection.snapshot();
    let overwrite = request.overwrite;
    tokio::spawn(async move {
        let _guard = guard;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let record = runtime
                .provider_login_process_store()
                .record_for_owner(&owner, &login_id);
            if !record.is_ok_and(|record| record.state == ProviderLoginProcessState::Running) {
                break;
            }
            let result = super::provider_auth_control::execute_get_provider_login_status_request(
                &runtime,
                &owner,
                crate::local::GetProviderLoginStatusRequest {
                    login_id: login_id.clone(),
                },
            )
            .await;
            if result.is_err() {
                break;
            }
            let state = runtime
                .with_app_side_effect(|app| app.pty_mut().poll_process_state(&login_id))
                .await;
            if let Ok(PtyProcessState::Exited { exit_code, signal }) = state {
                if !capture.lock().unwrap().finished {
                    continue;
                }
                let token = if exit_code == Some(0) && signal.is_none() {
                    capture.lock().unwrap().token()
                } else {
                    Err(capture_error())
                };
                let _ = runtime.provider_login_process_store().finish_setup_token(
                    &owner,
                    &login_id,
                    || {
                        registry.get(&owner, "claude", &profile.profile_id)?;
                        let token = token?;
                        crate::provider::store_provider_account_credential(
                            &config,
                            &owner,
                            "claude",
                            &profile.profile_id,
                            &token,
                            overwrite,
                        )?;
                        Ok(())
                    },
                );
                runtime.record_waiting_room_change();
                break;
            }
        }
        let _ = runtime
            .with_app_side_effect(|app| app.pty_mut().remove_process(&login_id))
            .await;
        let _ = runtime.provider_login_process_store().set_state(
            &owner,
            &login_id,
            ProviderLoginProcessState::Failed,
            crate::session::unix_epoch_ms(),
        );
    });
    Ok(LocalDaemonResponse::ProviderLoginStarted { login: workflow })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic() -> String {
        format!(
            "{}{}",
            std::str::from_utf8(TOKEN_PREFIX).unwrap(),
            "A".repeat(64)
        )
    }

    #[test]
    fn setup_token_capture_filters_every_chunk_before_projection() {
        let token = synthetic();
        let single = format!("Token: \x1b[32m{token}\x1b[0m\n");
        for size in 1..=single.len() {
            let mut capture = Capture::default();
            for chunk in single.as_bytes().chunks(size) {
                assert!(capture.read(chunk).is_empty());
            }
            capture.read(&[]);
            assert!(
                capture.token().is_ok_and(|value| *value == token),
                "chunk split lost private capture"
            );
        }
        let native = format!("https://claude.ai/oauth/authorize?client_id=fixture&code=true\nPaste code: \n\x1b[32m{token}\x1b[0m\nExport CLAUDE_CODE_OAUTH_TOKEN={token}\n");
        for size in 1..=native.len() {
            let mut capture = Capture::default();
            let mut projected = Vec::new();
            for chunk in native.as_bytes().chunks(size) {
                projected.extend(capture.read(chunk));
                assert!(
                    !projected
                        .windows(token.len())
                        .any(|part| part == token.as_bytes()),
                    "secret in reader projection"
                );
            }
            capture.read(&[]);
            assert!(
                capture.token().is_err(),
                "repeated candidates must fail closed"
            );
            let visible = String::from_utf8(projected).unwrap();
            assert!(visible.contains("https://claude.ai/oauth/authorize"));
            assert!(visible.contains("hidden provider response"));
            assert!(!visible.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        }
    }

    #[test]
    fn setup_token_capture_fails_closed_without_valid_complete_unique_token() {
        for output in [
            String::new(),
            "sk-ant-oat01-short".into(),
            format!("{}!", synthetic()),
            format!("{} {}", synthetic(), synthetic()),
        ] {
            let mut capture = Capture::default();
            capture.read(output.as_bytes());
            capture.read(&[]);
            assert!(capture.token().is_err());
        }
        let mut capture = Capture::default();
        capture.read(synthetic().as_bytes());
        assert!(capture.token().is_err());
        capture.read(&[]);
        assert!(capture.token().is_ok());
        capture.read(&vec![b'x'; CAPTURE_LIMIT]);
        assert!(capture.token().is_err());
        let mut capture = Capture::default();
        let url = format!("https://claude.ai/oauth/authorize?state={}\n", synthetic());
        assert!(capture.read(url.as_bytes()).is_empty());
        assert!(capture
            .read(b"https://evil.test/oauth/authorize?state=hidden\n")
            .is_empty());
    }
}
