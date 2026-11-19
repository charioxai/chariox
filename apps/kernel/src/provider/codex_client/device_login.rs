//! A device login completes only when the official app-server acknowledges that attempt.
use super::{CodexClient, CodexSocket, ProviderLoginStart};
use crate::error::DaemonError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::{Error, Message};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    #[serde(rename = "type")]
    kind: String,
    login_id: String,
    verification_url: Option<String>,
    user_code: Option<String>,
    auth_url: Option<String>,
}

pub(crate) struct CodexDeviceLogin {
    pub(crate) start: ProviderLoginStart,
    socket: CodexSocket,
    completion: Option<bool>,
    _helper: Arc<()>,
}

fn failed() -> DaemonError {
    DaemonError::LocalTransport { operation: "codex_device_login", message: "The Codex device sign-in disconnected or returned an invalid result; start sign-in again".into() }
}

impl CodexClient {
    pub fn start_login(&self, account_profile: &str) -> Result<ProviderLoginStart, DaemonError> {
        self.start_device_login(account_profile, Arc::new(()))
            .map(|session| session.start)
    }

    pub(crate) fn start_device_login(
        &self,
        account_profile: &str,
        helper: Arc<()>,
    ) -> Result<CodexDeviceLogin, DaemonError> {
        let mut socket = self.connect_initialized()?;
        socket
            .send(Message::Text(
                json!({"jsonrpc":"2.0", "id":1, "method":"account/login/start",
            "params":{"type":"chatgptDeviceCode"}})
                .to_string()
                .into(),
            ))
            .map_err(|_| failed())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut early = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(failed());
            }
            let raw = self
                .read_next_message(&mut socket, remaining)
                .map_err(|_| failed())?;
            let value: Value = serde_json::from_str(&raw).map_err(|_| failed())?;
            if value["id"] == 1 {
                let started: Started =
                    serde_json::from_value(value["result"].clone()).map_err(|_| failed())?;
                if started.kind != "chatgptDeviceCode"
                    || started.login_id.is_empty()
                    || started
                        .verification_url
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || started.user_code.as_deref().is_none_or(str::is_empty)
                {
                    return Err(failed());
                }
                let mut session = CodexDeviceLogin {
                    start: ProviderLoginStart {
                        provider: "codex".into(),
                        account_profile: account_profile.into(),
                        login_kind: started.kind,
                        login_id: Some(started.login_id),
                        auth_url: started.auth_url,
                        verification_url: started.verification_url,
                        user_code: started.user_code,
                    },
                    socket,
                    completion: None,
                    _helper: helper,
                };
                for value in early {
                    session.observe(&value);
                }
                return Ok(session);
            }
            if early.len() < 64 && value["method"] == "account/login/completed" {
                early.push(value);
            }
        }
    }
}

impl CodexDeviceLogin {
    fn observe(&mut self, value: &Value) {
        if value["method"] == "account/login/completed"
            && value["params"]["loginId"].as_str() == self.start.login_id.as_deref()
        {
            self.completion = Some(value["params"]["success"].as_bool().unwrap_or(false));
        }
    }

    pub(crate) fn poll_completion(&mut self) -> Result<Option<bool>, DaemonError> {
        if self.completion.is_some() {
            return Ok(self.completion);
        }
        for _ in 0..64 {
            match super::socket_io::read_socket_nonblocking(&mut self.socket) {
                Ok(Message::Text(text)) => {
                    let value = serde_json::from_str(&text).map_err(|_| failed())?;
                    self.observe(&value);
                    if self.completion.is_some() {
                        return Ok(self.completion);
                    }
                }
                Ok(Message::Close(_)) => return Err(failed()),
                Ok(_) => {}
                Err(Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None)
                }
                Err(_) => return Err(failed()),
            }
        }
        Ok(None)
    }
}
