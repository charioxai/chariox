use serde::{Deserialize, Serialize};

use crate::config::{HistoryArchiveMode, UserArchiveHistoryConfig};
use crate::error::DaemonError;
use crate::history::HistoryEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryArchiveClient {
    Disabled,
    External(ExternalHistoryArchiveClient),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalHistoryArchiveClient {
    base_url: String,
    token_env: Option<String>,
    require_durable_acceptance: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveCapabilities {
    #[serde(default)]
    pub append: bool,
    #[serde(default)]
    pub query: bool,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub full_text_search: bool,
    #[serde(default)]
    pub blob_refs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveAppendRequest {
    pub events: Vec<HistoryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveAppendResponse {
    #[serde(default)]
    pub accepted_event_ids: Vec<String>,
    #[serde(default)]
    pub rejected_events: Vec<HistoryArchiveRejectedEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveRejectedEvent {
    pub event_id: String,
    pub reason: String,
}

impl Default for HistoryArchiveCapabilities {
    fn default() -> Self {
        Self {
            append: false,
            query: false,
            search: false,
            full_text_search: false,
            blob_refs: false,
        }
    }
}

impl HistoryArchiveClient {
    pub fn from_config(config: &UserArchiveHistoryConfig) -> Result<Self, DaemonError> {
        match config.mode {
            HistoryArchiveMode::Disabled => Ok(Self::Disabled),
            HistoryArchiveMode::External => {
                let base_url = config
                    .url
                    .as_deref()
                    .ok_or_else(|| DaemonError::InvalidConfig {
                        field: "history.archive.url",
                        message: "value must be set when archive mode is external",
                    })?
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
                if base_url.is_empty() {
                    return Err(DaemonError::InvalidConfig {
                        field: "history.archive.url",
                        message: "value must not be empty",
                    });
                }
                Ok(Self::External(ExternalHistoryArchiveClient {
                    base_url,
                    token_env: config.token_env.clone(),
                    require_durable_acceptance: config.require_durable_acceptance.unwrap_or(true),
                }))
            }
        }
    }

    pub fn capabilities(&self) -> Result<HistoryArchiveCapabilities, DaemonError> {
        match self {
            Self::Disabled => Ok(HistoryArchiveCapabilities::default()),
            Self::External(client) => client.capabilities(),
        }
    }

    pub fn append_events(
        &self,
        events: &[HistoryEvent],
    ) -> Result<HistoryArchiveAppendResponse, DaemonError> {
        match self {
            Self::Disabled => Ok(HistoryArchiveAppendResponse {
                accepted_event_ids: Vec::new(),
                rejected_events: events
                    .iter()
                    .map(|event| HistoryArchiveRejectedEvent {
                        event_id: event.event_id.clone(),
                        reason: "history archive is disabled".to_string(),
                    })
                    .collect(),
            }),
            Self::External(client) => client.append_events(events),
        }
    }
}

impl ExternalHistoryArchiveClient {
    fn capabilities(&self) -> Result<HistoryArchiveCapabilities, DaemonError> {
        let request =
            self.authorized_request(ureq::get(&self.endpoint("/arroba/history/capabilities")))?;
        let response = request
            .call()
            .map_err(|error| archive_http_error("capabilities", error))?;
        decode_response_json(response, "history.archive.capabilities")
    }

    fn append_events(
        &self,
        events: &[HistoryEvent],
    ) -> Result<HistoryArchiveAppendResponse, DaemonError> {
        let request_body = HistoryArchiveAppendRequest {
            events: events.to_vec(),
        };
        let payload = serde_json::to_string(&request_body).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "encode history archive append request",
                message: error.to_string(),
            }
        })?;
        let request = self
            .authorized_request(ureq::post(&self.endpoint("/arroba/history/events")))?
            .set("content-type", "application/json");
        let response = request
            .send_string(&payload)
            .map_err(|error| archive_http_error("append", error))?;
        let archive_response = decode_response_json::<HistoryArchiveAppendResponse>(
            response,
            "history.archive.append",
        )?;
        if self.require_durable_acceptance {
            require_all_events_accepted(events, &archive_response)?;
        }
        Ok(archive_response)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn authorized_request(&self, request: ureq::Request) -> Result<ureq::Request, DaemonError> {
        let Some(token_env) = self.token_env.as_deref() else {
            return Ok(request);
        };
        let token = std::env::var(token_env).map_err(|error| DaemonError::LocalTransport {
            operation: "history.archive.auth",
            message: format!("failed to read token env `{token_env}`: {error}"),
        })?;
        if token.trim().is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "history.archive.auth",
                message: format!("token env `{token_env}` is empty"),
            });
        }
        Ok(request.set("authorization", &format!("Bearer {}", token.trim())))
    }
}

fn require_all_events_accepted(
    events: &[HistoryEvent],
    response: &HistoryArchiveAppendResponse,
) -> Result<(), DaemonError> {
    let accepted = response
        .accepted_event_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let missing = events
        .iter()
        .filter(|event| !accepted.contains(event.event_id.as_str()))
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    if missing.is_empty() && response.rejected_events.is_empty() {
        return Ok(());
    }
    let rejected = response
        .rejected_events
        .iter()
        .map(|event| format!("{}: {}", event.event_id, event.reason))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DaemonError::SessionHistoryFailed {
        session_id: None,
        operation: "verify history archive acceptance",
        message: format!(
            "archive adapter did not durably accept every event; missing=[{}], rejected=[{}]",
            missing.join(", "),
            rejected
        ),
    })
}

fn archive_http_error(operation: &'static str, error: ureq::Error) -> DaemonError {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            DaemonError::LocalTransport {
                operation: "history.archive.http",
                message: format!("{operation} failed with HTTP {code}: {body}"),
            }
        }
        ureq::Error::Transport(error) => DaemonError::LocalTransport {
            operation: "history.archive.http",
            message: format!("{operation} transport failed: {error}"),
        },
    }
}

fn decode_response_json<T: serde::de::DeserializeOwned>(
    response: ureq::Response,
    operation: &'static str,
) -> Result<T, DaemonError> {
    let body = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&body).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::history::{HistoryEvent, HistoryEventKind};

    #[test]
    fn disabled_archive_rejects_appends_without_io() {
        let client = HistoryArchiveClient::from_config(&UserArchiveHistoryConfig::default())
            .expect("disabled client should build");
        let event = test_event("evt-disabled");
        let response = client
            .append_events(&[event])
            .expect("disabled append should be reported locally");

        assert!(response.accepted_event_ids.is_empty());
        assert_eq!(response.rejected_events.len(), 1);
        assert_eq!(response.rejected_events[0].event_id, "evt-disabled");
    }

    #[test]
    fn external_archive_posts_events_and_requires_acceptance() {
        let (base_url, handle) = spawn_archive_server(
            "POST /arroba/history/events",
            r#"{"accepted_event_ids":["evt-accepted"],"rejected_events":[]}"#,
        );
        let config = UserArchiveHistoryConfig {
            mode: HistoryArchiveMode::External,
            url: Some(base_url),
            token_env: None,
            require_durable_acceptance: Some(true),
            ..UserArchiveHistoryConfig::default()
        };
        let client = HistoryArchiveClient::from_config(&config).expect("client should build");
        let response = client
            .append_events(&[test_event("evt-accepted")])
            .expect("adapter should accept event");

        assert_eq!(response.accepted_event_ids, vec!["evt-accepted"]);
        handle.join().expect("server should join");
    }

    #[test]
    fn external_archive_surfaces_missing_acceptance() {
        let (base_url, handle) = spawn_archive_server(
            "POST /arroba/history/events",
            r#"{"accepted_event_ids":[],"rejected_events":[{"event_id":"evt-missing","reason":"nope"}]}"#,
        );
        let config = UserArchiveHistoryConfig {
            mode: HistoryArchiveMode::External,
            url: Some(base_url),
            token_env: None,
            require_durable_acceptance: Some(true),
            ..UserArchiveHistoryConfig::default()
        };
        let client = HistoryArchiveClient::from_config(&config).expect("client should build");
        let error = client
            .append_events(&[test_event("evt-missing")])
            .expect_err("missing acceptance should fail");

        assert!(error
            .to_string()
            .contains("did not durably accept every event"));
        handle.join().expect("server should join");
    }

    fn test_event(event_id: &str) -> HistoryEvent {
        HistoryEvent {
            event_id: event_id.to_string(),
            sequence: 1,
            timestamp_ms: 1,
            workspace_id: None,
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            agent_alias: None,
            provider: Some("codex".to_string()),
            model: Some("gpt-5.4".to_string()),
            turn_id: None,
            prompt_id: None,
            provider_run_id: None,
            provider_session_id: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_node_id: None,
            machine_id: None,
            repo_root: None,
            worktree_path: None,
            kind: HistoryEventKind::UserPrompt,
            role: None,
            content: Some("hello".to_string()),
            content_ref: None,
            metadata: Default::default(),
            candidate_agent_ids: Vec::new(),
            candidate_prompt_ids: Vec::new(),
            candidate_turn_ids: Vec::new(),
            attribution_confidence: None,
            caused_by_event_id: None,
        }
    }

    fn spawn_archive_server(
        expected_request_line: &'static str,
        response_body: &'static str,
    ) -> (String, thread::JoinHandle<()>) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("test server should bind an ephemeral port");
        let address = listener.local_addr().expect("test server should have addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept one request");
            let mut reader = BufReader::new(stream.try_clone().expect("stream should clone"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("request line should read");
            assert!(
                request_line.trim_end().starts_with(expected_request_line),
                "request line `{}` should start with `{expected_request_line}`",
                request_line.trim_end()
            );
            let mut content_length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header should read");
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.strip_prefix("Content-Length: ") {
                    content_length = value.parse().expect("content length should parse");
                }
            }
            if content_length > 0 {
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).expect("body should read");
                assert!(String::from_utf8_lossy(&body).contains("\"events\""));
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("response should write");
        });
        (format!("http://{address}"), handle)
    }
}
