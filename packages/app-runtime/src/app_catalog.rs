//! Verified App declarations projected into the kernel's existing runtime MCP.
//!
//! A catalog and a schema-valid call are not an authorization grant. The kernel
//! must resolve the authenticated provider run, current binding and operation
//! policy before using this component, through the same path used by App views.

mod invocation;
mod payload;

use chariox_app_package::{ActionDeclaration, Limits, VerifiedPackage};
pub use invocation::{Actor, CallerContext, ValidatedToolCall};
use jsonschema::JSONSchema;
use rusqlite::Transaction;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    installation::{
        ReleaseMetadata, StageTrustBinding, VerifiedInstallCandidate, VerifiedStageError,
    },
    publisher_trust::TrustedPublisherSnapshot,
};

pub const MAX_TOOL_NAME_BYTES: usize = 64;
pub const MAX_CALL_BYTES: usize = 512 * 1024;
pub const MAX_CALL_NODES: usize = 32_768;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("app_catalog_provenance")]
    Provenance,
    #[error("app_catalog_stale")]
    Stale,
    #[error("app_catalog_invalid")]
    Invalid,
    #[error("app_catalog_schema")]
    Schema,
    #[error("app_catalog_name_collision")]
    NameCollision,
    #[error("app_catalog_unknown_tool")]
    UnknownTool,
    #[error("app_catalog_input")]
    Input,
    #[error("app_catalog_output")]
    Output,
    #[error("app_catalog_limit")]
    Limit,
    #[error("app_catalog_deadline")]
    Deadline,
    #[error("app_catalog_response")]
    Response,
    #[error("app_catalog_worker_error")]
    Worker(crate::wire::RemoteError),
    #[error("app_catalog_admission")]
    Admission(#[from] VerifiedStageError),
}
pub type Result<T> = std::result::Result<T, CatalogError>;

/// Trusted descriptive metadata. Only `name`, `description`, and `input_schema`
/// are projected to RuntimeToolSpec. An action is policy input, not permission.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub local_name: String,
    pub app_id: String,
    pub installation_id: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub action: Option<ActionDeclaration>,
}

struct CompiledTool {
    spec: ToolSpec,
    input: JSONSchema,
    output: Option<JSONSchema>,
}

/// Built once from an offline-verified package and its exact enrolled signer.
/// Current installation and signer admission are checked separately on every
/// use; holding this object does not keep a revoked installation running.
pub struct AppCatalog {
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
    release: ReleaseMetadata,
    tools: BTreeMap<String, CompiledTool>,
}

impl AppCatalog {
    pub fn compile(
        package: &VerifiedPackage<'_>,
        binding: &StageTrustBinding,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<Self> {
        let candidate = VerifiedInstallCandidate::from_verified(package, trust)?;
        let signer = trust.publisher();
        if binding.package_digest() != package.package_digest()
            || binding.publisher_id() != signer.publisher_id
            || binding.key_id() != signer.key_id
            || binding.trust_revision() != trust.revision()
            || binding.public_key_fingerprint()
                != format!("sha256:{:x}", Sha256::digest(signer.public_key.as_bytes()))
        {
            return Err(CatalogError::Provenance);
        }
        let limits = Limits::default();
        let declarations = package.declarations();
        if declarations.tools.len() > limits.max_declarations {
            return Err(CatalogError::Limit);
        }
        let mut tools = BTreeMap::new();
        for declaration in &declarations.tools {
            let name = tool_name(&binding.token().installation_id, &declaration.name);
            let action = declaration
                .action
                .as_ref()
                .map(|name| {
                    declarations
                        .actions
                        .iter()
                        .find(|action| action.name == *name)
                        .cloned()
                        .ok_or(CatalogError::Provenance)
                })
                .transpose()?;
            let spec = ToolSpec {
                name: name.clone(),
                local_name: declaration.name.clone(),
                app_id: package.manifest().app_id.clone(),
                installation_id: binding.token().installation_id.clone(),
                description: format!(
                    "{} / {} (installation {}). {}",
                    package.manifest().app_id,
                    declaration.name,
                    binding.token().installation_id,
                    declaration.description
                ),
                input_schema: declaration.input_schema.clone(),
                output_schema: declaration.output_schema.clone(),
                action,
            };
            let input = chariox_app_package::compile_schema(&spec.input_schema, true, &limits)
                .map_err(|_| CatalogError::Schema)?;
            let output = spec
                .output_schema
                .as_ref()
                .map(|schema| {
                    chariox_app_package::compile_schema(schema, false, &limits)
                        .map_err(|_| CatalogError::Schema)
                })
                .transpose()?;
            if tools
                .insert(
                    name,
                    CompiledTool {
                        spec,
                        input,
                        output,
                    },
                )
                .is_some()
            {
                return Err(CatalogError::NameCollision);
            }
        }
        Ok(Self {
            binding: binding.clone(),
            trust: trust.clone(),
            release: candidate.release_metadata().clone(),
            tools,
        })
    }

    /// Use the kernel writer's current transaction, not a cached discovery
    /// snapshot. The worker actor must fence already admitted work on changes.
    pub fn require_current(
        &self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
    ) -> Result<()> {
        let active = self
            .binding
            .require_active(transaction, trusted_owner, &self.trust)?;
        if active.release != self.release {
            return Err(CatalogError::Stale);
        }
        Ok(())
    }

    pub fn installation_id(&self) -> &str {
        &self.binding.token().installation_id
    }
    pub fn generation(&self) -> u64 {
        self.binding.token().generation
    }
    pub fn catalog_digest(&self) -> &str {
        &self.release.catalog_digest
    }

    /// Discovery data only. Call require_current before publication and then
    /// again at invocation: provider catalogs can remain visible after revoke.
    pub fn tools(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.values().map(|tool| &tool.spec)
    }

    pub fn tool(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.get(name).map(|tool| &tool.spec)
    }

    /// Check the complete runtime MCP namespace, including fixed tools, remote
    /// projections and other bound Apps. Hash names are never assumed unique.
    pub fn check_names<'a>(&self, occupied: impl IntoIterator<Item = &'a str>) -> Result<()> {
        let occupied: BTreeSet<_> = occupied.into_iter().collect();
        if self
            .tools
            .keys()
            .any(|name| occupied.contains(name.as_str()))
        {
            return Err(CatalogError::NameCollision);
        }
        Ok(())
    }
}

fn tool_name(installation: &str, local: &str) -> String {
    // Names are transport identifiers, not an encoded authority or an App
    // ontology. Verified local names are ASCII; the full identity is hashed.
    let mut hash = Sha256::new();
    hash.update(b"chariox.app-tool-name.v1\0");
    for part in [installation.as_bytes(), local.as_bytes()] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = format!("{:x}", hash.finalize());
    format!("app_{}_{}", &local[..local.len().min(27)], &digest[..32])
}
