use serde::{Deserialize, Serialize};

use std::path::Path;

use crate::artifacts::{ArtifactArchiveOutboxItem, ArtifactRecord, OperationalArtifactStore};
use crate::config::{HistoryArchiveMode, UserArchiveArtifactsConfig, UserArchiveHistoryConfig};
use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventQuery, OperationalHistoryStore};

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
pub struct HistoryArchiveSearchRequest {
    pub query: HistoryEventQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveSearchResponse {
    #[serde(default)]
    pub events: Vec<HistoryEvent>,
    #[serde(default)]
    pub next_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryArchiveRejectedEvent {
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactArchiveManifestRequest {
    pub artifacts: Vec<ArtifactRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactArchiveManifestResponse {
    #[serde(default)]
    pub accepted_artifact_ids: Vec<String>,
    #[serde(default)]
    pub rejected_artifacts: Vec<ArtifactArchiveRejectedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactArchiveRejectedArtifact {
    pub artifact_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryArchiveFlushOutcome {
    pub attempted_event_ids: Vec<String>,
    pub accepted_event_ids: Vec<String>,
    pub rejected_events: Vec<HistoryArchiveRejectedEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactArchiveFlushOutcome {
    pub attempted_artifact_ids: Vec<String>,
    pub accepted_artifact_ids: Vec<String>,
    pub rejected_artifacts: Vec<ArtifactArchiveRejectedArtifact>,
}

#[derive(Debug, Clone)]
pub struct HistoryArchiveExporter {
    store: OperationalHistoryStore,
    client: HistoryArchiveClient,
}

#[derive(Debug, Clone)]
pub struct ArtifactArchiveExporter {
    store: OperationalArtifactStore,
    client: HistoryArchiveClient,
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

impl HistoryArchiveExporter {
    pub fn new(store: OperationalHistoryStore, client: HistoryArchiveClient) -> Self {
        Self { store, client }
    }

    pub fn flush_pending_once(
        &self,
        limit: usize,
    ) -> Result<HistoryArchiveFlushOutcome, DaemonError> {
        let items = self.store.load_pending_archive_events(limit)?;
        if items.is_empty() {
            return Ok(HistoryArchiveFlushOutcome {
                attempted_event_ids: Vec::new(),
                accepted_event_ids: Vec::new(),
                rejected_events: Vec::new(),
            });
        }
        let events = items
            .iter()
            .map(|item| item.event.clone())
            .collect::<Vec<_>>();
        let attempted_event_ids = events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        match self.client.append_events(&events) {
            Ok(response) => {
                self.store
                    .mark_archive_events_accepted(&response.accepted_event_ids)?;
                let rejected_ids = response
                    .rejected_events
                    .iter()
                    .map(|event| event.event_id.clone())
                    .collect::<Vec<_>>();
                if !rejected_ids.is_empty() {
                    self.store.mark_archive_events_failed(
                        &rejected_ids,
                        "archive adapter rejected event",
                    )?;
                }
                Ok(HistoryArchiveFlushOutcome {
                    attempted_event_ids,
                    accepted_event_ids: response.accepted_event_ids,
                    rejected_events: response.rejected_events,
                })
            }
            Err(error) => {
                self.store
                    .mark_archive_events_failed(&attempted_event_ids, &error.to_string())?;
                Err(error)
            }
        }
    }
}

impl ArtifactArchiveExporter {
    pub fn new(store: OperationalArtifactStore, client: HistoryArchiveClient) -> Self {
        Self { store, client }
    }

    pub fn flush_pending_once(
        &self,
        limit: usize,
    ) -> Result<ArtifactArchiveFlushOutcome, DaemonError> {
        let items = self.store.load_pending_archive_artifacts(limit)?;
        if items.is_empty() {
            return Ok(ArtifactArchiveFlushOutcome {
                attempted_artifact_ids: Vec::new(),
                accepted_artifact_ids: Vec::new(),
                rejected_artifacts: Vec::new(),
            });
        }
        let attempted_artifact_ids = items
            .iter()
            .map(|item| item.record.artifact_id.clone())
            .collect::<Vec<_>>();
        match self.client.append_artifacts(&items) {
            Ok(response) => {
                self.store
                    .mark_archive_artifacts_accepted(&response.accepted_artifact_ids)?;
                let rejected_ids = response
                    .rejected_artifacts
                    .iter()
                    .map(|artifact| artifact.artifact_id.clone())
                    .collect::<Vec<_>>();
                if !rejected_ids.is_empty() {
                    self.store.mark_archive_artifacts_failed(
                        &rejected_ids,
                        "archive adapter rejected artifact",
                    )?;
                }
                Ok(ArtifactArchiveFlushOutcome {
                    attempted_artifact_ids,
                    accepted_artifact_ids: response.accepted_artifact_ids,
                    rejected_artifacts: response.rejected_artifacts,
                })
            }
            Err(error) => {
                self.store
                    .mark_archive_artifacts_failed(&attempted_artifact_ids, &error.to_string())?;
                Err(error)
            }
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

    pub fn from_artifact_config(config: &UserArchiveArtifactsConfig) -> Result<Self, DaemonError> {
        match config.mode {
            HistoryArchiveMode::Disabled => Ok(Self::Disabled),
            HistoryArchiveMode::External => {
                let base_url = config
                    .url
                    .as_deref()
                    .ok_or_else(|| DaemonError::InvalidConfig {
                        field: "artifacts.archive.url",
                        message: "value must be set when artifact archive mode is external",
                    })?
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
                if base_url.is_empty() {
                    return Err(DaemonError::InvalidConfig {
                        field: "artifacts.archive.url",
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

    pub fn search_events(
        &self,
        query: HistoryEventQuery,
    ) -> Result<HistoryArchiveSearchResponse, DaemonError> {
        match self {
            Self::Disabled => Ok(HistoryArchiveSearchResponse {
                events: Vec::new(),
                next_sequence: None,
            }),
            Self::External(client) => client.search_events(query),
        }
    }

    pub fn append_artifacts(
        &self,
        artifacts: &[ArtifactArchiveOutboxItem],
    ) -> Result<ArtifactArchiveManifestResponse, DaemonError> {
        match self {
            Self::Disabled => Ok(ArtifactArchiveManifestResponse {
                accepted_artifact_ids: Vec::new(),
                rejected_artifacts: artifacts
                    .iter()
                    .map(|artifact| ArtifactArchiveRejectedArtifact {
                        artifact_id: artifact.record.artifact_id.clone(),
                        reason: "artifact archive is disabled".to_string(),
                    })
                    .collect(),
            }),
            Self::External(client) => client.append_artifacts(artifacts),
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

    fn search_events(
        &self,
        query: HistoryEventQuery,
    ) -> Result<HistoryArchiveSearchResponse, DaemonError> {
        let request_body = HistoryArchiveSearchRequest { query };
        let payload = serde_json::to_string(&request_body).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "encode history archive search request",
                message: error.to_string(),
            }
        })?;
        let request = self
            .authorized_request(ureq::post(&self.endpoint("/arroba/history/search")))?
            .set("content-type", "application/json");
        let response = request
            .send_string(&payload)
            .map_err(|error| archive_http_error("search", error))?;
        decode_response_json::<HistoryArchiveSearchResponse>(response, "history.archive.search")
    }

    fn append_artifacts(
        &self,
        artifacts: &[ArtifactArchiveOutboxItem],
    ) -> Result<ArtifactArchiveManifestResponse, DaemonError> {
        for artifact in artifacts {
            self.put_artifact_blob(&artifact.record, &artifact.record.operational_path)?;
        }
        let records = artifacts
            .iter()
            .map(|artifact| artifact.record.clone())
            .collect::<Vec<_>>();
        let request_body = ArtifactArchiveManifestRequest {
            artifacts: records.clone(),
        };
        let payload = serde_json::to_string(&request_body).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "encode artifact archive manifest request",
                message: error.to_string(),
            }
        })?;
        let request = self
            .authorized_request(ureq::post(&self.endpoint("/arroba/artifacts/manifest")))?
            .set("content-type", "application/json");
        let response = request
            .send_string(&payload)
            .map_err(|error| archive_http_error("artifact manifest", error))?;
        let archive_response = decode_response_json::<ArtifactArchiveManifestResponse>(
            response,
            "artifacts.archive.manifest",
        )?;
        if self.require_durable_acceptance {
            require_all_artifacts_accepted(&records, &archive_response)?;
        }
        Ok(archive_response)
    }

    fn put_artifact_blob(&self, record: &ArtifactRecord, path: &Path) -> Result<(), DaemonError> {
        let bytes = std::fs::read(path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: record.session_id.clone(),
            operation: "read artifact archive blob",
            message: error.to_string(),
        })?;
        let request = self
            .authorized_request(ureq::put(
                &self.endpoint(&format!("/arroba/artifacts/blobs/{}", record.artifact_id)),
            ))?
            .set("content-type", "application/octet-stream")
            .set("x-arroba-sha256", &record.sha256)
            .set("x-arroba-size-bytes", &record.size_bytes.to_string());
        let response = request
            .send_bytes(&bytes)
            .map_err(|error| archive_http_error("artifact blob", error))?;
        if !(200..300).contains(&response.status()) {
            return Err(DaemonError::LocalTransport {
                operation: "artifacts.archive.blob",
                message: format!(
                    "artifact blob upload failed with HTTP {}",
                    response.status()
                ),
            });
        }
        Ok(())
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

fn require_all_artifacts_accepted(
    artifacts: &[ArtifactRecord],
    response: &ArtifactArchiveManifestResponse,
) -> Result<(), DaemonError> {
    let accepted = response
        .accepted_artifact_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let missing = artifacts
        .iter()
        .filter(|artifact| !accepted.contains(artifact.artifact_id.as_str()))
        .map(|artifact| artifact.artifact_id.clone())
        .collect::<Vec<_>>();
    if missing.is_empty() && response.rejected_artifacts.is_empty() {
        return Ok(());
    }
    let rejected = response
        .rejected_artifacts
        .iter()
        .map(|artifact| format!("{}: {}", artifact.artifact_id, artifact.reason))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DaemonError::SessionHistoryFailed {
        session_id: None,
        operation: "verify artifact archive acceptance",
        message: format!(
            "archive adapter did not durably accept every artifact; missing=[{}], rejected=[{}]",
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

    #[test]
    fn exporter_flushes_pending_events_through_adapter() {
        let path = std::env::temp_dir().join(format!(
            "arroba-archive-exporter-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store =
            OperationalHistoryStore::open(path.clone()).expect("operational store should open");
        store
            .enqueue_archive_events(&[test_event("evt-flush")])
            .expect("event should enqueue");
        let (base_url, handle) = spawn_archive_server(
            "POST /arroba/history/events",
            r#"{"accepted_event_ids":["evt-flush"],"rejected_events":[]}"#,
        );
        let config = UserArchiveHistoryConfig {
            mode: HistoryArchiveMode::External,
            url: Some(base_url),
            token_env: None,
            require_durable_acceptance: Some(true),
            ..UserArchiveHistoryConfig::default()
        };
        let client = HistoryArchiveClient::from_config(&config).expect("client should build");
        let exporter = HistoryArchiveExporter::new(store.clone(), client);

        let outcome = exporter
            .flush_pending_once(10)
            .expect("flush should archive event");

        assert_eq!(outcome.attempted_event_ids, vec!["evt-flush"]);
        assert_eq!(outcome.accepted_event_ids, vec!["evt-flush"]);
        assert!(store
            .load_pending_archive_events(10)
            .expect("pending events should load")
            .is_empty());
        handle.join().expect("server should join");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
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
