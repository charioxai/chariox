use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

static REMOTE_EXTENSION_INVOCATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Mcp,
    Skill,
    Script,
    Connector,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionSource {
    #[default]
    Home,
    Worker,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCatalogSource {
    Home,
    Worker,
    #[default]
    All,
}

impl ExtensionCatalogSource {
    pub fn includes(self, source: ExtensionSource) -> bool {
        matches!(self, Self::All)
            || matches!(
                (self, source),
                (Self::Home, ExtensionSource::Home) | (Self::Worker, ExtensionSource::Worker)
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionCatalogEntry {
    pub source: ExtensionSource,
    pub resolved_kernel_id: String,
    pub kind: ExtensionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_hash: Option<String>,
    #[serde(default)]
    pub environments: Vec<String>,
    #[serde(default)]
    pub credentials: Vec<String>,
    #[serde(default)]
    pub credential_required: bool,
    #[serde(default)]
    pub max_safety: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExtensionCatalog {
    pub agent_id: String,
    pub home_kernel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
    pub worker_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_error: Option<String>,
    #[serde(default)]
    pub entries: Vec<ExtensionCatalogEntry>,
}

impl ExtensionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skill",
            Self::Script => "script",
            Self::Connector => "connector",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionAuthority {
    Home,
    Worker,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionDefinitionOrigin {
    Home,
    Worker,
    ProjectedSnapshot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionExecutionLocation {
    Home,
    Worker,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<RemoteExtensionTool>,
}

impl RemoteExtensionManifest {
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn manifest_hash(&self) -> String {
        let serialized =
            serde_json::to_vec(self).unwrap_or_else(|_| b"remote-extension-manifest".to_vec());
        let digest = Sha256::digest(&serialized);
        hex_digest(&digest)
    }

    pub fn validate_unique_tool_names(
        &self,
        operation: &'static str,
    ) -> Result<(), crate::error::DaemonError> {
        let mut seen = std::collections::BTreeMap::<&str, &RemoteExtensionTool>::new();
        for tool in &self.tools {
            if let Some(existing) = seen.insert(tool.tool_name.as_str(), tool) {
                return Err(crate::error::DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy extension tool name `{}` is duplicated by `{}:{}` and `{}:{}`",
                        tool.tool_name,
                        existing.kind.as_str(),
                        existing.name,
                        tool.kind.as_str(),
                        tool.name
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn home_proxy_runtime_tool_specs(
        &self,
    ) -> impl Iterator<Item = crate::transport::runtime_tools::RuntimeToolSpec> + '_ {
        self.tools
            .iter()
            .filter(|tool| tool.execution_location == ExtensionExecutionLocation::Home)
            .filter(|tool| matches!(tool.kind, ExtensionKind::Script | ExtensionKind::Connector))
            .map(|tool| crate::transport::runtime_tools::RuntimeToolSpec {
                name: tool.tool_name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
    }

    pub fn home_proxy_mcp_server_names(&self) -> impl Iterator<Item = &str> {
        self.tools
            .iter()
            .filter(|tool| tool.execution_location == ExtensionExecutionLocation::Home)
            .filter(|tool| tool.kind == ExtensionKind::Mcp)
            .map(|tool| tool.tool_name.as_str())
    }

    pub fn without_mcp_tools(mut self) -> Self {
        self.tools.retain(|tool| tool.kind != ExtensionKind::Mcp);
        self
    }

    pub fn home_proxy_tool(&self, tool_name: &str) -> Option<&RemoteExtensionTool> {
        self.tools.iter().find(|tool| {
            tool.execution_location == ExtensionExecutionLocation::Home
                && tool.tool_name == tool_name
        })
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionTool {
    pub kind: ExtensionKind,
    pub name: String,
    pub tool_name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    pub authority: ExtensionAuthority,
    pub definition_origin: ExtensionDefinitionOrigin,
    pub execution_location: ExtensionExecutionLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionInvocationMetadata {
    pub invocation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_call_id: Option<String>,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub started_at_ms: u64,
}

impl RemoteExtensionInvocationMetadata {
    pub fn new(
        provider_run_id: &str,
        tool_name: &str,
        provider_tool_call_id: Option<String>,
    ) -> Self {
        let started_at_ms = crate::session::unix_epoch_ms();
        let sanitized_tool_name = tool_name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let sequence = REMOTE_EXTENSION_INVOCATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            invocation_id: format!(
                "{provider_run_id}:{sanitized_tool_name}:{started_at_ms}:{sequence}"
            ),
            provider_tool_call_id,
            attempt: 1,
            idempotency_key: None,
            started_at_ms,
        }
    }
}

impl Default for RemoteExtensionInvocationMetadata {
    fn default() -> Self {
        let started_at_ms = crate::session::unix_epoch_ms();
        let sequence = REMOTE_EXTENSION_INVOCATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            invocation_id: format!("legacy-{started_at_ms}-{sequence}"),
            provider_tool_call_id: None,
            attempt: 1,
            idempotency_key: None,
            started_at_ms,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteExtensionManifestSyncState {
    Synced,
    Syncing,
    Pending,
    Failed,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionManifestSyncStatus {
    pub state: RemoteExtensionManifestSyncState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempted_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_revoke: Option<bool>,
}

impl RemoteExtensionManifestSyncStatus {
    pub fn pending(manifest_hash: String, pending_revoke: bool) -> Self {
        Self {
            state: RemoteExtensionManifestSyncState::Pending,
            manifest_hash: Some(manifest_hash),
            last_attempted_at_ms: None,
            last_synced_at_ms: None,
            last_error: None,
            pending_revoke: pending_revoke.then_some(true),
        }
    }

    pub fn syncing(mut self) -> Self {
        self.state = RemoteExtensionManifestSyncState::Syncing;
        self.last_attempted_at_ms = Some(next_remote_extension_sync_attempt_ms());
        self.last_error = None;
        self
    }

    pub fn synced(manifest_hash: String) -> Self {
        let now = crate::session::unix_epoch_ms();
        Self {
            state: RemoteExtensionManifestSyncState::Synced,
            manifest_hash: Some(manifest_hash),
            last_attempted_at_ms: Some(now),
            last_synced_at_ms: Some(now),
            last_error: None,
            pending_revoke: None,
        }
    }

    pub fn failed(mut self, error: impl Into<String>) -> Self {
        self.state = RemoteExtensionManifestSyncState::Failed;
        self.last_attempted_at_ms = Some(crate::session::unix_epoch_ms());
        self.last_error = Some(error.into());
        self
    }
}

fn next_remote_extension_sync_attempt_ms() -> u64 {
    static LAST_ATTEMPT_MS: AtomicU64 = AtomicU64::new(0);
    let now = crate::session::unix_epoch_ms();
    loop {
        let previous = LAST_ATTEMPT_MS.load(Ordering::Relaxed);
        let next = now.max(previous.saturating_add(1));
        if LAST_ATTEMPT_MS
            .compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return next;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExtensionGrant {
    #[serde(default)]
    pub source: ExtensionSource,
    pub kind: ExtensionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, alias = "maxSafety", skip_serializing_if = "Option::is_none")]
    pub max_safety: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(kind: ExtensionKind, name: &str) -> RemoteExtensionTool {
        RemoteExtensionTool {
            kind,
            name: name.to_string(),
            tool_name: name.to_string(),
            description: format!("{name} description"),
            input_schema: serde_json::json!({"type": "object"}),
            authority: ExtensionAuthority::Home,
            definition_origin: ExtensionDefinitionOrigin::Home,
            execution_location: ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: None,
            version_hash: None,
        }
    }

    #[test]
    fn remote_manifest_projects_runtime_tools_but_not_mcp_servers() {
        let manifest = RemoteExtensionManifest {
            tools: vec![
                tool(ExtensionKind::Script, "home_script"),
                tool(ExtensionKind::Connector, "home_connector_lookup"),
                tool(ExtensionKind::Mcp, "home_browser"),
            ],
        };

        let specs = manifest
            .home_proxy_runtime_tool_specs()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert_eq!(specs, vec!["home_script", "home_connector_lookup"]);
        assert_eq!(
            manifest.home_proxy_mcp_server_names().collect::<Vec<_>>(),
            vec!["home_browser"]
        );
        assert!(manifest.home_proxy_tool("home_script").is_some());
        assert!(manifest.home_proxy_tool("missing").is_none());
    }

    #[test]
    fn native_worker_manifest_removes_mcp_tools_only() {
        let manifest = RemoteExtensionManifest {
            tools: vec![
                tool(ExtensionKind::Script, "home_script"),
                tool(ExtensionKind::Connector, "home_connector_lookup"),
                tool(ExtensionKind::Mcp, "worker_browser"),
            ],
        }
        .without_mcp_tools();

        assert_eq!(
            manifest
                .tools
                .iter()
                .map(|tool| tool.tool_name.as_str())
                .collect::<Vec<_>>(),
            vec!["home_script", "home_connector_lookup"]
        );
    }

    #[test]
    fn remote_manifest_rejects_duplicate_home_proxy_tool_names() {
        let manifest = RemoteExtensionManifest {
            tools: vec![
                tool(ExtensionKind::Script, "shared_name"),
                tool(ExtensionKind::Connector, "shared_name"),
            ],
        };

        let error = manifest
            .validate_unique_tool_names("test manifest")
            .expect_err("duplicate tool names should be rejected");

        assert!(error.to_string().contains("duplicated"));
    }

    #[test]
    fn remote_manifest_hash_changes_with_projected_tools() {
        let first = RemoteExtensionManifest {
            tools: vec![tool(ExtensionKind::Script, "home_script")],
        };
        let second = RemoteExtensionManifest {
            tools: vec![tool(ExtensionKind::Script, "home_script_v2")],
        };

        assert_ne!(first.manifest_hash(), second.manifest_hash());
    }

    #[test]
    fn remote_manifest_sync_status_records_pending_revoke_and_failure() {
        let pending = RemoteExtensionManifestSyncStatus::pending("hash-1".to_string(), true);
        assert_eq!(pending.state, RemoteExtensionManifestSyncState::Pending);
        assert_eq!(pending.pending_revoke, Some(true));

        let failed = pending.syncing().failed("relay unavailable");
        assert_eq!(failed.state, RemoteExtensionManifestSyncState::Failed);
        assert_eq!(failed.last_error.as_deref(), Some("relay unavailable"));
        assert!(failed.last_attempted_at_ms.is_some());
    }

    #[test]
    fn remote_extension_invocation_ids_are_unique_for_fast_calls() {
        let first = RemoteExtensionInvocationMetadata::new("run-1", "tool", None);
        let second = RemoteExtensionInvocationMetadata::new("run-1", "tool", None);

        assert_ne!(first.invocation_id, second.invocation_id);
        assert!(first.invocation_id.starts_with("run-1:tool:"));
        assert!(second.invocation_id.starts_with("run-1:tool:"));
    }

    #[test]
    fn legacy_extension_grant_defaults_to_home_source() {
        let grant: ExtensionGrant = serde_json::from_value(serde_json::json!({
            "kind": "skill",
            "name": "review"
        }))
        .expect("legacy grant should deserialize");

        assert_eq!(grant.source, ExtensionSource::Home);
        assert_eq!(
            serde_json::to_value(grant)
                .expect("grant should serialize")
                .pointer("/source"),
            Some(&serde_json::json!("home"))
        );
    }

    #[test]
    fn grant_match_identity_includes_source() {
        let worker = ExtensionGrant::new(ExtensionKind::Skill, "review")
            .from_source(ExtensionSource::Worker);

        assert!(worker.matches(ExtensionSource::Worker, &ExtensionKind::Skill, "review"));
        assert!(!worker.matches(ExtensionSource::Home, &ExtensionKind::Skill, "review"));
    }
}

impl ExtensionGrant {
    pub fn new(kind: ExtensionKind, name: impl Into<String>) -> Self {
        Self {
            source: ExtensionSource::Home,
            kind,
            name: name.into(),
            environment: None,
            credential: None,
            max_safety: None,
        }
    }

    pub fn script(name: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            source: ExtensionSource::Home,
            kind: ExtensionKind::Script,
            name: name.into(),
            environment: Some(environment.into()),
            credential: None,
            max_safety: None,
        }
    }

    pub fn connector(
        name: impl Into<String>,
        credential: Option<String>,
        max_safety: impl Into<String>,
    ) -> Self {
        Self {
            source: ExtensionSource::Home,
            kind: ExtensionKind::Connector,
            name: name.into(),
            environment: None,
            credential,
            max_safety: Some(max_safety.into()),
        }
    }

    pub fn from_source(mut self, source: ExtensionSource) -> Self {
        self.source = source;
        self
    }

    pub fn matches(&self, source: ExtensionSource, kind: &ExtensionKind, name: &str) -> bool {
        self.source == source && &self.kind == kind && self.name == name
    }
}

pub(crate) fn extension_grant_manifest_hash(
    grants: &[ExtensionGrant],
) -> Result<String, crate::error::DaemonError> {
    let mut grants = grants.to_vec();
    grants.sort();
    let bytes =
        serde_json::to_vec(&grants).map_err(|error| crate::error::DaemonError::LocalTransport {
            operation: "worker extension grant sync",
            message: format!("failed to serialize worker extension grants: {error}"),
        })?;
    let digest = Sha256::digest(bytes);
    Ok(hex_digest(&digest))
}
