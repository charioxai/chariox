//! OpenCode session endpoint operations and snapshots.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::DaemonError;

use super::{OpenCodeClient, OpenCodeMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeSessionSnapshot {
    pub status: String,
    pub messages: Vec<OpenCodeMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionCreated {
    id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeSessionStatus {
    #[serde(rename = "type")]
    pub(super) kind: String,
}

impl OpenCodeClient {
    pub fn create_session(&self, permission: Option<Value>) -> Result<String, DaemonError> {
        let mut body = json!({});
        if let Some(permission) = permission {
            body["permission"] = permission;
        }
        let created: OpenCodeSessionCreated =
            self.send_json_request("POST", "/session", Some(&body))?;
        Ok(created.id)
    }

    pub fn create_session_with_retry(
        &self,
        permission: Option<Value>,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<String, DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.create_session(permission.clone()) {
                Ok(session_id) => return Ok(session_id),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }

    pub fn abort_session(&self, session_id: &str) -> Result<(), DaemonError> {
        self.send_json_request::<serde_json::Value>(
            "POST",
            &format!("/session/{session_id}/abort"),
            Some(&json!({})),
        )?;
        Ok(())
    }

    pub fn reply_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<(), DaemonError> {
        let _: bool = self.send_json_request(
            "POST",
            &format!("/session/{session_id}/permissions/{permission_id}"),
            Some(&json!({ "response": response })),
        )?;
        Ok(())
    }

    pub fn snapshot(&self, session_id: &str) -> Result<OpenCodeSessionSnapshot, DaemonError> {
        let status = self.session_status(session_id)?;
        let messages = self.messages(session_id)?;

        Ok(OpenCodeSessionSnapshot { status, messages })
    }

    pub fn session_status(&self, session_id: &str) -> Result<String, DaemonError> {
        let status_map: BTreeMap<String, OpenCodeSessionStatus> =
            self.send_json_request("GET", "/session/status", None)?;
        // OpenCode removes idle sessions from SessionStatus.list(), so omission means idle.
        Ok(status_map
            .get(session_id)
            .map(|status| status.kind.clone())
            .unwrap_or_else(|| "idle".to_string()))
    }

    pub fn messages(&self, session_id: &str) -> Result<Vec<OpenCodeMessage>, DaemonError> {
        self.send_json_request("GET", &format!("/session/{session_id}/message"), None)
    }
}
