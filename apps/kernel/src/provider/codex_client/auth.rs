use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::DaemonError;

use super::{resolve_codex_executable, CodexClient};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub auth_state: String,
    pub account_profile: Option<String>,
    pub login_hint: Option<String>,
    pub detected_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLoginStart {
    pub provider: String,
    pub login_kind: String,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexGetAccountResponse {
    account: Option<CodexAccount>,
    #[serde(rename = "requiresOpenaiAuth")]
    requires_openai_auth: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAccount {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexLoginStartResponse {
    #[serde(rename = "type")]
    login_kind: String,
    #[serde(rename = "loginId", default)]
    login_id: Option<String>,
    #[serde(rename = "authUrl", default)]
    auth_url: Option<String>,
    #[serde(rename = "verificationUrl", default)]
    verification_url: Option<String>,
    #[serde(rename = "userCode", default)]
    user_code: Option<String>,
}

impl CodexClient {
    pub fn auth_status(&self) -> Result<ProviderAuthStatus, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexGetAccountResponse =
            self.send_request(&mut socket, &mut next_request_id, "account/read", json!({}))?;
        Ok(ProviderAuthStatus {
            provider: "codex".to_string(),
            auth_state: if response.account.is_some() {
                "authenticated".to_string()
            } else if response.requires_openai_auth {
                "not_logged_in".to_string()
            } else {
                "unknown".to_string()
            },
            account_profile: response.account.and_then(|account| account.email),
            login_hint: Some("Run /provider login codex to authenticate Codex.".to_string()),
            detected_version: codex_version().ok(),
        })
    }

    pub fn start_login(&self) -> Result<ProviderLoginStart, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexLoginStartResponse = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/start",
            json!({ "type": "chatgptDeviceCode" }),
        )?;
        Ok(ProviderLoginStart {
            provider: "codex".to_string(),
            login_kind: response.login_kind,
            login_id: response.login_id,
            auth_url: response.auth_url,
            verification_url: response.verification_url,
            user_code: response.user_code,
        })
    }
}

fn codex_version() -> Result<String, DaemonError> {
    let executable = resolve_codex_executable()?;
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_version",
            message: error.to_string(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok(stderr);
    }
    Err(DaemonError::LocalTransport {
        operation: "codex_version",
        message: "codex returned no version text".to_string(),
    })
}
