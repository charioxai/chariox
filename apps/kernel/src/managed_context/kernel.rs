use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::UserCredentialConfig;
use crate::connector::CharioxConnectorDefinition;
use crate::extension::ExtensionKind;
use crate::mcp::CharioxMcpServerConfig;
use crate::script::CharioxScriptRuntime;

mod export;
mod import;
mod source_snapshot;
pub use export::export_kernel_context;
pub use import::import_kernel_context;
pub(crate) use import::{cleanup_kernel_context_import, configured_managed_kernel_context_paths};
pub use source_snapshot::scavenge_source_snapshots;
pub(crate) const MAX_KERNEL_CONTEXT_SNAPSHOT_BYTES: usize = export::MAX_SNAPSHOT_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelContextExportRequest {
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub vault: crate::secret::TransferredVaultSnapshot,
}

#[derive(Clone, PartialEq)]
pub struct KernelContextImportRequest {
    pub snapshot: KernelContextSnapshot,
    pub expected_source: crate::secret::TransferredVaultSourceBinding,
    pub target_kernel_id: String,
    pub target_private_key: String,
    pub capability_root: PathBuf,
    pub vault_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelContextImportReceipt {
    pub schema_version: u32,
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub snapshot_sha256: String,
    pub capability_root: PathBuf,
    pub extension_count: usize,
    pub dependency_count: usize,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelContextSnapshot {
    pub payload: KernelContextPayload,
    pub snapshot_sha256: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelContextPayload {
    pub schema_version: u32,
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub compatibility: KernelContextCompatibility,
    pub extensions: Vec<KernelExtensionSnapshot>,
    pub dependencies: Vec<KernelExtensionDependency>,
    pub vault: crate::secret::TransferredVaultSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelContextCompatibility {
    pub source_kernel_version: String,
    pub local_daemon_protocol_version: u32,
    pub relay_peer_protocol_version: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelExtensionSnapshot {
    pub kind: ExtensionKind,
    pub scope: KernelExtensionScope,
    pub name: String,
    pub definition_sha256: String,
    pub definition: KernelExtensionDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelExtensionScope {
    User,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelExtensionDefinition {
    Mcp {
        config: CharioxMcpServerConfig,
    },
    Skill {
        package: crate::skill::CharioxSkillPackage,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        executable_paths: Vec<String>,
    },
    Script {
        script: KernelScriptSnapshot,
    },
    Connector {
        definition: CharioxConnectorDefinition,
    },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelScriptSnapshot {
    pub runtime: CharioxScriptRuntime,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    pub source_sha256: String,
    pub source_base64: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelExtensionDependency {
    Environment {
        name: String,
        runtime: PortableEnvironmentRuntime,
    },
    UserConnectorAdapter {
        name: String,
        definition_sha256: String,
        files: Vec<KernelPackageFile>,
    },
    BundledConnectorAdapter {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        adapter_protocol: String,
        artifact_sha256: String,
    },
    Credential {
        credential: UserCredentialConfig,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortableEnvironmentRuntime {
    Python {
        version: String,
        files: Vec<KernelPackageFile>,
    },
    Node {
        version: String,
        files: Vec<KernelPackageFile>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PortableEnvironmentKind {
    Python,
    Node,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableEnvironmentManifest {
    schema_version: u32,
    runtime: PortableEnvironmentKind,
    version: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelPackageFile {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub executable: bool,
    pub content_base64: String,
}

impl fmt::Debug for KernelContextSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelContextSnapshot")
            .field("payload", &self.payload)
            .field("snapshot_sha256", &self.snapshot_sha256)
            .finish()
    }
}

impl fmt::Debug for KernelContextImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelContextImportRequest")
            .field("snapshot", &self.snapshot)
            .field("expected_source", &self.expected_source)
            .field("target_kernel_id", &self.target_kernel_id)
            .field("target_private_key", &"[REDACTED]")
            .field("capability_root", &self.capability_root)
            .field("vault_path", &self.vault_path)
            .finish()
    }
}

impl fmt::Debug for KernelContextPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelContextPayload")
            .field("schema_version", &self.schema_version)
            .field("context_id", &self.context_id)
            .field("source_kernel_id", &self.source_kernel_id)
            .field("source_key_thumbprint", &self.source_key_thumbprint)
            .field("target_kernel_id", &self.target_kernel_id)
            .field("compatibility", &self.compatibility)
            .field("extension_count", &self.extensions.len())
            .field("dependency_count", &self.dependencies.len())
            .field("has_vault", &true)
            .finish()
    }
}

impl fmt::Debug for KernelExtensionSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelExtensionSnapshot")
            .field("kind", &self.kind)
            .field("scope", &self.scope)
            .field("name", &self.name)
            .field("definition_sha256", &self.definition_sha256)
            .field("definition", &self.definition)
            .finish()
    }
}

impl fmt::Debug for KernelExtensionDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mcp { config } => formatter
                .debug_struct("Mcp")
                .field("name", &config.name)
                .finish_non_exhaustive(),
            Self::Skill {
                package,
                executable_paths,
            } => formatter
                .debug_struct("Skill")
                .field("name", &package.metadata.name)
                .field("version_hash", &package.version_hash)
                .field("file_count", &package.files.len())
                .field("executable_file_count", &executable_paths.len())
                .finish(),
            Self::Script { script } => formatter
                .debug_struct("Script")
                .field("runtime", &script.runtime)
                .field("source_sha256", &script.source_sha256)
                .finish_non_exhaustive(),
            Self::Connector { definition } => formatter
                .debug_struct("Connector")
                .field("name", &definition.name)
                .field("adapter", &definition.adapter)
                .finish_non_exhaustive(),
        }
    }
}

impl fmt::Debug for KernelScriptSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelScriptSnapshot")
            .field("runtime", &self.runtime)
            .field("source_sha256", &self.source_sha256)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for KernelExtensionDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment { name, runtime } => formatter
                .debug_struct("Environment")
                .field("name", name)
                .field("runtime", runtime)
                .finish(),
            Self::UserConnectorAdapter {
                name,
                definition_sha256,
                files,
            } => formatter
                .debug_struct("UserConnectorAdapter")
                .field("name", name)
                .field("definition_sha256", definition_sha256)
                .field("file_count", &files.len())
                .finish(),
            Self::BundledConnectorAdapter {
                name,
                version,
                adapter_protocol,
                artifact_sha256,
            } => formatter
                .debug_struct("BundledConnectorAdapter")
                .field("name", name)
                .field("version", version)
                .field("adapter_protocol", adapter_protocol)
                .field("artifact_sha256", artifact_sha256)
                .finish(),
            Self::Credential { credential } => formatter
                .debug_struct("Credential")
                .field("id", &credential.id)
                .finish_non_exhaustive(),
        }
    }
}

impl fmt::Debug for PortableEnvironmentRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Python { version, files } => formatter
                .debug_struct("Python")
                .field("version", version)
                .field("file_count", &files.len())
                .finish(),
            Self::Node { version, files } => formatter
                .debug_struct("Node")
                .field("version", version)
                .field("file_count", &files.len())
                .finish(),
        }
    }
}

impl fmt::Debug for KernelPackageFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelPackageFile")
            .field("sha256", &self.sha256)
            .field("size_bytes", &self.size_bytes)
            .field("executable", &self.executable)
            .finish_non_exhaustive()
    }
}
