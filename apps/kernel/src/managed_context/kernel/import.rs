use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{validate_credentials, UserCredentialSourceConfig};
use crate::connector::{
    CharioxConnectorAdapterDefinition, CharioxConnectorAdapterRegistry, ConnectorAdapterSource,
};
use crate::error::DaemonError;
use crate::extension::ExtensionKind;
use crate::mcp::CharioxMcpTransportConfig;
use crate::script::{CharioxEnvironmentConfig, CharioxEnvironmentRuntime, CharioxScriptRuntime};

use super::export::{
    bundled_adapter_artifact_hash, decoded_package_file, extension_definition_hash,
    package_definition_hash, reject_literal_secrets_in_json, serialized_json_measure,
    validate_identifier, validate_node_package_files, validate_portable_credential_injection,
    validate_portable_mcp, validate_portable_user_adapter, validate_python_requirements_lock,
    validate_safe_package_file, validate_sha256, MAX_DEPENDENCIES, MAX_EXTENSIONS, MAX_FILE_BYTES,
    MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_FILES,
};
use super::{
    KernelContextImportReceipt, KernelContextImportRequest, KernelContextSnapshot,
    KernelExtensionDefinition, KernelExtensionDependency, KernelExtensionScope, KernelPackageFile,
    PortableEnvironmentKind, PortableEnvironmentManifest, PortableEnvironmentRuntime,
    KERNEL_CONTEXT_SCHEMA_VERSION,
};

const IMPORT_RECEIPT_NAME: &str = "kernel-context-import.json";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RUNTIME_PROBE_BYTES: u64 = 64 * 1024;
const MAX_RUNTIME_PROBE_ENTRIES: u64 = 32;
const MAX_MATERIALIZED_CONTEXT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_MATERIALIZED_CONTEXT_ENTRIES: u64 = 100_000;
static RUNTIME_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredScriptMetadata {
    name: String,
    runtime: CharioxScriptRuntime,
    entrypoint: String,
    description: String,
    input_schema: serde_json::Value,
    definition_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
}

struct ImportLock {
    file: File,
}

impl Drop for ImportLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

struct MaterializationBudget {
    bytes: u64,
    entries: u64,
}

impl MaterializationBudget {
    fn new() -> Self {
        Self {
            bytes: 0,
            entries: 0,
        }
    }

    fn file(&mut self, size: u64) -> Result<(), DaemonError> {
        self.bytes = self.bytes.saturating_add(size).saturating_add(4096);
        self.entries = self.entries.saturating_add(1);
        self.ensure_available()
    }

    fn directory(&mut self) -> Result<(), DaemonError> {
        self.bytes = self.bytes.saturating_add(4096);
        self.entries = self.entries.saturating_add(1);
        self.ensure_available()
    }

    fn ensure_available(&self) -> Result<(), DaemonError> {
        if self.bytes > MAX_MATERIALIZED_CONTEXT_BYTES
            || self.entries > MAX_MATERIALIZED_CONTEXT_ENTRIES
        {
            return Err(import_error(
                "kernel context materialization exceeds its resource limits",
            ));
        }
        Ok(())
    }
}

pub fn import_kernel_context(
    request: KernelContextImportRequest,
) -> Result<KernelContextImportReceipt, DaemonError> {
    let target_public_key = crate::transport::relay_crypto::public_key_from_private_key_base64(
        &request.target_private_key,
    )?;
    let target_key_thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(&target_public_key);
    validate_snapshot(
        &request.snapshot,
        &request.expected_source,
        &request.target_kernel_id,
        &target_key_thumbprint,
    )?;
    validate_import_paths(&request.capability_root, &request.vault_path)?;

    let parent = request
        .capability_root
        .parent()
        .ok_or_else(|| import_error("kernel context capability root must have a private parent"))?;
    ensure_private_directory(parent)?;
    let _lock = acquire_import_lock(parent)?;
    let receipt = import_receipt(&request.snapshot, request.capability_root.clone());
    if request.capability_root.exists() {
        return verify_existing_import(&request, &receipt);
    }

    let staging = staging_path(parent, &request.snapshot.snapshot_sha256);
    cleanup_staging(&staging)?;
    let result = (|| {
        ensure_private_directory(&staging)?;
        let mut budget = MaterializationBudget::new();
        budget.directory()?;
        materialize_snapshot(
            &request.snapshot,
            &staging,
            &request.capability_root,
            &mut budget,
        )?;
        write_json_file(
            &staging.join(IMPORT_RECEIPT_NAME),
            &receipt,
            false,
            &mut budget,
        )?;
        ensure_tree_within_budget(&staging)?;
        sync_private_tree(&staging)?;

        crate::secret::install_transferred_vault_snapshot(
            &request.vault_path,
            &request.snapshot.payload.vault,
            &request.expected_source,
            &request.target_kernel_id,
            &request.target_private_key,
        )?;
        match publish_directory_no_clobber(&staging, &request.capability_root) {
            Ok(()) => sync_directory(parent)?,
            Err(_) if request.capability_root.exists() => {
                return verify_existing_import(&request, &receipt)
            }
            Err(error) => return Err(error),
        }
        Ok(receipt.clone())
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

pub(crate) fn cleanup_kernel_context_import(
    receipt: &KernelContextImportReceipt,
    vault_path: &Path,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    validate_import_paths(&receipt.capability_root, vault_path)?;
    let target_public_key =
        crate::transport::relay_crypto::public_key_from_private_key_base64(target_private_key)?;
    if receipt.target_key_thumbprint
        != crate::runtime::terminal_pairings::public_key_thumbprint(&target_public_key)
    {
        return Err(import_error(
            "kernel context rollback target key does not match the receipt",
        ));
    }
    let parent = receipt
        .capability_root
        .parent()
        .ok_or_else(|| import_error("kernel context capability root has no parent"))?;
    ensure_private_directory(parent)?;
    let _lock = acquire_import_lock(parent)?;
    match fs::symlink_metadata(&receipt.capability_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(import_error(
                    "kernel context rollback target is not a private directory",
                ));
            }
            verify_published_receipt(&receipt.capability_root, receipt)?;
            fs::remove_dir_all(&receipt.capability_root)
                .map_err(|error| import_io_error("remove failed kernel context", error))?;
            sync_directory(parent)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(import_io_error(
                "inspect failed kernel context rollback target",
                error,
            ))
        }
    }
    crate::secret::remove_installed_transferred_vault(
        vault_path,
        &crate::secret::TransferredVaultSourceBinding {
            context_id: receipt.context_id.clone(),
            source_kernel_id: receipt.source_kernel_id.clone(),
            source_key_thumbprint: receipt.source_key_thumbprint.clone(),
        },
        &receipt.target_kernel_id,
        target_private_key,
    )
}

fn verify_existing_import(
    request: &KernelContextImportRequest,
    expected_receipt: &KernelContextImportReceipt,
) -> Result<KernelContextImportReceipt, DaemonError> {
    let receipt = verify_published_receipt(&request.capability_root, expected_receipt)?;
    crate::secret::validate_installed_transferred_vault(
        &request.vault_path,
        &request.expected_source,
        &request.target_kernel_id,
        &request.target_private_key,
    )?;
    Ok(receipt)
}

fn validate_snapshot(
    snapshot: &KernelContextSnapshot,
    expected_source: &crate::secret::TransferredVaultSourceBinding,
    target_kernel_id: &str,
    target_key_thumbprint: &str,
) -> Result<(), DaemonError> {
    let payload = &snapshot.payload;
    if payload.schema_version != KERNEL_CONTEXT_SCHEMA_VERSION {
        return Err(import_error("unsupported kernel context schema version"));
    }
    validate_identifier(&payload.context_id, "context id")?;
    validate_identifier(&payload.source_kernel_id, "source kernel id")?;
    validate_sha256(&payload.source_key_thumbprint, "source key thumbprint")?;
    validate_identifier(&payload.target_kernel_id, "target kernel id")?;
    validate_sha256(&payload.target_key_thumbprint, "target key thumbprint")?;
    validate_sha256(&snapshot.snapshot_sha256, "snapshot digest")?;
    let (_, actual_snapshot_sha256) = serialized_json_measure(payload, MAX_SNAPSHOT_BYTES)?;
    if actual_snapshot_sha256 != snapshot.snapshot_sha256 {
        return Err(import_error("kernel context payload digest does not match"));
    }
    if payload.context_id != expected_source.context_id
        || payload.source_kernel_id != expected_source.source_kernel_id
        || payload.source_key_thumbprint != expected_source.source_key_thumbprint
    {
        return Err(import_error(
            "kernel context source or context binding does not match",
        ));
    }
    if payload.target_kernel_id != target_kernel_id
        || payload.target_key_thumbprint != target_key_thumbprint
    {
        return Err(import_error("kernel context target binding does not match"));
    }
    if payload.compatibility.local_daemon_protocol_version
        > crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        || payload.compatibility.relay_peer_protocol_version
            > crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    {
        return Err(import_error(
            "kernel context was exported by a newer incompatible kernel",
        ));
    }
    crate::secret::validate_transferred_vault_snapshot_for_export(
        &payload.vault,
        expected_source,
        target_kernel_id,
        target_key_thumbprint,
    )?;
    validate_total_snapshot_file_count(snapshot)?;
    validate_extensions(snapshot)?;
    validate_dependencies(snapshot)?;
    validate_extension_dependencies(snapshot)
}

fn validate_total_snapshot_file_count(snapshot: &KernelContextSnapshot) -> Result<(), DaemonError> {
    let extension_files = snapshot
        .payload
        .extensions
        .iter()
        .map(|extension| match &extension.definition {
            KernelExtensionDefinition::Skill { package, .. } => package.files.len(),
            KernelExtensionDefinition::Script { .. } => 1,
            KernelExtensionDefinition::Mcp { runtime, .. } => {
                runtime.as_ref().map_or(0, |runtime| runtime.files.len())
            }
            KernelExtensionDefinition::Connector { .. } => 0,
        })
        .sum::<usize>();
    let dependency_files = snapshot
        .payload
        .dependencies
        .iter()
        .map(|dependency| match dependency {
            KernelExtensionDependency::Environment {
                runtime:
                    PortableEnvironmentRuntime::Python { files, .. }
                    | PortableEnvironmentRuntime::Node { files, .. },
                ..
            }
            | KernelExtensionDependency::UserConnectorAdapter { files, .. } => files.len(),
            KernelExtensionDependency::BundledConnectorAdapter { .. }
            | KernelExtensionDependency::Credential { .. } => 0,
        })
        .sum::<usize>();
    if 1_usize
        .saturating_add(extension_files)
        .saturating_add(dependency_files)
        > MAX_SNAPSHOT_FILES
    {
        return Err(import_error(
            "kernel context snapshot file count exceeds its limit",
        ));
    }
    Ok(())
}

fn validate_extensions(snapshot: &KernelContextSnapshot) -> Result<(), DaemonError> {
    if snapshot.payload.extensions.len() > MAX_EXTENSIONS {
        return Err(import_error("kernel Extension count exceeds its limit"));
    }
    let mut identities = BTreeSet::new();
    let mut file_count = 1_usize;
    for extension in &snapshot.payload.extensions {
        if extension.scope != KernelExtensionScope::User
            || !identities.insert((extension.kind.clone(), extension.name.clone()))
        {
            return Err(import_error(
                "kernel context contains a duplicate or unsupported Extension",
            ));
        }
        crate::mcp::validate_registry_name(&extension.name, "Extension name")?;
        validate_sha256(&extension.definition_sha256, "Extension definition digest")?;
        if extension_definition_hash(&extension.definition)? != extension.definition_sha256 {
            return Err(import_error(format!(
                "kernel Extension `{}` definition digest does not match",
                extension.name
            )));
        }
        match (&extension.kind, &extension.definition) {
            (ExtensionKind::Mcp, KernelExtensionDefinition::Mcp { config, runtime }) => {
                if config.name != extension.name {
                    return Err(import_error("MCP Extension name does not match"));
                }
                config.validate()?;
                validate_portable_mcp(config, runtime.as_ref())?;
                if let Some(runtime) = runtime {
                    validate_package_files(&runtime.files)?;
                    file_count = file_count.saturating_add(runtime.files.len());
                }
            }
            (
                ExtensionKind::Skill,
                KernelExtensionDefinition::Skill {
                    package,
                    executable_paths,
                },
            ) => {
                if package.metadata.name != extension.name {
                    return Err(import_error("skill Extension name does not match"));
                }
                file_count = file_count.saturating_add(package.files.len());
                validate_skill_package(package, executable_paths)?;
            }
            (ExtensionKind::Script, KernelExtensionDefinition::Script { script }) => {
                file_count = file_count.saturating_add(1);
                validate_script_snapshot(&extension.name, script)?;
            }
            (ExtensionKind::Connector, KernelExtensionDefinition::Connector { definition }) => {
                if definition.name != extension.name {
                    return Err(import_error("connector Extension name does not match"));
                }
                definition.validate()?;
                for operation in &definition.operations {
                    reject_literal_secrets_in_json(&operation.config)?;
                }
            }
            _ => {
                return Err(import_error(
                    "kernel Extension kind does not match its definition",
                ))
            }
        }
    }
    if file_count > MAX_SNAPSHOT_FILES {
        return Err(import_error(
            "kernel context snapshot file count exceeds its limit",
        ));
    }
    Ok(())
}

fn validate_dependencies(snapshot: &KernelContextSnapshot) -> Result<(), DaemonError> {
    if snapshot.payload.dependencies.len() > MAX_DEPENDENCIES {
        return Err(import_error(
            "kernel Extension dependency count exceeds its limit",
        ));
    }
    let mut identities = BTreeSet::new();
    let mut file_count = 1_usize;
    for dependency in &snapshot.payload.dependencies {
        let identity = dependency_identity(dependency);
        if !identities.insert(identity) {
            return Err(import_error(
                "kernel context contains duplicate Extension dependencies",
            ));
        }
        match dependency {
            KernelExtensionDependency::Environment { name, runtime } => {
                crate::mcp::validate_registry_name(name, "environment name")?;
                let files = match runtime {
                    PortableEnvironmentRuntime::Python { version, files } => {
                        validate_runtime_version(version)?;
                        validate_environment_manifest(
                            name,
                            version,
                            PortableEnvironmentKind::Python,
                            files,
                            &["manifest.json", "requirements.lock"],
                        )?;
                        validate_python_requirements_lock(files)?;
                        files
                    }
                    PortableEnvironmentRuntime::Node { version, files } => {
                        validate_runtime_version(version)?;
                        validate_environment_manifest(
                            name,
                            version,
                            PortableEnvironmentKind::Node,
                            files,
                            &["manifest.json", "package.json", "package-lock.json"],
                        )?;
                        validate_node_package_files(files)?;
                        files
                    }
                };
                validate_package_files(files)?;
                file_count = file_count.saturating_add(files.len());
            }
            KernelExtensionDependency::UserConnectorAdapter {
                name,
                definition_sha256,
                files,
            } => {
                crate::mcp::validate_registry_name(name, "connector adapter name")?;
                validate_sha256(definition_sha256, "connector adapter package digest")?;
                validate_package_files(files)?;
                if package_definition_hash(files) != *definition_sha256 {
                    return Err(import_error(format!(
                        "connector adapter `{name}` package digest does not match"
                    )));
                }
                let manifest = decoded_package_file(files, "adapter.yaml")?;
                let mut adapter =
                    serde_yaml::from_slice::<CharioxConnectorAdapterDefinition>(&manifest)
                        .map_err(|_| import_error("connector adapter manifest is invalid"))?;
                adapter.source = Some(ConnectorAdapterSource::User);
                adapter.manifest_path = Some(PathBuf::from("adapter.yaml"));
                if adapter.name != *name {
                    return Err(import_error("connector adapter name does not match"));
                }
                adapter.validate()?;
                validate_portable_user_adapter(&adapter)?;
                file_count = file_count.saturating_add(files.len());
            }
            KernelExtensionDependency::BundledConnectorAdapter {
                name,
                version,
                adapter_protocol,
                artifact_sha256,
            } => {
                crate::mcp::validate_registry_name(name, "bundled connector adapter name")?;
                validate_sha256(artifact_sha256, "bundled connector adapter digest")?;
                let installed = CharioxConnectorAdapterRegistry::user()?
                    .get(name)?
                    .ok_or_else(|| {
                        import_error(format!(
                            "bundled connector adapter `{name}` is not installed in the target image"
                        ))
                    })?;
                if installed.source != Some(ConnectorAdapterSource::Bundled)
                    || installed.version != *version
                    || installed.adapter_protocol != *adapter_protocol
                    || bundled_adapter_artifact_hash(&installed)? != *artifact_sha256
                {
                    return Err(import_error(format!(
                        "bundled connector adapter `{name}` does not match the target image"
                    )));
                }
            }
            KernelExtensionDependency::Credential { credential } => {
                validate_credentials(std::slice::from_ref(credential))?;
                if !matches!(credential.source, UserCredentialSourceConfig::Vault { .. }) {
                    return Err(import_error(format!(
                        "credential `{}` is not Vault-backed",
                        credential.id
                    )));
                }
                validate_portable_credential_injection(credential)?;
            }
        }
    }
    if file_count > MAX_SNAPSHOT_FILES {
        return Err(import_error(
            "kernel context snapshot file count exceeds its limit",
        ));
    }
    Ok(())
}

fn validate_extension_dependencies(snapshot: &KernelContextSnapshot) -> Result<(), DaemonError> {
    let adapters = snapshot
        .payload
        .dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            KernelExtensionDependency::UserConnectorAdapter { name, .. }
            | KernelExtensionDependency::BundledConnectorAdapter { name, .. } => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let credentials = snapshot
        .payload
        .dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            KernelExtensionDependency::Credential { credential } => Some(credential.id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for extension in &snapshot.payload.extensions {
        match &extension.definition {
            KernelExtensionDefinition::Connector { definition }
                if !adapters.contains(definition.adapter.as_str()) =>
            {
                return Err(import_error(format!(
                    "connector `{}` has no transferred or image-bound adapter",
                    definition.name
                )));
            }
            KernelExtensionDefinition::Mcp { config, .. } => {
                for credential in mcp_credential_bindings(config) {
                    if !credentials.contains(credential) {
                        return Err(import_error(format!(
                            "MCP `{}` references missing credential `{credential}`",
                            config.name
                        )));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn materialize_snapshot(
    snapshot: &KernelContextSnapshot,
    staging: &Path,
    final_root: &Path,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let user_root = staging.join("user");
    ensure_budgeted_directory(&user_root, budget)?;
    materialize_dependencies(snapshot, staging, final_root, budget)?;
    for extension in &snapshot.payload.extensions {
        match &extension.definition {
            KernelExtensionDefinition::Mcp { config, runtime } => {
                let root = user_root.join("mcps");
                ensure_budgeted_directory(&root, budget)?;
                let mut materialized = config.clone();
                if let Some(runtime) = runtime {
                    let staged_runtime_root = root.join(&extension.name);
                    let final_runtime_root =
                        final_root.join("user").join("mcps").join(&extension.name);
                    ensure_budgeted_directory(&staged_runtime_root, budget)?;
                    for file in &runtime.files {
                        let bytes = decode_kernel_package_file(file)?;
                        write_package_file(
                            &staged_runtime_root,
                            &file.path,
                            &bytes,
                            file.executable,
                            budget,
                        )?;
                    }
                    let CharioxMcpTransportConfig::Stdio { command, cwd, .. } =
                        &mut materialized.transport
                    else {
                        return Err(import_error(
                            "stdio MCP runtime is attached to a non-stdio definition",
                        ));
                    };
                    *command = final_runtime_root
                        .join(&runtime.command_path)
                        .to_str()
                        .ok_or_else(|| import_error("materialized MCP command is not UTF-8"))?
                        .to_string();
                    *cwd = Some(match runtime.cwd_path.as_deref() {
                        Some(relative) => final_runtime_root.join(relative),
                        None => final_runtime_root,
                    });
                }
                write_json_file(
                    &root.join(format!("{}.json", extension.name)),
                    &materialized,
                    false,
                    budget,
                )?;
            }
            KernelExtensionDefinition::Skill {
                package,
                executable_paths,
            } => {
                let root = user_root.join("skills").join(&extension.name);
                ensure_budgeted_directory(&root, budget)?;
                let executable = executable_paths
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                for file in &package.files {
                    let bytes = decode_package_content(
                        &file.path,
                        &file.content_base64,
                        None,
                        &file.sha256,
                    )?;
                    write_package_file(
                        &root,
                        &file.path,
                        &bytes,
                        executable.contains(file.path.as_str()),
                        budget,
                    )?;
                }
            }
            KernelExtensionDefinition::Script { script } => {
                let root = user_root.join("scripts").join(&extension.name);
                ensure_budgeted_directory(&root, budget)?;
                let entrypoint = match script.runtime {
                    CharioxScriptRuntime::Python => "script.py",
                    CharioxScriptRuntime::TypeScript => "script.ts",
                };
                let source = decode_package_content(
                    entrypoint,
                    &script.source_base64,
                    None,
                    &script.source_sha256,
                )?;
                write_bytes_file(&root.join(entrypoint), &source, false, budget)?;
                let metadata = StoredScriptMetadata {
                    name: extension.name.clone(),
                    runtime: script.runtime.clone(),
                    entrypoint: entrypoint.to_string(),
                    description: script.description.clone(),
                    input_schema: script.input_schema.clone(),
                    definition_hash: script_registry_definition_hash(
                        &source,
                        &script.description,
                        &script.input_schema,
                    ),
                    timeout_sec: script.timeout_sec,
                };
                write_json_file(&root.join("metadata.json"), &metadata, false, budget)?;
            }
            KernelExtensionDefinition::Connector { definition } => {
                let root = user_root.join("connectors").join("definitions");
                ensure_budgeted_directory(&root, budget)?;
                write_yaml_file(
                    &root.join(format!("{}.yaml", extension.name)),
                    definition,
                    budget,
                )?;
            }
        }
    }
    Ok(())
}

fn materialize_dependencies(
    snapshot: &KernelContextSnapshot,
    staging: &Path,
    final_root: &Path,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let user_root = staging.join("user");
    for dependency in &snapshot.payload.dependencies {
        match dependency {
            KernelExtensionDependency::Environment { name, runtime } => {
                materialize_environment(name, runtime, staging, final_root, budget)?;
            }
            KernelExtensionDependency::UserConnectorAdapter { name, files, .. } => {
                let root = user_root.join("connectors").join("adapters").join(name);
                ensure_budgeted_directory(&root, budget)?;
                for file in files {
                    let bytes = decode_kernel_package_file(file)?;
                    write_package_file(&root, &file.path, &bytes, file.executable, budget)?;
                }
            }
            KernelExtensionDependency::BundledConnectorAdapter { .. } => {}
            KernelExtensionDependency::Credential { credential } => {
                let root = user_root.join("credentials");
                ensure_budgeted_directory(&root, budget)?;
                write_yaml_file(
                    &root.join(format!("{}.yaml", credential.id)),
                    credential,
                    budget,
                )?;
            }
        }
    }
    Ok(())
}

fn materialize_environment(
    name: &str,
    runtime: &PortableEnvironmentRuntime,
    staging: &Path,
    final_root: &Path,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let staged_package_root = staging
        .join("user")
        .join("envs")
        .join(".portable")
        .join(name);
    let final_package_root = final_root
        .join("user")
        .join("envs")
        .join(".portable")
        .join(name);
    ensure_budgeted_directory(&staged_package_root, budget)?;
    let files = match runtime {
        PortableEnvironmentRuntime::Python { files, .. }
        | PortableEnvironmentRuntime::Node { files, .. } => files,
    };
    for file in files {
        let bytes = decode_kernel_package_file(file)?;
        write_package_file(
            &staged_package_root,
            &file.path,
            &bytes,
            file.executable,
            budget,
        )?;
    }
    let config = match runtime {
        PortableEnvironmentRuntime::Python { version, .. } => {
            let system_python = find_runtime(&["python3", "python"], version, "Python", staging)?;
            let requirements = fs::read_to_string(staged_package_root.join("requirements.lock"))
                .map_err(|error| import_io_error("read Python requirements lock", error))?;
            if requirements_has_packages(&requirements) {
                let staged_venv = staged_package_root.join("venv");
                run_install_command(
                    Command::new(&system_python)
                        .arg("-m")
                        .arg("venv")
                        .arg("--copies")
                        .arg(&staged_venv),
                    "create Python virtual environment",
                    staging,
                )?;
                let staged_python = python_venv_binary(&staged_venv);
                run_install_command(
                    Command::new(&staged_python)
                        .arg("-m")
                        .arg("pip")
                        .arg("install")
                        .arg("--only-binary=:all:")
                        .arg("--require-hashes")
                        .arg("--no-deps")
                        .arg("--no-cache-dir")
                        .arg("--no-input")
                        .arg("--disable-pip-version-check")
                        .arg("-r")
                        .arg(staged_package_root.join("requirements.lock")),
                    "install Python environment",
                    staging,
                )?;
                remove_python_venv_compatibility_symlink(staging, &staged_venv)?;
                CharioxEnvironmentConfig {
                    name: name.to_string(),
                    runtime: CharioxEnvironmentRuntime::Python {
                        python: python_venv_binary(&final_package_root.join("venv")),
                    },
                }
            } else {
                CharioxEnvironmentConfig {
                    name: name.to_string(),
                    runtime: CharioxEnvironmentRuntime::Python {
                        python: system_python,
                    },
                }
            }
        }
        PortableEnvironmentRuntime::Node { version, .. } => {
            let node = find_runtime(&["node"], version, "Node", staging)?;
            let lock = decoded_package_file(files, "package-lock.json")?;
            if node_lock_has_packages(&lock)? {
                let npm = find_sibling_or_path_runtime(&node, "npm")?;
                let mut command = Command::new(npm);
                command
                    .current_dir(&staged_package_root)
                    .arg("ci")
                    .arg("--ignore-scripts")
                    .arg("--bin-links=false")
                    .arg("--no-audit")
                    .arg("--no-fund");
                run_install_command(&mut command, "install Node environment", staging)?;
            }
            CharioxEnvironmentConfig {
                name: name.to_string(),
                runtime: CharioxEnvironmentRuntime::Node {
                    node,
                    package_root: Some(final_package_root),
                },
            }
        }
    };
    ensure_tree_within_budget(staging)?;
    let definitions = staging.join("user").join("envs");
    ensure_budgeted_directory(&definitions, budget)?;
    write_json_file(
        &definitions.join(format!("{name}.json")),
        &config,
        false,
        budget,
    )
}

fn validate_skill_package(
    package: &crate::skill::CharioxSkillPackage,
    executable_paths: &[String],
) -> Result<(), DaemonError> {
    if package.files.is_empty() || package.files.len() > MAX_PACKAGE_FILES {
        return Err(import_error("skill package file count is invalid"));
    }
    let mut paths = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut version_hasher = Sha256::new();
    let mut total_bytes = 0_u64;
    for file in &package.files {
        let bytes = decode_package_content(&file.path, &file.content_base64, None, &file.sha256)?;
        validate_safe_package_file(&file.path, &bytes)?;
        validate_portable_package_path(&file.path)?;
        let alias = crate::managed_context::portable_path::portable_path_alias_key(&file.path)
            .ok_or_else(|| import_error("skill package path is not portable"))?;
        if !paths.insert(file.path.as_str()) || !aliases.insert(alias) {
            return Err(import_error("skill package contains duplicate paths"));
        }
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_PACKAGE_BYTES {
            return Err(import_error("skill package exceeds its byte limit"));
        }
        version_hasher.update(file.path.as_bytes());
        version_hasher.update([0]);
        version_hasher.update(file.sha256.as_bytes());
        version_hasher.update([0]);
    }
    let version_hash = format!("{:x}", version_hasher.finalize());
    if version_hash != package.version_hash || !paths.contains("SKILL.md") {
        return Err(import_error("skill package version digest is invalid"));
    }
    let mut executable = BTreeSet::new();
    for path in executable_paths {
        validate_portable_package_path(path)?;
        if !paths.contains(path.as_str()) || !executable.insert(path.as_str()) {
            return Err(import_error("skill executable path is invalid"));
        }
    }
    Ok(())
}

fn validate_script_snapshot(
    name: &str,
    script: &super::KernelScriptSnapshot,
) -> Result<(), DaemonError> {
    if script.description.trim().is_empty() {
        return Err(import_error(format!(
            "script `{name}` has no portable description"
        )));
    }
    jsonschema::JSONSchema::compile(&script.input_schema)
        .map_err(|_| import_error(format!("script `{name}` input schema is invalid")))?;
    let bytes = decode_package_content(name, &script.source_base64, None, &script.source_sha256)?;
    let extension = match script.runtime {
        CharioxScriptRuntime::Python => "py",
        CharioxScriptRuntime::TypeScript => "ts",
    };
    if bytes.len() as u64 > MAX_FILE_BYTES || extension.is_empty() {
        return Err(import_error(format!("script `{name}` source is invalid")));
    }
    Ok(())
}

fn validate_package_files(files: &[KernelPackageFile]) -> Result<(), DaemonError> {
    if files.is_empty() || files.len() > MAX_PACKAGE_FILES {
        return Err(import_error("kernel package file count is invalid"));
    }
    let mut aliases = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for file in files {
        validate_portable_package_path(&file.path)?;
        let alias = crate::managed_context::portable_path::portable_path_alias_key(&file.path)
            .ok_or_else(|| import_error("kernel package path is not portable"))?;
        if !aliases.insert(alias) {
            return Err(import_error("kernel package contains duplicate paths"));
        }
        let bytes = decode_kernel_package_file(file)?;
        validate_safe_package_file(&file.path, &bytes)?;
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_PACKAGE_BYTES {
            return Err(import_error("kernel package exceeds its byte limit"));
        }
    }
    Ok(())
}

fn validate_environment_manifest(
    name: &str,
    version: &str,
    expected_runtime: PortableEnvironmentKind,
    files: &[KernelPackageFile],
    expected_paths: &[&str],
) -> Result<(), DaemonError> {
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let expected = expected_paths.iter().copied().collect::<BTreeSet<_>>();
    if paths != expected {
        return Err(import_error(format!(
            "environment `{name}` contains an unexpected portable file set"
        )));
    }
    let manifest = serde_json::from_slice::<PortableEnvironmentManifest>(&decoded_package_file(
        files,
        "manifest.json",
    )?)
    .map_err(|_| import_error(format!("environment `{name}` manifest is invalid")))?;
    if manifest.schema_version != 1
        || manifest.runtime != expected_runtime
        || manifest.version != version
    {
        return Err(import_error(format!(
            "environment `{name}` manifest does not match its dependency"
        )));
    }
    Ok(())
}

fn dependency_identity(dependency: &KernelExtensionDependency) -> (u8, &str) {
    match dependency {
        KernelExtensionDependency::Environment { name, .. } => (0, name),
        KernelExtensionDependency::UserConnectorAdapter { name, .. } => (1, name),
        KernelExtensionDependency::BundledConnectorAdapter { name, .. } => (1, name),
        KernelExtensionDependency::Credential { credential } => (2, &credential.id),
    }
}

fn mcp_credential_bindings(config: &crate::mcp::CharioxMcpServerConfig) -> Vec<&str> {
    match &config.transport {
        CharioxMcpTransportConfig::Stdio { credential_env, .. } => credential_env
            .values()
            .map(|binding| binding.credential.as_str())
            .collect(),
        CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_credential,
            credential_http_headers,
            ..
        } => bearer_token_credential
            .iter()
            .map(String::as_str)
            .chain(
                credential_http_headers
                    .values()
                    .map(|binding| binding.credential.as_str()),
            )
            .collect(),
    }
}

fn import_receipt(
    snapshot: &KernelContextSnapshot,
    capability_root: PathBuf,
) -> KernelContextImportReceipt {
    KernelContextImportReceipt {
        schema_version: 1,
        context_id: snapshot.payload.context_id.clone(),
        source_kernel_id: snapshot.payload.source_kernel_id.clone(),
        source_key_thumbprint: snapshot.payload.source_key_thumbprint.clone(),
        target_kernel_id: snapshot.payload.target_kernel_id.clone(),
        target_key_thumbprint: snapshot.payload.target_key_thumbprint.clone(),
        snapshot_sha256: snapshot.snapshot_sha256.clone(),
        capability_root,
        extension_count: snapshot.payload.extensions.len(),
        dependency_count: snapshot.payload.dependencies.len(),
    }
}

fn verify_published_receipt(
    root: &Path,
    expected: &KernelContextImportReceipt,
) -> Result<KernelContextImportReceipt, DaemonError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| import_io_error("inspect published kernel context", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(import_error(
            "published kernel context root is not a private directory",
        ));
    }
    let bytes = read_bounded_file(&root.join(IMPORT_RECEIPT_NAME), 64 * 1024)?;
    let receipt = serde_json::from_slice::<KernelContextImportReceipt>(&bytes)
        .map_err(|_| import_error("published kernel context receipt is invalid"))?;
    if &receipt != expected {
        return Err(import_error(
            "refusing to replace a different published kernel context",
        ));
    }
    Ok(receipt)
}

fn validate_import_paths(capability_root: &Path, vault_path: &Path) -> Result<(), DaemonError> {
    if !capability_root.is_absolute() || !vault_path.is_absolute() {
        return Err(import_error(
            "kernel context destinations must be absolute paths",
        ));
    }
    if capability_root == vault_path
        || capability_root.starts_with(vault_path)
        || vault_path.starts_with(capability_root)
    {
        return Err(import_error(
            "kernel context capability and Vault destinations must be separate",
        ));
    }
    let configured_capability_root = configured_absolute_path(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        "managed capability isolation root",
    )?;
    let configured_vault_path =
        configured_absolute_path("CHARIOX_MANAGED_VAULT_PATH", "managed Vault path")?;
    if capability_root != configured_capability_root || vault_path != configured_vault_path {
        return Err(import_error(
            "kernel context destinations do not match the running managed kernel configuration",
        ));
    }
    Ok(())
}

fn configured_absolute_path(name: &str, label: &str) -> Result<PathBuf, DaemonError> {
    let path = std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| import_error(format!("{label} is not configured")))?;
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(import_error(format!(
            "{label} is not an absolute normalized path"
        )));
    }
    Ok(path)
}

pub(crate) fn configured_managed_kernel_context_paths() -> Result<(PathBuf, PathBuf), DaemonError> {
    Ok((
        configured_absolute_path(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            "managed capability isolation root",
        )?,
        configured_absolute_path("CHARIOX_MANAGED_VAULT_PATH", "managed Vault path")?,
    ))
}

fn acquire_import_lock(parent: &Path) -> Result<ImportLock, DaemonError> {
    let path = parent.join(".kernel-context-import.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| import_io_error("open kernel context import lock", error))?;
    file.lock_exclusive()
        .map_err(|error| import_io_error("lock kernel context import", error))?;
    Ok(ImportLock { file })
}

fn staging_path(parent: &Path, snapshot_sha256: &str) -> PathBuf {
    parent.join(format!(
        ".kernel-context-{}.staging",
        &snapshot_sha256[..16]
    ))
}

fn cleanup_staging(path: &Path) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(import_io_error("inspect kernel context staging", error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(import_error(
            "kernel context staging path is not a private directory",
        ));
    }
    fs::remove_dir_all(path)
        .map_err(|error| import_io_error("remove stale kernel context staging", error))
}

fn ensure_budgeted_directory(
    path: &Path,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    if path.exists() {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor
            .parent()
            .ok_or_else(|| import_error("kernel context directory has no parent"))?;
    }
    for directory in missing.iter().rev() {
        budget.directory()?;
        fs::create_dir(directory)
            .map_err(|error| import_io_error("create kernel context directory", error))?;
        set_private_directory_permissions(directory)?;
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path)
        .map_err(|error| import_io_error("create private kernel context directory", error))?;
    set_private_directory_permissions(path)
}

fn write_json_file(
    path: &Path,
    value: &impl Serialize,
    executable: bool,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| import_error("serialize kernel context JSON"))?;
    bytes.push(b'\n');
    write_bytes_file(path, &bytes, executable, budget)
}

fn write_yaml_file(
    path: &Path,
    value: &impl Serialize,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let bytes = serde_yaml::to_string(value)
        .map_err(|_| import_error("serialize kernel context YAML"))?
        .into_bytes();
    write_bytes_file(path, &bytes, false, budget)
}

fn write_package_file(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    executable: bool,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    validate_portable_package_path(relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        ensure_budgeted_directory(parent, budget)?;
    }
    write_bytes_file(&path, bytes, executable, budget)
}

fn write_bytes_file(
    path: &Path,
    bytes: &[u8],
    executable: bool,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    if bytes.len() as u64 > MAX_FILE_BYTES || path.exists() {
        return Err(import_error("kernel context file is invalid or duplicated"));
    }
    budget.file(bytes.len() as u64)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    let mut file = options
        .open(path)
        .map_err(|error| import_io_error("create kernel context file", error))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| import_io_error("write kernel context file", error))?;
    set_private_file_permissions(path, executable)
}

fn validate_portable_package_path(path: &str) -> Result<(), DaemonError> {
    if !crate::managed_context::portable_path::is_portable_relative_path(path) {
        return Err(import_error(format!(
            "kernel package path `{path}` is not portable"
        )));
    }
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(import_error(format!(
            "kernel package path `{path}` is not contained"
        )));
    }
    Ok(())
}

fn decode_kernel_package_file(file: &KernelPackageFile) -> Result<Vec<u8>, DaemonError> {
    decode_package_content(
        &file.path,
        &file.content_base64,
        Some(file.size_bytes),
        &file.sha256,
    )
}

fn decode_package_content(
    path: &str,
    content_base64: &str,
    expected_size: Option<u64>,
    expected_sha256: &str,
) -> Result<Vec<u8>, DaemonError> {
    validate_sha256(expected_sha256, "package file digest")?;
    let maximum_encoded = MAX_FILE_BYTES
        .saturating_add(2)
        .saturating_div(3)
        .saturating_mul(4) as usize;
    if content_base64.len() > maximum_encoded {
        return Err(import_error(format!(
            "kernel package file `{path}` exceeds its encoded limit"
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64)
        .map_err(|_| import_error(format!("kernel package file `{path}` is invalid base64")))?;
    if bytes.len() as u64 > MAX_FILE_BYTES
        || expected_size.is_some_and(|size| size != bytes.len() as u64)
        || sha256_hex(&bytes) != expected_sha256
    {
        return Err(import_error(format!(
            "kernel package file `{path}` does not match its digest"
        )));
    }
    Ok(bytes)
}

fn validate_runtime_version(version: &str) -> Result<(), DaemonError> {
    if version.len() > 64
        || version.split('.').count() < 2
        || version
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(import_error("portable runtime version is invalid"));
    }
    Ok(())
}

fn find_runtime(
    names: &[&str],
    expected_version: &str,
    label: &str,
    budget_root: &Path,
) -> Result<PathBuf, DaemonError> {
    for name in names {
        let Some(candidate) = find_on_path(name) else {
            continue;
        };
        let Ok((status, output)) = probe_runtime_version(&candidate, budget_root) else {
            continue;
        };
        let observed = String::from_utf8_lossy(&output);
        let observed = observed
            .split_whitespace()
            .find(|part| {
                part.chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_digit())
                    || part.starts_with('v')
            })
            .unwrap_or_default()
            .trim_start_matches('v');
        if status.success() && observed == expected_version {
            return Ok(candidate);
        }
    }
    Err(import_error(format!(
        "target image does not provide {label} {expected_version}"
    )))
}

fn probe_runtime_version(
    runtime: &Path,
    budget_root: &Path,
) -> Result<(ExitStatus, Vec<u8>), DaemonError> {
    let sequence = RUNTIME_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = budget_root.join(format!(".runtime-probe-{}-{sequence}", std::process::id()));
    cleanup_staging(&root)?;
    ensure_private_directory(&root)?;
    let _cleanup = TemporaryDirectoryCleanup(root.clone());
    let stdout_path = root.join("stdout");
    let stderr_path = root.join("stderr");
    let stdout = create_private_output_file(&stdout_path)?;
    let stderr = create_private_output_file(&stderr_path)?;
    let mut command = Command::new(runtime);
    command.arg("--version");
    sanitize_child_environment(&mut command, &root)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    configure_child_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| import_io_error("probe target runtime version", error))?;
    let status = wait_for_child(
        &mut child,
        "probe target runtime version",
        &root,
        RUNTIME_PROBE_TIMEOUT,
        MAX_RUNTIME_PROBE_BYTES,
        MAX_RUNTIME_PROBE_ENTRIES,
        false,
    )?;
    let stdout = read_bounded_regular_file(
        &stdout_path,
        MAX_RUNTIME_PROBE_BYTES,
        "read target runtime version",
    )?;
    let stderr = read_bounded_regular_file(
        &stderr_path,
        MAX_RUNTIME_PROBE_BYTES,
        "read target runtime version",
    )?;
    let mut output = if stdout.is_empty() { stderr } else { stdout };
    if output.len() as u64 > MAX_RUNTIME_PROBE_BYTES {
        return Err(import_error("target runtime version output is too large"));
    }
    output.truncate(MAX_RUNTIME_PROBE_BYTES as usize);
    Ok((status, output))
}

fn find_sibling_or_path_runtime(runtime: &Path, name: &str) -> Result<PathBuf, DaemonError> {
    if let Some(parent) = runtime.parent() {
        let sibling = parent.join(name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    find_on_path(name).ok_or_else(|| import_error(format!("target image does not provide {name}")))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for root in std::env::split_paths(&path) {
        let candidate = root.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = root.join(format!("{name}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn requirements_has_packages(requirements: &str) -> bool {
    requirements
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#') && line != "--require-hashes")
}

fn node_lock_has_packages(bytes: &[u8]) -> Result<bool, DaemonError> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|_| import_error("Node package lock is invalid"))?;
    Ok(value
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|packages| packages.keys().any(|path| !path.is_empty())))
}

fn run_install_command(
    command: &mut Command,
    operation: &'static str,
    budget_root: &Path,
) -> Result<(), DaemonError> {
    let runtime_root = budget_root.join(".installer-runtime");
    cleanup_staging(&runtime_root)?;
    ensure_private_directory(&runtime_root)?;
    let _cleanup = TemporaryDirectoryCleanup(runtime_root.clone());
    sanitize_child_environment(command, &runtime_root)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_child_process_group(command);
    let mut child = command
        .spawn()
        .map_err(|error| import_io_error(operation, error))?;
    let status = wait_for_child(
        &mut child,
        operation,
        budget_root,
        INSTALL_TIMEOUT,
        MAX_MATERIALIZED_CONTEXT_BYTES,
        MAX_MATERIALIZED_CONTEXT_ENTRIES,
        true,
    )?;
    if !status.success() {
        return Err(import_error(format!("{operation} failed")));
    }
    Ok(())
}

fn sanitize_child_environment(
    command: &mut Command,
    runtime_root: &Path,
) -> Result<(), DaemonError> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| import_error("target image PATH is not configured"))?;
    let home = runtime_root.join("home");
    let temporary = runtime_root.join("tmp");
    let npm_cache = runtime_root.join("npm-cache");
    for directory in [&home, &temporary, &npm_cache] {
        ensure_private_directory(directory)?;
    }
    command
        .env_clear()
        .env("PATH", path)
        .env("HOME", &home)
        .env("TMPDIR", &temporary)
        .env("TMP", &temporary)
        .env("TEMP", &temporary)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("PIP_CONFIG_FILE", null_device())
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .env("PIP_NO_INPUT", "1")
        .env("NPM_CONFIG_USERCONFIG", null_device())
        .env("NPM_CONFIG_CACHE", npm_cache)
        .env("NPM_CONFIG_IGNORE_SCRIPTS", "true")
        .env("NPM_CONFIG_BIN_LINKS", "false")
        .env("NPM_CONFIG_AUDIT", "false")
        .env("NPM_CONFIG_FUND", "false");
    Ok(())
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

fn create_private_output_file(path: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| import_io_error("create runtime probe output", error))
}

fn wait_for_child(
    child: &mut Child,
    operation: &'static str,
    budget_root: &Path,
    timeout: Duration,
    maximum_bytes: u64,
    maximum_entries: u64,
    allow_python_venv_symlink: bool,
) -> Result<ExitStatus, DaemonError> {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_child_descendants(child.id());
                ensure_tree_within_limits(
                    budget_root,
                    maximum_bytes,
                    maximum_entries,
                    allow_python_venv_symlink,
                )?;
                return Ok(status);
            }
            Ok(None) => {}
            Err(error) => {
                terminate_child_tree(child);
                return Err(import_io_error(operation, error));
            }
        }
        if let Err(error) = ensure_tree_within_limits(
            budget_root,
            maximum_bytes,
            maximum_entries,
            allow_python_venv_symlink,
        ) {
            terminate_child_tree(child);
            return Err(error);
        }
        if started.elapsed() >= timeout {
            terminate_child_tree(child);
            return Err(import_error(format!("{operation} timed out")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_child_descendants(process_group: u32) {
    let _ = unsafe { libc::kill(-(process_group as i32), libc::SIGKILL) };
}

#[cfg(not(unix))]
fn terminate_child_descendants(_process_group: u32) {}

fn terminate_child_tree(child: &mut Child) {
    terminate_child_descendants(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

struct TemporaryDirectoryCleanup(PathBuf);

impl Drop for TemporaryDirectoryCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
fn python_venv_binary(root: &Path) -> PathBuf {
    root.join("Scripts").join("python.exe")
}

#[cfg(not(windows))]
fn python_venv_binary(root: &Path) -> PathBuf {
    root.join("bin").join("python")
}

fn ensure_tree_within_budget(root: &Path) -> Result<(), DaemonError> {
    ensure_tree_within_limits(
        root,
        MAX_MATERIALIZED_CONTEXT_BYTES,
        MAX_MATERIALIZED_CONTEXT_ENTRIES,
        false,
    )
}

fn ensure_tree_within_limits(
    root: &Path,
    maximum_bytes: u64,
    maximum_entries: u64,
    allow_python_venv_symlink: bool,
) -> Result<(), DaemonError> {
    let root_entries = fs::read_dir(root)
        .map_err(|error| import_io_error("inspect materialized kernel context", error))?;
    let mut stack = vec![root_entries];
    let mut bytes = 0_u64;
    let mut entries = 0_u64;
    while let Some(directory_entries) = stack.pop() {
        for entry in directory_entries {
            account_materialized_entry(&mut bytes, &mut entries, maximum_bytes, maximum_entries)?;
            let Some(entry) = read_materialized_entry(entry)? else {
                continue;
            };
            let Some(metadata) = materialized_entry_file_type(&entry)? else {
                continue;
            };
            if metadata.is_symlink() {
                let Some(target) = read_materialized_symlink(&entry.path())? else {
                    continue;
                };
                if !allow_python_venv_symlink
                    || !is_python_venv_compatibility_symlink_path(root, &entry.path())?
                    || target != Path::new("lib")
                {
                    return Err(import_error(
                        "materialized kernel context contains a symbolic link",
                    ));
                }
            } else if metadata.is_dir() {
                let Some(child_entries) = read_discovered_materialized_directory(&entry.path())?
                else {
                    continue;
                };
                stack.push(child_entries);
            } else if metadata.is_file() {
                let Some(size) = materialized_entry_size(&entry)? else {
                    continue;
                };
                bytes = bytes.saturating_add(size);
                ensure_materialized_tree_limits(bytes, entries, maximum_bytes, maximum_entries)?;
            } else {
                return Err(import_error(
                    "materialized kernel context contains a special file",
                ));
            }
        }
    }
    Ok(())
}

fn account_materialized_entry(
    bytes: &mut u64,
    entries: &mut u64,
    maximum_bytes: u64,
    maximum_entries: u64,
) -> Result<(), DaemonError> {
    *bytes = bytes.saturating_add(4096);
    *entries = entries.saturating_add(1);
    ensure_materialized_tree_limits(*bytes, *entries, maximum_bytes, maximum_entries)
}

fn ensure_materialized_tree_limits(
    bytes: u64,
    entries: u64,
    maximum_bytes: u64,
    maximum_entries: u64,
) -> Result<(), DaemonError> {
    if entries > maximum_entries || bytes > maximum_bytes {
        return Err(import_error(
            "materialized kernel context exceeds its resource limits",
        ));
    }
    Ok(())
}

fn read_discovered_materialized_directory(path: &Path) -> Result<Option<fs::ReadDir>, DaemonError> {
    match fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(import_io_error(
            "inspect materialized kernel context",
            error,
        )),
    }
}

fn read_materialized_entry(
    entry: io::Result<fs::DirEntry>,
) -> Result<Option<fs::DirEntry>, DaemonError> {
    match entry {
        Ok(entry) => Ok(Some(entry)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(import_io_error(
            "inspect materialized kernel context",
            error,
        )),
    }
}

fn materialized_entry_file_type(entry: &fs::DirEntry) -> Result<Option<fs::FileType>, DaemonError> {
    match entry.file_type() {
        Ok(file_type) => Ok(Some(file_type)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(import_io_error(
            "inspect materialized kernel context",
            error,
        )),
    }
}

fn materialized_entry_size(entry: &fs::DirEntry) -> Result<Option<u64>, DaemonError> {
    match entry.metadata() {
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(import_io_error(
            "inspect materialized kernel context",
            error,
        )),
    }
}

fn read_materialized_symlink(path: &Path) -> Result<Option<PathBuf>, DaemonError> {
    match fs::read_link(path) {
        Ok(target) => Ok(Some(target)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(import_io_error(
            "inspect materialized kernel context",
            error,
        )),
    }
}

fn is_python_venv_compatibility_symlink(root: &Path, path: &Path) -> Result<bool, DaemonError> {
    if !is_python_venv_compatibility_symlink_path(root, path)? {
        return Ok(false);
    }
    fs::read_link(path)
        .map(|target| target == Path::new("lib"))
        .map_err(|error| import_io_error("inspect Python virtual environment symlink", error))
}

fn is_python_venv_compatibility_symlink_path(
    root: &Path,
    path: &Path,
) -> Result<bool, DaemonError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| import_error("materialized kernel context path escaped staging"))?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 6
        || components[0].as_os_str() != "user"
        || components[1].as_os_str() != "envs"
        || components[2].as_os_str() != ".portable"
        || components[4].as_os_str() != "venv"
        || components[5].as_os_str() != "lib64"
    {
        return Ok(false);
    }
    Ok(true)
}

fn remove_python_venv_compatibility_symlink(
    staging_root: &Path,
    venv_root: &Path,
) -> Result<(), DaemonError> {
    let path = venv_root.join("lib64");
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                && is_python_venv_compatibility_symlink(staging_root, &path)? =>
        {
            fs::remove_file(path).map_err(|error| {
                import_io_error(
                    "remove Python virtual environment compatibility symlink",
                    error,
                )
            })
        }
        Ok(_) => Err(import_error(
            "Python virtual environment contains an unexpected lib64 entry",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(import_io_error(
            "inspect Python virtual environment compatibility symlink",
            error,
        )),
    }
}

fn read_bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, DaemonError> {
    read_bounded_regular_file(path, maximum, "read kernel context receipt")
}

fn read_bounded_regular_file(
    path: &Path,
    maximum: u64,
    operation: &'static str,
) -> Result<Vec<u8>, DaemonError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| import_io_error(operation, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(import_error(
            "kernel context file is not a bounded regular file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| import_io_error(operation, error))?;
    if bytes.len() as u64 > maximum {
        return Err(import_error("kernel context file exceeds its size limit"));
    }
    Ok(bytes)
}

fn sync_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| import_io_error("sync kernel context directory", error))?;
    }
    Ok(())
}

fn sync_private_tree(root: &Path) -> Result<(), DaemonError> {
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        directories.push(directory.clone());
        for entry in fs::read_dir(&directory)
            .map_err(|error| import_io_error("sync kernel context tree", error))?
        {
            let entry =
                entry.map_err(|error| import_io_error("sync kernel context tree", error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| import_io_error("sync kernel context tree", error))?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                File::open(entry.path())
                    .and_then(|file| file.sync_all())
                    .map_err(|error| import_io_error("sync kernel context file", error))?;
            } else {
                return Err(import_error(
                    "kernel context staging contains an unsupported filesystem entry",
                ));
            }
        }
    }
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    use std::os::unix::ffi::OsStrExt;
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| import_error("kernel context staging path contains NUL"))?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| import_error("kernel context destination path contains NUL"))?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(import_io_error(
            "publish kernel context capability root",
            io::Error::last_os_error(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    use std::os::unix::ffi::OsStrExt;
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| import_error("kernel context staging path contains NUL"))?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| import_error("kernel context destination path contains NUL"))?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(import_io_error(
            "publish kernel context capability root",
            io::Error::last_os_error(),
        ))
    }
}

#[cfg(windows)]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    fs::rename(source, destination)
        .map_err(|error| import_io_error("publish kernel context capability root", error))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn publish_directory_no_clobber(_source: &Path, _destination: &Path) -> Result<(), DaemonError> {
    Err(import_error(
        "atomic kernel context publication is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| import_io_error("protect kernel context directory", error))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), DaemonError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path, executable: bool) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
    )
    .map_err(|error| import_io_error("protect kernel context file", error))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path, _executable: bool) -> Result<(), DaemonError> {
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn script_registry_definition_hash(
    source: &[u8],
    description: &str,
    input_schema: &serde_json::Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source);
    hasher.update(description.as_bytes());
    hasher.update(input_schema.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn import_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "kernel_context_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

fn import_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_kernel_context",
        operation: "kernel_context.import",
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{TransferredVaultSourceBinding, VaultUnlockLease};

    #[test]
    fn imports_unified_kernel_context_and_replays_exact_receipt() {
        let _guard = crate::env_lock::lock();
        let root = test_root("round-trip");
        let source_vault = root.join("source-vault.json");
        let target_vault = root.join("target-vault.json");
        let capability_root = root.join("target").join("managed-context").join("kernel");
        let previous_paths = set_managed_import_paths(&capability_root, &target_vault);
        let source_private = crate::transport::relay_crypto::generate_private_key_base64();
        let target_private = crate::transport::relay_crypto::generate_private_key_base64();
        let target_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&target_private)
                .expect("target public key");

        crate::secret::unlock_chariox_encrypted_vault(
            &source_vault,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("source Vault should unlock");
        let vault = crate::secret::export_transferred_vault_snapshot(
            &source_vault,
            "context-1",
            "source-kernel",
            &source_private,
            "target-kernel",
            &target_public,
        )
        .expect("Vault snapshot should export");
        let expected_source = TransferredVaultSourceBinding {
            context_id: vault.context_id.clone(),
            source_kernel_id: vault.source_kernel_id.clone(),
            source_key_thumbprint: vault.source_key_thumbprint.clone(),
        };
        let snapshot = fixture_snapshot(vault);
        let request = KernelContextImportRequest {
            snapshot: snapshot.clone(),
            expected_source,
            target_kernel_id: "target-kernel".to_string(),
            target_private_key: target_private,
            capability_root: capability_root.clone(),
            vault_path: target_vault.clone(),
        };

        let receipt = import_kernel_context(request.clone()).expect("kernel context should import");
        assert_eq!(receipt.extension_count, 4);
        assert_eq!(receipt.dependency_count, 0);
        assert_eq!(
            import_kernel_context(request.clone()).expect("exact import should replay"),
            receipt
        );

        let previous_isolation = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &capability_root);
        assert_eq!(
            crate::credential::CharioxCredentialRegistry::user_root(),
            Some(capability_root.join("user").join("credentials"))
        );
        assert_eq!(
            crate::connector::CharioxConnectorRegistry::user_root(),
            Some(
                capability_root
                    .join("user")
                    .join("connectors")
                    .join("definitions")
            )
        );
        assert_eq!(
            crate::connector::CharioxConnectorAdapterRegistry::user_root(),
            Some(
                capability_root
                    .join("user")
                    .join("connectors")
                    .join("adapters")
            )
        );
        restore_env("CHARIOX_CAPABILITY_ISOLATION_ROOT", previous_isolation);

        let mcps =
            crate::mcp::CharioxMcpRegistry::new(vec![capability_root.join("user").join("mcps")])
                .list()
                .expect("imported MCPs should load");
        assert_eq!(mcps.len(), 2);
        let portable = mcps
            .iter()
            .find(|config| config.name == "portable")
            .expect("portable stdio MCP should import");
        let CharioxMcpTransportConfig::Stdio { command, cwd, .. } = &portable.transport else {
            panic!("portable MCP should remain stdio");
        };
        let runtime_root = capability_root.join("user/mcps/portable");
        assert_eq!(Path::new(command), runtime_root.join("bin/server"));
        assert_eq!(cwd.as_deref(), Some(runtime_root.as_path()));
        assert_eq!(
            fs::read(runtime_root.join("bin/server")).expect("stdio runtime should read"),
            b"#!/bin/sh\nexit 0\n",
        );
        let skills = crate::skill::CharioxSkillRegistry::new(vec![capability_root
            .join("user")
            .join("skills")])
        .list()
        .expect("imported skills should load");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "review");
        let scripts = crate::script::CharioxScriptRegistry::new(vec![capability_root
            .join("user")
            .join("scripts")])
        .list()
        .expect("imported scripts should load");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "helper");
        assert_eq!(
            fs::read(&target_vault).expect("target Vault should read"),
            fs::read(&source_vault).expect("source Vault should read")
        );
        assert!(
            crate::secret::chariox_encrypted_vault_status(&target_vault)
                .expect("target Vault status should load")
                .unlocked
        );
        let receipt_debug = format!("{receipt:?}");
        assert!(!receipt_debug.contains(&snapshot.payload.vault.vault_file_base64));

        crate::secret::lock_chariox_encrypted_vault(&source_vault).ok();
        crate::secret::lock_chariox_encrypted_vault(&target_vault).ok();
        fs::remove_file(&target_vault).expect("target Vault should remove");
        assert!(import_kernel_context(request)
            .expect_err("replay must validate current Vault health")
            .to_string()
            .contains("Vault"));
        restore_managed_import_paths(previous_paths);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_wrong_bindings_and_occupied_roots_before_publication() {
        let _guard = crate::env_lock::lock();
        let root = test_root("reject");
        let source_vault = root.join("source-vault.json");
        let target_vault = root.join("target-vault.json");
        let capability_root = root.join("target").join("managed-context").join("kernel");
        let previous_paths = set_managed_import_paths(&capability_root, &target_vault);
        let source_private = crate::transport::relay_crypto::generate_private_key_base64();
        let target_private = crate::transport::relay_crypto::generate_private_key_base64();
        let target_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&target_private)
                .expect("target public key");
        crate::secret::unlock_chariox_encrypted_vault(
            &source_vault,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("source Vault should unlock");
        let vault = crate::secret::export_transferred_vault_snapshot(
            &source_vault,
            "context-2",
            "source-kernel",
            &source_private,
            "target-kernel",
            &target_public,
        )
        .expect("Vault snapshot should export");
        let snapshot = fixture_snapshot(vault.clone());
        let wrong_destination = KernelContextImportRequest {
            snapshot: snapshot.clone(),
            expected_source: TransferredVaultSourceBinding {
                context_id: vault.context_id.clone(),
                source_kernel_id: vault.source_kernel_id.clone(),
                source_key_thumbprint: vault.source_key_thumbprint.clone(),
            },
            target_kernel_id: "target-kernel".to_string(),
            target_private_key: target_private.clone(),
            capability_root: root.join("unconfigured-capabilities"),
            vault_path: target_vault.clone(),
        };
        assert!(import_kernel_context(wrong_destination)
            .expect_err("unconfigured destination should reject")
            .to_string()
            .contains("running managed kernel configuration"));
        let wrong_source = KernelContextImportRequest {
            snapshot: snapshot.clone(),
            expected_source: TransferredVaultSourceBinding {
                context_id: "wrong-context".to_string(),
                source_kernel_id: vault.source_kernel_id.clone(),
                source_key_thumbprint: vault.source_key_thumbprint.clone(),
            },
            target_kernel_id: "target-kernel".to_string(),
            target_private_key: target_private.clone(),
            capability_root: capability_root.clone(),
            vault_path: target_vault.clone(),
        };
        let error = import_kernel_context(wrong_source).expect_err("wrong source should reject");
        assert!(error.to_string().contains("binding does not match"));
        assert!(!capability_root.exists());
        assert!(!target_vault.exists());

        fs::create_dir_all(&capability_root).expect("occupied root should create");
        fs::write(capability_root.join("owner"), "other\n").expect("occupied marker should write");
        let occupied = KernelContextImportRequest {
            snapshot,
            expected_source: TransferredVaultSourceBinding {
                context_id: vault.context_id,
                source_kernel_id: vault.source_kernel_id,
                source_key_thumbprint: vault.source_key_thumbprint,
            },
            target_kernel_id: "target-kernel".to_string(),
            target_private_key: target_private,
            capability_root: capability_root.clone(),
            vault_path: target_vault.clone(),
        };
        let error = import_kernel_context(occupied).expect_err("occupied root should reject");
        assert!(error.to_string().contains("receipt"));
        assert!(!target_vault.exists());
        assert_eq!(
            fs::read_to_string(capability_root.join("owner")).expect("owner marker should remain"),
            "other\n"
        );

        crate::secret::lock_chariox_encrypted_vault(&source_vault).ok();
        restore_managed_import_paths(previous_paths);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_cross_platform_package_aliases_and_unknown_environment_manifest_fields() {
        let content = b"safe";
        let sha256 = sha256_hex(content);
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        let aliases = vec![
            KernelPackageFile {
                path: "Docs/file.txt".to_string(),
                sha256: sha256.clone(),
                size_bytes: content.len() as u64,
                executable: false,
                content_base64: encoded.clone(),
            },
            KernelPackageFile {
                path: "docs/file.txt".to_string(),
                sha256,
                size_bytes: content.len() as u64,
                executable: false,
                content_base64: encoded,
            },
        ];
        assert!(validate_package_files(&aliases)
            .expect_err("portable aliases should reject")
            .to_string()
            .contains("duplicate"));

        let manifest =
            br#"{"schema_version":1,"runtime":"python","version":"3.11.8","extra":"no"}"#;
        let requirements = b"--require-hashes\n";
        let files = vec![
            kernel_file("manifest.json", manifest),
            kernel_file("requirements.lock", requirements),
        ];
        assert!(validate_environment_manifest(
            "python",
            "3.11.8",
            PortableEnvironmentKind::Python,
            &files,
            &["manifest.json", "requirements.lock"],
        )
        .expect_err("unknown manifest fields should reject")
        .to_string()
        .contains("manifest is invalid"));
    }

    #[cfg(unix)]
    #[test]
    fn permits_only_the_standard_python_venv_compatibility_symlink_during_install() {
        use std::os::unix::fs::symlink;

        let root = test_root("python-venv-symlink");
        let venv = root.join("user/envs/.portable/python/venv");
        fs::create_dir_all(venv.join("lib")).expect("venv lib should create");
        symlink("lib", venv.join("lib64")).expect("venv compatibility link should create");

        ensure_tree_within_limits(
            &root,
            MAX_MATERIALIZED_CONTEXT_BYTES,
            MAX_MATERIALIZED_CONTEXT_ENTRIES,
            true,
        )
        .expect("installer scan should permit the exact venv compatibility link");
        assert!(ensure_tree_within_budget(&root)
            .expect_err("published context must reject every symlink")
            .to_string()
            .contains("symbolic link"));
        remove_python_venv_compatibility_symlink(&root, &venv)
            .expect("compatibility link should remove");
        ensure_tree_within_budget(&root).expect("final tree should be link-free");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn budget_scan_tolerates_a_nested_directory_removed_after_discovery() {
        let root = test_root("budget-scan-vanished-directory");
        let vanished = root.join("transient");
        fs::create_dir_all(&vanished).expect("transient directory should create");
        fs::remove_dir(&vanished).expect("transient directory should disappear");

        let mut bytes = 0;
        let mut entries = 0;
        account_materialized_entry(
            &mut bytes,
            &mut entries,
            MAX_MATERIALIZED_CONTEXT_BYTES,
            MAX_MATERIALIZED_CONTEXT_ENTRIES,
        )
        .expect("discovered directory should count toward the scan budget");
        assert!(read_discovered_materialized_directory(&vanished)
            .expect("a previously discovered nested directory may disappear during scanning")
            .is_none());
        assert_eq!((bytes, entries), (4096, 1));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn budget_scan_requires_its_root_to_remain_available() {
        let root = test_root("budget-scan-missing-root");
        fs::remove_dir(&root).expect("scan root should disappear");

        let error = ensure_tree_within_budget(&root).expect_err("missing root should fail");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                operation: "inspect materialized kernel context",
                ..
            }
        ));
    }

    #[test]
    fn budget_scan_tolerates_an_entry_removed_after_directory_read() {
        let root = test_root("budget-scan-vanished-entry");
        let transient = root.join("transient");
        fs::write(&transient, b"temporary").expect("transient file should create");
        let entry = fs::read_dir(&root)
            .expect("scan root should read")
            .next()
            .expect("transient entry should exist")
            .expect("transient entry should read");
        fs::remove_file(&transient).expect("transient entry should disappear");

        let mut bytes = 0;
        let mut entries = 0;
        account_materialized_entry(
            &mut bytes,
            &mut entries,
            MAX_MATERIALIZED_CONTEXT_BYTES,
            MAX_MATERIALIZED_CONTEXT_ENTRIES,
        )
        .expect("discovered entry should count toward the scan budget");
        assert_eq!(
            materialized_entry_size(&entry)
                .expect("a previously discovered entry may disappear during scanning"),
            None
        );
        assert_eq!((bytes, entries), (4096, 1));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn standard_linux_python_venv_is_publishable_after_compatibility_cleanup() {
        let Some(python) = find_on_path("python3").or_else(|| find_on_path("python")) else {
            return;
        };
        let root = test_root("real-python-venv");
        let venv = root.join("user/envs/.portable/python/venv");
        fs::create_dir_all(venv.parent().expect("venv should have parent"))
            .expect("environment root should create");
        let mut command = Command::new(python);
        command.arg("-m").arg("venv").arg("--copies").arg(&venv);

        run_install_command(
            &mut command,
            "create test Python virtual environment",
            &root,
        )
        .expect("standard Python venv should create");
        remove_python_venv_compatibility_symlink(&root, &venv)
            .expect("standard lib64 link should remove when present");
        ensure_tree_within_budget(&root).expect("cleaned venv should be publishable");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn dependency_installer_clears_ambient_secrets_and_kills_descendants() {
        let _guard = crate::env_lock::lock();
        let root = test_root("installer-isolation");
        let previous = std::env::var_os("CHARIOX_INSTALLER_SECRET_CANARY");
        std::env::set_var("CHARIOX_INSTALLER_SECRET_CANARY", "must-not-arrive");
        let mut command = Command::new("/bin/sh");
        command.current_dir(&root).arg("-c").arg(
            "printf '%s' \"${CHARIOX_INSTALLER_SECRET_CANARY-unset}\" > inherited.txt; (sleep 1; printf leaked > descendant.txt) &",
        );

        run_install_command(&mut command, "test isolated installer", &root)
            .expect("isolated installer should finish");
        restore_env("CHARIOX_INSTALLER_SECRET_CANARY", previous);
        assert_eq!(
            fs::read_to_string(root.join("inherited.txt")).expect("canary result should read"),
            "unset"
        );
        std::thread::sleep(Duration::from_millis(1_200));
        assert!(!root.join("descendant.txt").exists());
        assert!(!root.join(".installer-runtime").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_publication_refuses_an_existing_empty_destination() {
        let root = test_root("publish-no-clobber");
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).expect("source should create");
        fs::create_dir_all(&destination).expect("destination should create");

        assert!(publish_directory_no_clobber(&source, &destination).is_err());
        assert!(source.is_dir());
        assert!(destination.is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_probe_rejects_burst_output_after_the_process_exits() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("runtime-probe-bound");
        let staging = root.join("staging");
        fs::create_dir_all(&staging).expect("staging should create");
        let runtime = root.join("runtime");
        fs::write(
            &runtime,
            "#!/bin/sh\ndd if=/dev/zero bs=131072 count=1 2>/dev/null\n",
        )
        .expect("runtime probe fixture should write");
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
            .expect("runtime probe fixture should be executable");

        assert!(probe_runtime_version(&runtime, &staging)
            .expect_err("oversized runtime output should reject")
            .to_string()
            .contains("resource limits"));
        assert!(fs::read_dir(&staging)
            .expect("staging should remain readable")
            .next()
            .is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn fixture_snapshot(vault: crate::secret::TransferredVaultSnapshot) -> KernelContextSnapshot {
        let mcp_definition = KernelExtensionDefinition::Mcp {
            config: Box::new(crate::mcp::CharioxMcpServerConfig::streamable_http(
                "docs",
                "https://example.test/mcp",
            )),
            runtime: None,
        };
        let stdio_bytes = b"#!/bin/sh\nexit 0\n";
        let stdio_files = vec![KernelPackageFile {
            path: "bin/server".to_string(),
            sha256: sha256_hex(stdio_bytes),
            size_bytes: stdio_bytes.len() as u64,
            executable: true,
            content_base64: base64::engine::general_purpose::STANDARD.encode(stdio_bytes),
        }];
        let stdio_definition = KernelExtensionDefinition::Mcp {
            config: Box::new(crate::mcp::CharioxMcpServerConfig::stdio(
                "portable",
                "bin/server",
                Vec::new(),
            )),
            runtime: Some(super::super::KernelMcpStdioRuntimeSnapshot {
                command_path: "bin/server".to_string(),
                cwd_path: None,
                package_sha256: package_definition_hash(&stdio_files),
                files: stdio_files,
            }),
        };
        let skill_bytes = b"---\nname: review\ndescription: Review code\n---\nReview carefully.\n";
        let skill_sha256 = sha256_hex(skill_bytes);
        let mut skill_hasher = Sha256::new();
        skill_hasher.update(b"SKILL.md");
        skill_hasher.update([0]);
        skill_hasher.update(skill_sha256.as_bytes());
        skill_hasher.update([0]);
        let skill_definition = KernelExtensionDefinition::Skill {
            package: crate::skill::CharioxSkillPackage {
                metadata: crate::skill::CharioxSkillMetadata {
                    name: "review".to_string(),
                    description: "Review code".to_string(),
                    short_description: None,
                    path: PathBuf::from("SKILL.md"),
                },
                version_hash: format!("{:x}", skill_hasher.finalize()),
                files: vec![crate::skill::CharioxSkillPackageFile {
                    path: "SKILL.md".to_string(),
                    sha256: skill_sha256,
                    content_base64: base64::engine::general_purpose::STANDARD.encode(skill_bytes),
                }],
            },
            executable_paths: Vec::new(),
        };
        let script_bytes = b"def run(value: str) -> str:\n    \"\"\"Return value.\"\"\"\n    return value\n\ndef test_run():\n    assert run('ok') == 'ok'\n";
        let script_definition = KernelExtensionDefinition::Script {
            script: super::super::KernelScriptSnapshot {
                runtime: CharioxScriptRuntime::Python,
                description: "Return value.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false
                }),
                timeout_sec: Some(30),
                source_sha256: sha256_hex(script_bytes),
                source_base64: base64::engine::general_purpose::STANDARD.encode(script_bytes),
            },
        };
        let extensions = vec![
            extension(ExtensionKind::Mcp, "docs", mcp_definition),
            extension(ExtensionKind::Mcp, "portable", stdio_definition),
            extension(ExtensionKind::Skill, "review", skill_definition),
            extension(ExtensionKind::Script, "helper", script_definition),
        ];
        let payload = super::super::KernelContextPayload {
            schema_version: KERNEL_CONTEXT_SCHEMA_VERSION,
            context_id: vault.context_id.clone(),
            source_kernel_id: vault.source_kernel_id.clone(),
            source_key_thumbprint: vault.source_key_thumbprint.clone(),
            target_kernel_id: vault.target_kernel_id.clone(),
            target_key_thumbprint: vault.target_key_thumbprint.clone(),
            compatibility: super::super::KernelContextCompatibility {
                source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version:
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions,
            dependencies: Vec::new(),
            vault,
        };
        let (_, snapshot_sha256) =
            serialized_json_measure(&payload, MAX_SNAPSHOT_BYTES).expect("snapshot should hash");
        KernelContextSnapshot {
            payload,
            snapshot_sha256,
        }
    }

    fn extension(
        kind: ExtensionKind,
        name: &str,
        definition: KernelExtensionDefinition,
    ) -> super::super::KernelExtensionSnapshot {
        let definition_sha256 =
            extension_definition_hash(&definition).expect("definition should hash");
        super::super::KernelExtensionSnapshot {
            kind,
            scope: KernelExtensionScope::User,
            name: name.to_string(),
            definition_sha256,
            definition,
        }
    }

    fn kernel_file(path: &str, bytes: &[u8]) -> KernelPackageFile {
        KernelPackageFile {
            path: path.to_string(),
            sha256: sha256_hex(bytes),
            size_bytes: bytes.len() as u64,
            executable: false,
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    fn set_managed_import_paths(
        capability_root: &Path,
        vault_path: &Path,
    ) -> (Option<std::ffi::OsString>, Option<std::ffi::OsString>) {
        let previous = (
            std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT"),
            std::env::var_os("CHARIOX_MANAGED_VAULT_PATH"),
        );
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", capability_root);
        std::env::set_var("CHARIOX_MANAGED_VAULT_PATH", vault_path);
        previous
    }

    fn restore_managed_import_paths(
        previous: (Option<std::ffi::OsString>, Option<std::ffi::OsString>),
    ) {
        restore_env("CHARIOX_CAPABILITY_ISOLATION_ROOT", previous.0);
        restore_env("CHARIOX_MANAGED_VAULT_PATH", previous.1);
    }

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "chariox-kernel-context-import-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test root should create");
        root
    }
}
