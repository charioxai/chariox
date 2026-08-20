use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const KERNEL_CONTEXT_SCHEMA_VERSION: u32 = 1;
const MAX_EXTENSIONS: usize = 2_048;
const MAX_DEPENDENCIES: usize = 2_048;
const MAX_PACKAGE_FILES: usize = 1_024;
const MAX_PACKAGE_ENTRIES: usize = 2_048;
const MAX_DIRECTORY_DEPTH: usize = 64;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 16_384;

struct SnapshotMemoryBudget {
    bytes: usize,
    files: usize,
}

impl SnapshotMemoryBudget {
    fn new(request: &KernelContextExportRequest) -> Result<Self, DaemonError> {
        let mut bytes = request
            .context_id
            .len()
            .saturating_add(request.source_kernel_id.len())
            .saturating_add(request.source_key_thumbprint.len())
            .saturating_add(request.target_kernel_id.len())
            .saturating_add(request.target_key_thumbprint.len())
            .saturating_add(4096);
        bytes =
            bytes.saturating_add(serialized_json_measure(&request.vault, MAX_SNAPSHOT_BYTES)?.0);
        let budget = Self { bytes, files: 1 };
        budget.ensure_available()?;
        Ok(budget)
    }

    fn consume<T: Serialize>(&mut self, value: &T) -> Result<(), DaemonError> {
        let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(self.bytes);
        self.bytes = self
            .bytes
            .saturating_add(serialized_json_measure(value, remaining)?.0);
        self.ensure_available()
    }

    fn consume_files(&mut self, count: usize) -> Result<(), DaemonError> {
        self.files = self.files.saturating_add(count);
        if self.files > MAX_SNAPSHOT_FILES {
            return Err(kernel_context_error(
                "kernel context snapshot file count exceeds its limit",
            ));
        }
        Ok(())
    }

    fn ensure_available(&self) -> Result<(), DaemonError> {
        if self.bytes > MAX_SNAPSHOT_BYTES {
            return Err(kernel_context_error(
                "kernel context snapshot exceeds its size limit",
            ));
        }
        Ok(())
    }
}

struct BoundedJsonHashWriter {
    bytes: usize,
    maximum_bytes: usize,
    hasher: Sha256,
}

impl Write for BoundedJsonHashWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() > self.maximum_bytes.saturating_sub(self.bytes) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "serialized kernel context exceeds its size limit",
            ));
        }
        self.hasher.update(buffer);
        self.bytes += buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_json_measure<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<(usize, String), DaemonError> {
    let mut writer = BoundedJsonHashWriter {
        bytes: 0,
        maximum_bytes,
        hasher: Sha256::new(),
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        kernel_context_error(format!("serialize kernel context component: {error}"))
    })?;
    Ok((writer.bytes, format!("{:x}", writer.hasher.finalize())))
}

use super::*;
use crate::config::{UserCredentialInjectionConfig, UserCredentialSourceConfig};
use crate::connector::{CharioxConnectorAdapterDefinition, ConnectorAdapterSource};
use crate::mcp::CharioxMcpTransportConfig;
use crate::script::CharioxEnvironmentRuntime;

pub fn export_kernel_context(
    request: KernelContextExportRequest,
) -> Result<KernelContextSnapshot, DaemonError> {
    validate_identifier(&request.context_id, "context id")?;
    validate_identifier(&request.source_kernel_id, "source kernel id")?;
    validate_sha256(&request.source_key_thumbprint, "source key thumbprint")?;
    validate_identifier(&request.target_kernel_id, "target kernel id")?;
    validate_sha256(&request.target_key_thumbprint, "target key thumbprint")?;
    crate::secret::validate_transferred_vault_snapshot_for_export(
        &request.vault,
        &crate::secret::TransferredVaultSourceBinding {
            context_id: request.context_id.clone(),
            source_kernel_id: request.source_kernel_id.clone(),
            source_key_thumbprint: request.source_key_thumbprint.clone(),
        },
        &request.target_kernel_id,
        &request.target_key_thumbprint,
    )?;
    let sources = super::source_snapshot::KernelContextSourceSnapshot::capture()?;
    let mut budget = SnapshotMemoryBudget::new(&request)?;

    let mut extensions = Vec::new();
    export_mcps(&sources.mcp_root, &mut extensions, &mut budget)?;
    export_skills(&sources.skill_root, &mut extensions, &mut budget)?;
    export_scripts(&sources.script_root, &mut extensions, &mut budget)?;
    let referenced_connector_adapters =
        export_connectors(&sources.connector_root, &mut extensions, &mut budget)?;
    extensions.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
    });
    if extensions.len() > MAX_EXTENSIONS {
        return Err(kernel_context_error(
            "kernel Extension count exceeds its limit",
        ));
    }
    let unique_extensions = extensions
        .iter()
        .map(|extension| (extension.kind.clone(), extension.name.as_str()))
        .collect::<BTreeSet<_>>();
    if unique_extensions.len() != extensions.len() {
        return Err(kernel_context_error(
            "kernel Extension snapshot contains duplicate definitions",
        ));
    }

    let mut dependencies = Vec::new();
    export_environments(
        &sources.environment_root,
        sources.original_environment_root.as_deref(),
        &mut dependencies,
        &mut budget,
    )?;
    export_connector_adapters(
        &sources.connector_adapter_root,
        sources.bundled_adapter_roots.clone(),
        &mut dependencies,
        &mut budget,
        &referenced_connector_adapters,
    )?;
    export_credentials(&sources.credential_root, &mut dependencies, &mut budget)?;
    if dependencies.len() > MAX_DEPENDENCIES {
        return Err(kernel_context_error(
            "kernel Extension dependency count exceeds its limit",
        ));
    }
    let payload = KernelContextPayload {
        schema_version: KERNEL_CONTEXT_SCHEMA_VERSION,
        context_id: request.context_id,
        source_kernel_id: request.source_kernel_id,
        source_key_thumbprint: request.source_key_thumbprint,
        target_kernel_id: request.target_kernel_id,
        target_key_thumbprint: request.target_key_thumbprint,
        compatibility: KernelContextCompatibility {
            source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
            local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            relay_peer_protocol_version: crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        },
        extensions,
        dependencies,
        vault: request.vault,
    };
    let (_, snapshot_sha256) = serialized_json_measure(&payload, MAX_SNAPSHOT_BYTES)?;
    Ok(KernelContextSnapshot {
        snapshot_sha256,
        payload,
    })
}

fn export_mcps(
    root: &Path,
    extensions: &mut Vec<KernelExtensionSnapshot>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<(), DaemonError> {
    let registry = crate::mcp::CharioxMcpRegistry::new(vec![root.to_path_buf()]);
    for config in registry.list()? {
        validate_portable_mcp(&config)?;
        let name = config.name.clone();
        let definition = KernelExtensionDefinition::Mcp { config };
        push_extension(
            extensions,
            budget,
            KernelExtensionSnapshot {
                kind: ExtensionKind::Mcp,
                scope: KernelExtensionScope::User,
                name,
                definition_sha256: extension_definition_hash(&definition)?,
                definition,
            },
        )?;
    }
    Ok(())
}

fn export_skills(
    root: &Path,
    extensions: &mut Vec<KernelExtensionSnapshot>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<(), DaemonError> {
    let registry = crate::skill::CharioxSkillRegistry::new(vec![root.to_path_buf()]);
    for metadata in registry.list()? {
        let skill_root = metadata
            .path
            .parent()
            .ok_or_else(|| kernel_context_error("skill metadata path has no parent"))?;
        let mut package = registry.package(&metadata.name)?.ok_or_else(|| {
            kernel_context_error(format!(
                "skill `{}` disappeared during export",
                metadata.name
            ))
        })?;
        let executable_paths = skill_executable_paths(skill_root, &package)?;
        package.metadata.path = PathBuf::from("SKILL.md");
        let definition = KernelExtensionDefinition::Skill {
            package,
            executable_paths,
        };
        push_extension(
            extensions,
            budget,
            KernelExtensionSnapshot {
                kind: ExtensionKind::Skill,
                scope: KernelExtensionScope::User,
                name: metadata.name,
                definition_sha256: extension_definition_hash(&definition)?,
                definition,
            },
        )?;
    }
    Ok(())
}

fn export_scripts(
    root: &Path,
    extensions: &mut Vec<KernelExtensionSnapshot>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<(), DaemonError> {
    let registry = crate::script::CharioxScriptRegistry::new(vec![root.to_path_buf()]);
    for metadata in registry.list()? {
        crate::mcp::validate_registry_name(&metadata.name, "script name")?;
        let source_path = validate_captured_file(root, &metadata.path, "script entrypoint")?;
        let source = read_bounded_regular_file(&source_path, MAX_FILE_BYTES)?;
        validate_safe_package_file("script-source", &source)?;
        let definition = KernelExtensionDefinition::Script {
            script: KernelScriptSnapshot {
                runtime: metadata.runtime,
                description: metadata.description,
                input_schema: metadata.input_schema,
                timeout_sec: metadata.timeout_sec,
                source_sha256: sha256_hex(&source),
                source_base64: base64::engine::general_purpose::STANDARD.encode(source),
            },
        };
        push_extension(
            extensions,
            budget,
            KernelExtensionSnapshot {
                kind: ExtensionKind::Script,
                scope: KernelExtensionScope::User,
                name: metadata.name,
                definition_sha256: extension_definition_hash(&definition)?,
                definition,
            },
        )?;
    }
    Ok(())
}

fn export_connectors(
    root: &Path,
    extensions: &mut Vec<KernelExtensionSnapshot>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<BTreeSet<String>, DaemonError> {
    let registry = crate::connector::CharioxConnectorRegistry::new(root.to_path_buf());
    let mut referenced_adapters = BTreeSet::new();
    for definition in registry.list()? {
        for operation in &definition.operations {
            reject_literal_secrets_in_json(&operation.config)?;
        }
        referenced_adapters.insert(definition.adapter.clone());
        let name = definition.name.clone();
        let definition = KernelExtensionDefinition::Connector { definition };
        push_extension(
            extensions,
            budget,
            KernelExtensionSnapshot {
                kind: ExtensionKind::Connector,
                scope: KernelExtensionScope::User,
                name,
                definition_sha256: extension_definition_hash(&definition)?,
                definition,
            },
        )?;
    }
    Ok(referenced_adapters)
}

fn export_environments(
    root: &Path,
    original_root: Option<&Path>,
    dependencies: &mut Vec<KernelExtensionDependency>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<(), DaemonError> {
    let registry = crate::script::CharioxEnvironmentRegistry::new(vec![root.to_path_buf()]);
    for environment in registry.list()? {
        crate::mcp::validate_registry_name(&environment.name, "environment name")?;
        let runtime = export_portable_environment(root, original_root, &environment)?;
        push_dependency(
            dependencies,
            budget,
            KernelExtensionDependency::Environment {
                name: environment.name,
                runtime,
            },
        )?;
    }
    Ok(())
}

fn export_portable_environment(
    registry_root: &Path,
    original_registry_root: Option<&Path>,
    environment: &crate::script::CharioxEnvironmentConfig,
) -> Result<PortableEnvironmentRuntime, DaemonError> {
    crate::mcp::validate_registry_name(&environment.name, "environment name")?;
    let package_root = registry_root.join(".portable").join(&environment.name);
    validate_captured_directory(registry_root, &package_root, "portable environment package")?;
    let manifest_bytes = read_bounded_regular_file(&package_root.join("manifest.json"), 64 * 1024)
        .map_err(|_| {
            kernel_context_error(format!(
                "environment `{}` has no portable runtime manifest",
                environment.name
            ))
        })?;
    let manifest = serde_json::from_slice::<PortableEnvironmentManifest>(&manifest_bytes).map_err(
        |error| {
            kernel_context_error(format!(
                "environment `{}` portable manifest is invalid: {error}",
                environment.name
            ))
        },
    )?;
    if manifest.schema_version != 1 || !is_exact_runtime_version(&manifest.version) {
        return Err(kernel_context_error(format!(
            "environment `{}` portable runtime version is invalid",
            environment.name
        )));
    }
    match (&environment.runtime, manifest.runtime) {
        (CharioxEnvironmentRuntime::Python { python }, PortableEnvironmentKind::Python) => {
            if !is_standard_runtime_path(
                python,
                &["python", "python3", "python.exe", "python3.exe"],
            ) {
                return Err(kernel_context_error(format!(
                    "environment `{}` does not have a reproducible Python runtime and requirements lock",
                    environment.name
                )));
            }
            let files = package_declared_environment_files(
                &package_root,
                &["manifest.json", "requirements.lock"],
            )?;
            validate_python_requirements_lock(&files)?;
            Ok(PortableEnvironmentRuntime::Python {
                version: manifest.version,
                files,
            })
        }
        (
            CharioxEnvironmentRuntime::Node {
                node,
                package_root: configured_package_root,
            },
            PortableEnvironmentKind::Node,
        ) => {
            let expected_original_root =
                original_registry_root.map(|root| root.join(".portable").join(&environment.name));
            let configured_root_matches =
                configured_package_root.as_ref().is_some_and(|configured| {
                    expected_original_root
                        .as_ref()
                        .is_some_and(|expected| same_lexical_path(configured, expected))
                });
            if !is_standard_runtime_path(node, &["node", "node.exe"]) || !configured_root_matches {
                return Err(kernel_context_error(format!(
                    "environment `{}` does not have a reproducible Node runtime and package lock",
                    environment.name
                )));
            }
            let files = package_declared_environment_files(
                &package_root,
                &["manifest.json", "package.json", "package-lock.json"],
            )?;
            validate_node_package_files(&files)?;
            Ok(PortableEnvironmentRuntime::Node {
                version: manifest.version,
                files,
            })
        }
        _ => Err(kernel_context_error(format!(
            "environment `{}` portable manifest runtime does not match its registry definition",
            environment.name
        ))),
    }
}

fn package_declared_environment_files(
    root: &Path,
    names: &[&str],
) -> Result<Vec<KernelPackageFile>, DaemonError> {
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        if !crate::managed_context::portable_path::is_portable_relative_path(name) {
            return Err(kernel_context_error(
                "portable environment file name is invalid",
            ));
        }
        let (bytes, metadata) =
            read_bounded_regular_file_with_metadata(&root.join(name), MAX_FILE_BYTES)?;
        files.push(KernelPackageFile {
            path: (*name).to_string(),
            sha256: sha256_hex(&bytes),
            size_bytes: bytes.len() as u64,
            executable: is_executable(&metadata),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn is_exact_runtime_version(version: &str) -> bool {
    version.len() <= 64
        && version.split('.').count() >= 2
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_standard_runtime_path(path: &Path, allowed_names: &[&str]) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !allowed_names.iter().any(|allowed| *allowed == file_name) {
        return false;
    }
    if path.components().count() == 1 {
        return true;
    }
    matches!(
        path.parent().and_then(Path::to_str),
        Some("/usr/bin" | "/usr/local/bin" | "/opt/homebrew/bin")
    )
}

fn same_lexical_path(left: &Path, right: &Path) -> bool {
    normalize_lexical_path(left) == normalize_lexical_path(right)
}

fn validate_captured_file(
    root: &Path,
    candidate: &Path,
    label: &'static str,
) -> Result<PathBuf, DaemonError> {
    validate_captured_path(root, candidate, label, false)
}

fn validate_captured_directory(
    root: &Path,
    candidate: &Path,
    label: &'static str,
) -> Result<PathBuf, DaemonError> {
    validate_captured_path(root, candidate, label, true)
}

fn validate_captured_path(
    root: &Path,
    candidate: &Path,
    label: &'static str,
    expect_directory: bool,
) -> Result<PathBuf, DaemonError> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| kernel_context_error(format!("{label} escaped its captured registry")))?;
    let relative = portable_relative_path(relative)?;
    if !crate::managed_context::portable_path::is_portable_relative_path(&relative) {
        return Err(kernel_context_error(format!(
            "{label} is not a portable relative path"
        )));
    }
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|error| kernel_context_io_error("inspect captured registry path", error))?;
    if metadata.file_type().is_symlink()
        || expect_directory != metadata.is_dir()
        || (!expect_directory && !metadata.is_file())
    {
        return Err(kernel_context_error(format!(
            "{label} is not a captured {}",
            if expect_directory {
                "directory"
            } else {
                "file"
            }
        )));
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| kernel_context_io_error("resolve captured registry root", error))?;
    let canonical_candidate = fs::canonicalize(candidate)
        .map_err(|error| kernel_context_io_error("resolve captured registry path", error))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(kernel_context_error(format!(
            "{label} escaped its captured registry"
        )));
    }
    Ok(canonical_candidate)
}

fn normalize_lexical_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str())
            }
            Component::CurDir => {}
            Component::ParentDir => return None,
        }
    }
    Some(normalized)
}

fn validate_python_requirements_lock(files: &[KernelPackageFile]) -> Result<(), DaemonError> {
    let bytes = decoded_package_file(files, "requirements.lock")?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| kernel_context_error("Python requirements lock is not UTF-8"))?;
    let logical_lines = text.replace("\\\n", " ");
    for line in logical_lines.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line == "--require-hashes" {
            continue;
        }
        let mut parts = line.split_whitespace();
        let requirement = parts.next().unwrap_or_default();
        let hashes = parts.collect::<Vec<_>>();
        if line.starts_with('-')
            || requirement.contains("file:")
            || requirement.contains("git+")
            || requirement.contains("@")
            || !requirement.contains("==")
            || hashes.is_empty()
            || hashes
                .iter()
                .any(|value| !value.starts_with("--hash=sha256:"))
        {
            return Err(kernel_context_error(
                "Python requirements lock must contain only hash-pinned packages",
            ));
        }
    }
    Ok(())
}

fn validate_node_package_files(files: &[KernelPackageFile]) -> Result<(), DaemonError> {
    let package =
        serde_json::from_slice::<serde_json::Value>(&decoded_package_file(files, "package.json")?)
            .map_err(|error| {
                kernel_context_error(format!("Node package.json is invalid: {error}"))
            })?;
    let lock = serde_json::from_slice::<serde_json::Value>(&decoded_package_file(
        files,
        "package-lock.json",
    )?)
    .map_err(|error| kernel_context_error(format!("Node package lock is invalid: {error}")))?;
    if lock
        .get("lockfileVersion")
        .and_then(serde_json::Value::as_u64)
        .is_none_or(|version| version < 2)
    {
        return Err(kernel_context_error(
            "Node package lock must use lockfileVersion 2 or newer",
        ));
    }
    let packages = lock
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| kernel_context_error("Node package lock has no packages map"))?;
    for (path, entry) in packages {
        if path.is_empty() {
            continue;
        }
        let integrity = entry
            .get("integrity")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let resolved = entry
            .get("resolved")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !(integrity.starts_with("sha512-") || integrity.starts_with("sha256-"))
            || !url::Url::parse(resolved).is_ok_and(|url| url.scheme() == "https")
            || url_contains_credentials(resolved)
        {
            return Err(kernel_context_error(format!(
                "Node package lock entry `{path}` is not integrity-pinned to HTTPS"
            )));
        }
    }
    reject_secret_package_metadata(&package)?;
    reject_secret_package_metadata(&lock)
}

fn decoded_package_file(files: &[KernelPackageFile], path: &str) -> Result<Vec<u8>, DaemonError> {
    let file = files
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| kernel_context_error(format!("package file `{path}` is missing")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.content_base64)
        .map_err(|error| {
            kernel_context_error(format!(
                "package file `{path}` is not valid base64: {error}"
            ))
        })?;
    if bytes.len() as u64 != file.size_bytes || sha256_hex(&bytes) != file.sha256 {
        return Err(kernel_context_error(format!(
            "package file `{path}` does not match its declared digest"
        )));
    }
    Ok(bytes)
}

fn reject_secret_package_metadata(value: &serde_json::Value) -> Result<(), DaemonError> {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                if name == "scripts"
                    || name == "hasInstallScript" && value.as_bool().unwrap_or(false)
                {
                    return Err(kernel_context_error(
                        "Node package metadata contains lifecycle scripts",
                    ));
                }
                if secret_package_metadata_key(name, value) {
                    return Err(kernel_context_error(
                        "Node package metadata contains credential-like fields",
                    ));
                }
                reject_secret_package_metadata(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_secret_package_metadata(value)?;
            }
        }
        serde_json::Value::String(value)
            if url_contains_credentials(value)
                || value.starts_with("file:")
                || value.starts_with("link:")
                || value.starts_with("workspace:")
                || value.starts_with("git+file:") =>
        {
            return Err(kernel_context_error(
                "Node package metadata contains a nonportable or credential-bearing dependency",
            ))
        }
        _ => {}
    }
    Ok(())
}

fn secret_package_metadata_key(name: &str, value: &serde_json::Value) -> bool {
    sensitive_json_credential_field(name, value)
        || name.trim().eq_ignore_ascii_case("username") && !value.is_null()
}

fn url_contains_credentials(value: &str) -> bool {
    let parsed = url::Url::parse(value).or_else(|_| {
        url::Url::parse("https://portable.invalid/")
            .expect("static URL is valid")
            .join(value)
    });
    parsed.is_ok_and(|url| {
        !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url
                .query_pairs()
                .any(|(name, _)| sensitive_credential_field(&name))
    })
}

fn export_connector_adapters(
    user_root: &Path,
    bundled_roots: Vec<PathBuf>,
    dependencies: &mut Vec<KernelExtensionDependency>,
    budget: &mut SnapshotMemoryBudget,
    referenced_adapters: &BTreeSet<String>,
) -> Result<(), DaemonError> {
    let registry = crate::connector::CharioxConnectorAdapterRegistry::new(
        user_root.to_path_buf(),
        bundled_roots,
    );
    for name in referenced_adapters {
        let adapter = registry.get(name)?.ok_or_else(|| {
            kernel_context_error(format!(
                "connector adapter `{name}` disappeared during export"
            ))
        })?;
        match adapter.source {
            Some(ConnectorAdapterSource::User) => {
                validate_portable_user_adapter(&adapter)?;
                let manifest = adapter.manifest_path.as_ref().ok_or_else(|| {
                    kernel_context_error(format!(
                        "user connector adapter `{}` has no manifest path",
                        adapter.name
                    ))
                })?;
                let root = manifest.parent().ok_or_else(|| {
                    kernel_context_error("connector adapter manifest has no parent")
                })?;
                let files = package_directory(root)?;
                push_dependency(
                    dependencies,
                    budget,
                    KernelExtensionDependency::UserConnectorAdapter {
                        name: adapter.name,
                        definition_sha256: package_definition_hash(&files),
                        files,
                    },
                )?;
            }
            Some(ConnectorAdapterSource::Bundled) => {
                let artifact_sha256 = bundled_adapter_artifact_hash(&adapter)?;
                push_dependency(
                    dependencies,
                    budget,
                    KernelExtensionDependency::BundledConnectorAdapter {
                        name: adapter.name,
                        version: adapter.version,
                        adapter_protocol: adapter.adapter_protocol,
                        artifact_sha256,
                    },
                )?;
            }
            None => {
                return Err(kernel_context_error(
                    "connector adapter source is not identified",
                ))
            }
        }
    }
    Ok(())
}

fn export_credentials(
    root: &Path,
    dependencies: &mut Vec<KernelExtensionDependency>,
    budget: &mut SnapshotMemoryBudget,
) -> Result<(), DaemonError> {
    let registry = crate::credential::CharioxCredentialRegistry::new(root.to_path_buf());
    for mut credential in registry.list()? {
        if !matches!(credential.source, UserCredentialSourceConfig::Vault { .. }) {
            return Err(kernel_context_error(format!(
                "credential `{}` uses a nonportable env or file source",
                credential.id
            )));
        }
        validate_portable_credential_injection(&credential)?;
        if let Some(metadata) = &mut credential.metadata {
            metadata.created_by_kind = None;
            metadata.created_by_id = None;
            metadata.session_id = None;
            metadata.provider_run_id = None;
        }
        push_dependency(
            dependencies,
            budget,
            KernelExtensionDependency::Credential { credential },
        )?;
    }
    Ok(())
}

fn validate_portable_credential_injection(
    credential: &crate::config::UserCredentialConfig,
) -> Result<(), DaemonError> {
    let UserCredentialInjectionConfig::Header { value, .. } = &credential.injection else {
        return Ok(());
    };
    const PLACEHOLDER: &str = "${secret}";
    let placeholder_count = value.match_indices(PLACEHOLDER).count();
    let prefix = value.strip_suffix(PLACEHOLDER);
    if placeholder_count != 1
        || !matches!(
            prefix,
            Some("" | "Bearer " | "Token " | "ApiKey " | "Key " | "SSWS ")
        )
    {
        return Err(kernel_context_error(format!(
            "credential `{}` header injection must use one supported `${{secret}}` template",
            credential.id
        )));
    }
    Ok(())
}

fn push_extension(
    extensions: &mut Vec<KernelExtensionSnapshot>,
    budget: &mut SnapshotMemoryBudget,
    extension: KernelExtensionSnapshot,
) -> Result<(), DaemonError> {
    if extensions.len() >= MAX_EXTENSIONS {
        return Err(kernel_context_error(
            "kernel Extension count exceeds its limit",
        ));
    }
    let file_count = match &extension.definition {
        KernelExtensionDefinition::Skill { package, .. } => package.files.len(),
        KernelExtensionDefinition::Script { .. } => 1,
        KernelExtensionDefinition::Mcp { .. } | KernelExtensionDefinition::Connector { .. } => 0,
    };
    budget.consume_files(file_count)?;
    budget.consume(&extension)?;
    extensions.push(extension);
    Ok(())
}

fn push_dependency(
    dependencies: &mut Vec<KernelExtensionDependency>,
    budget: &mut SnapshotMemoryBudget,
    dependency: KernelExtensionDependency,
) -> Result<(), DaemonError> {
    if dependencies.len() >= MAX_DEPENDENCIES {
        return Err(kernel_context_error(
            "kernel Extension dependency count exceeds its limit",
        ));
    }
    match &dependency {
        KernelExtensionDependency::Environment {
            runtime:
                PortableEnvironmentRuntime::Python { files, .. }
                | PortableEnvironmentRuntime::Node { files, .. },
            ..
        }
        | KernelExtensionDependency::UserConnectorAdapter { files, .. } => {
            budget.consume_files(files.len())?
        }
        KernelExtensionDependency::BundledConnectorAdapter { .. }
        | KernelExtensionDependency::Credential { .. } => {}
    }
    budget.consume(&dependency)?;
    dependencies.push(dependency);
    Ok(())
}

fn bundled_adapter_artifact_hash(
    adapter: &CharioxConnectorAdapterDefinition,
) -> Result<String, DaemonError> {
    validate_portable_bundled_adapter(adapter)?;
    let manifest = adapter.manifest_path.as_ref().ok_or_else(|| {
        kernel_context_error(format!(
            "bundled connector adapter `{}` has no manifest path",
            adapter.name
        ))
    })?;
    let root = manifest
        .parent()
        .ok_or_else(|| kernel_context_error("bundled adapter manifest has no parent"))?;
    let files = package_directory(root)?;
    let package_sha256 = package_definition_hash(&files);
    let portable_definition = (
        adapter.kind.as_str(),
        adapter.name.as_str(),
        adapter.version.as_deref(),
        adapter.adapter_protocol.as_str(),
        &adapter.command,
        &adapter.args,
        adapter.description.as_deref(),
        package_sha256,
    );
    let bytes = serde_json::to_vec(&portable_definition).map_err(|error| {
        kernel_context_error(format!(
            "serialize bundled connector adapter `{}`: {error}",
            adapter.name
        ))
    })?;
    Ok(sha256_hex(&bytes))
}

fn validate_portable_bundled_adapter(
    adapter: &CharioxConnectorAdapterDefinition,
) -> Result<(), DaemonError> {
    if adapter.command.is_absolute()
        || adapter.command.components().count() < 2
        || adapter
            .command
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        || adapter
            .args
            .iter()
            .any(|argument| argument_has_host_path(argument))
    {
        return Err(kernel_context_error(format!(
            "bundled connector adapter `{}` is not bound to a portable packaged command",
            adapter.name
        )));
    }
    Ok(())
}

fn extension_definition_hash(
    definition: &KernelExtensionDefinition,
) -> Result<String, DaemonError> {
    serde_json::to_vec(definition)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| {
            kernel_context_error(format!("serialize kernel Extension definition: {error}"))
        })
}

fn skill_executable_paths(
    skill_root: &Path,
    package: &crate::skill::CharioxSkillPackage,
) -> Result<Vec<String>, DaemonError> {
    validate_portable_package_paths(package.files.iter().map(|file| file.path.as_str()))?;
    let mut executable_paths = Vec::new();
    for packaged_file in &package.files {
        let relative = Path::new(&packaged_file.path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(kernel_context_error(
                "skill package contains a nonportable path",
            ));
        }
        let (bytes, metadata) =
            read_bounded_regular_file_with_metadata(&skill_root.join(relative), MAX_FILE_BYTES)?;
        if sha256_hex(&bytes) != packaged_file.sha256 {
            return Err(kernel_context_error(format!(
                "skill package file `{}` changed during export",
                packaged_file.path
            )));
        }
        validate_safe_package_file(&packaged_file.path, &bytes)?;
        if is_executable(&metadata) {
            executable_paths.push(packaged_file.path.clone());
        }
    }
    executable_paths.sort();
    Ok(executable_paths)
}

fn validate_portable_mcp(config: &CharioxMcpServerConfig) -> Result<(), DaemonError> {
    match &config.transport {
        CharioxMcpTransportConfig::Stdio { .. } => {
            return Err(kernel_context_error(format!(
                "stdio MCP `{}` has no signed portable runtime artifact",
                config.name
            )))
        }
        CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            ..
        } => {
            let parsed = url::Url::parse(url).map_err(|error| {
                kernel_context_error(format!("MCP `{}` URL is invalid: {error}", config.name))
            })?;
            if !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
                || parsed
                    .query_pairs()
                    .any(|(name, _)| sensitive_credential_field(&name))
                || bearer_token_env_var.is_some()
                || !env_http_headers.is_empty()
                || http_headers.keys().any(|name| sensitive_http_header(name))
            {
                return Err(kernel_context_error(format!(
                    "MCP `{}` contains literal or ambient credentials; use credential bindings",
                    config.name
                )));
            }
        }
    }
    Ok(())
}

fn validate_portable_user_adapter(
    adapter: &CharioxConnectorAdapterDefinition,
) -> Result<(), DaemonError> {
    if adapter.command.is_absolute() || adapter.command.components().count() < 2 {
        return Err(kernel_context_error(format!(
            "user connector adapter `{}` must use a packaged relative command",
            adapter.name
        )));
    }
    if adapter
        .command
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(kernel_context_error(format!(
            "user connector adapter `{}` command escapes its package",
            adapter.name
        )));
    }
    if adapter
        .args
        .iter()
        .any(|argument| argument_has_host_path(argument))
    {
        return Err(kernel_context_error(format!(
            "user connector adapter `{}` uses a source-host path argument",
            adapter.name
        )));
    }
    Ok(())
}

fn sensitive_credential_field(name: &str) -> bool {
    let normalized = normalized_credential_field(name);
    if matches!(
        normalized.as_str(),
        "continuationtoken" | "tokenbudget" | "secretenabled"
    ) {
        return false;
    }
    if normalized == "sig" || auth_credential_field(&normalized) {
        return true;
    }
    [
        "authorization",
        "cookie",
        "credential",
        "credentials",
        "password",
        "passwd",
        "token",
        "secret",
        "apikey",
        "accesskey",
        "accesskeyid",
        "privatekey",
        "secretkey",
        "signingkey",
        "signature",
        "passphrase",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

fn normalized_credential_field(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn auth_credential_field(normalized: &str) -> bool {
    normalized.ends_with("auth") || normalized.ends_with("authentication")
}

fn sensitive_json_credential_field(name: &str, value: &serde_json::Value) -> bool {
    let normalized = normalized_credential_field(name);
    if auth_credential_field(&normalized) {
        return auth_json_value_is_credential(value);
    }
    sensitive_credential_field(name) && !value.is_null()
}

fn auth_json_value_is_credential(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => !auth_string_is_portable_metadata(value),
        serde_json::Value::Number(_) => true,
        serde_json::Value::Array(values) => values.iter().any(auth_json_value_is_credential),
        serde_json::Value::Object(fields) => fields
            .iter()
            .any(|(name, value)| auth_object_field_is_credential(name, value)),
        serde_json::Value::Bool(_) | serde_json::Value::Null => false,
    }
}

fn auth_object_field_is_credential(name: &str, value: &serde_json::Value) -> bool {
    let normalized = normalized_credential_field(name);
    if matches!(normalized.as_str(), "mode" | "scheme" | "type" | "method") {
        return !value.as_str().is_some_and(auth_string_is_portable_metadata);
    }
    if matches!(normalized.as_str(), "enabled" | "required" | "optional") {
        return !matches!(value, serde_json::Value::Bool(_) | serde_json::Value::Null);
    }
    if matches!(
        normalized.as_str(),
        "credentialid" | "credentialref" | "bindingid" | "bindingref"
    ) {
        return !value.as_str().is_some_and(|value| {
            crate::mcp::validate_registry_name(value, "credential reference").is_ok()
        });
    }
    if matches!(
        normalized.as_str(),
        "url" | "endpoint" | "issuer" | "audience"
    ) {
        return !value.as_str().is_some_and(safe_auth_url);
    }
    if sensitive_json_credential_field(name, value) {
        return true;
    }
    auth_json_value_is_credential(value)
}

fn auth_string_is_portable_metadata(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "none"
            | "disabled"
            | "optional"
            | "required"
            | "basic"
            | "bearer"
            | "digest"
            | "oauth"
            | "oauth2"
            | "ntlm"
            | "sigv4"
    ) || safe_auth_url(value)
}

fn safe_auth_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && !url
                .query_pairs()
                .any(|(name, _)| sensitive_credential_field(&name))
    })
}

fn sensitive_http_header(name: &str) -> bool {
    sensitive_credential_field(name)
        || matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "x-api-key" | "x-auth-token" | "x-access-token" | "set-cookie"
        )
}

fn argument_has_host_path(argument: &str) -> bool {
    let candidate = argument
        .split_once('=')
        .map(|(_, value)| value)
        .unwrap_or(argument)
        .trim_matches(['\'', '"']);
    Path::new(candidate).is_absolute()
        || candidate.starts_with("~/")
        || candidate.starts_with("~\\")
        || candidate.starts_with("\\\\")
        || candidate
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':')
        || candidate
            .split(['/', '\\'])
            .any(|component| component == "..")
}

fn package_directory(root: &Path) -> Result<Vec<KernelPackageFile>, DaemonError> {
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| kernel_context_io_error("inspect package root", error))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(kernel_context_error(
            "kernel Extension package root must be a directory without links",
        ));
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| kernel_context_io_error("resolve package root", error))?;
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut entry_count = 0_usize;
    collect_package_files(
        &canonical_root,
        &canonical_root,
        &mut files,
        &mut total_bytes,
        &mut entry_count,
        0,
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    validate_portable_package_paths(files.iter().map(|file| file.path.as_str()))?;
    Ok(files)
}

fn collect_package_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<KernelPackageFile>,
    total_bytes: &mut u64,
    entry_count: &mut usize,
    depth: usize,
) -> Result<(), DaemonError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(kernel_context_error(
            "kernel Extension package exceeds its directory depth limit",
        ));
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| kernel_context_io_error("enumerate package", error))?
    {
        let path = entry
            .map_err(|error| kernel_context_io_error("enumerate package", error))?
            .path();
        *entry_count = entry_count.saturating_add(1);
        if *entry_count > MAX_PACKAGE_ENTRIES {
            return Err(kernel_context_error(
                "kernel Extension package entry count exceeds its limit",
            ));
        }
        paths.push(path);
    }
    paths.sort();
    for path in paths {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| kernel_context_io_error("inspect package path", error))?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            return Err(kernel_context_error(
                "kernel Extension package contains a symlink or special file",
            ));
        }
        if metadata.is_dir() {
            collect_package_files(root, &path, files, total_bytes, entry_count, depth + 1)?;
            continue;
        }
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(kernel_context_error(
                "kernel Extension package file count exceeds its limit",
            ));
        }
        let (bytes, opened_metadata) =
            read_bounded_regular_file_with_metadata(&path, MAX_FILE_BYTES)?;
        *total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if *total_bytes > MAX_PACKAGE_BYTES {
            return Err(kernel_context_error(
                "kernel Extension package bytes exceed their limit",
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| kernel_context_error("package path escaped its root"))?;
        let relative = portable_relative_path(relative)?;
        validate_safe_package_file(&relative, &bytes)?;
        files.push(KernelPackageFile {
            path: relative,
            sha256: sha256_hex(&bytes),
            size_bytes: bytes.len() as u64,
            executable: is_executable(&opened_metadata),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(())
}

fn portable_relative_path(path: &Path) -> Result<String, DaemonError> {
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(kernel_context_error(
                "kernel Extension package path is not relative",
            ));
        };
        components.push(
            component
                .to_str()
                .ok_or_else(|| kernel_context_error("kernel Extension package path is not UTF-8"))?
                .to_string(),
        );
    }
    let portable = components.join("/");
    if !crate::managed_context::portable_path::is_portable_relative_path(&portable) {
        return Err(kernel_context_error(format!(
            "kernel Extension package path `{portable}` is not portable"
        )));
    }
    Ok(portable)
}

fn validate_portable_package_paths<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), DaemonError> {
    let mut aliases = BTreeSet::new();
    for path in paths {
        let alias = crate::managed_context::portable_path::portable_path_alias_key(path)
            .ok_or_else(|| {
                kernel_context_error(format!(
                    "kernel Extension package path `{path}` is not portable"
                ))
            })?;
        if sensitive_extension_package_path(path) {
            return Err(kernel_context_error(format!(
                "kernel Extension package path `{path}` may contain credentials"
            )));
        }
        if !aliases.insert(alias) {
            return Err(kernel_context_error(format!(
                "kernel Extension package contains colliding path `{path}`"
            )));
        }
    }
    Ok(())
}

fn sensitive_extension_package_path(path: &str) -> bool {
    path.split('/').any(|component| {
        let component = component.to_ascii_lowercase();
        component.starts_with(".env")
            || matches!(
                component.as_str(),
                ".npmrc"
                    | ".pypirc"
                    | ".netrc"
                    | ".git-credentials"
                    | ".ssh"
                    | "id_rsa"
                    | "id_ed25519"
                    | "credentials.json"
                    | "credentials.yaml"
                    | "credentials.yml"
                    | "secrets.json"
                    | "secrets.yaml"
                    | "secrets.yml"
            )
            || component.ends_with(".pem")
            || component.ends_with(".p12")
            || component.ends_with(".pfx")
            || component.ends_with(".key")
    })
}

fn validate_safe_package_file(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    if sensitive_extension_package_path(path)
        || bytes
            .windows(b"PRIVATE KEY-----".len())
            .any(|window| window == b"PRIVATE KEY-----")
    {
        return Err(kernel_context_error(format!(
            "kernel Extension package file `{path}` may contain credentials"
        )));
    }
    Ok(())
}

fn reject_literal_secrets_in_json(value: &serde_json::Value) -> Result<(), DaemonError> {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                if sensitive_json_credential_field(name, value) {
                    return Err(kernel_context_error(
                        "connector configuration contains literal credential-like fields",
                    ));
                }
                reject_literal_secrets_in_json(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_literal_secrets_in_json(value)?;
            }
        }
        serde_json::Value::String(value) if url_contains_credentials(value) => {
            return Err(kernel_context_error(
                "connector configuration contains a credential-bearing URL",
            ))
        }
        _ => {}
    }
    Ok(())
}

fn read_bounded_regular_file(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>, DaemonError> {
    read_bounded_regular_file_with_metadata(path, maximum_bytes).map(|(bytes, _)| bytes)
}

fn read_bounded_regular_file_with_metadata(
    path: &Path,
    maximum_bytes: u64,
) -> Result<(Vec<u8>, fs::Metadata), DaemonError> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| kernel_context_io_error("inspect kernel context file", error))?;
    if path_metadata.file_type().is_symlink()
        || !path_metadata.is_file()
        || path_metadata.len() > maximum_bytes
    {
        return Err(kernel_context_error(
            "kernel context file must be a bounded regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| kernel_context_io_error("open kernel context file", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| kernel_context_io_error("inspect opened kernel context file", error))?;
    if !metadata.is_file()
        || metadata.len() > maximum_bytes
        || opened_metadata_is_reparse_point(&metadata)
    {
        return Err(kernel_context_error(
            "kernel context file must remain a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| kernel_context_io_error("read kernel context file", error))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(kernel_context_error(
            "kernel context file exceeds its size limit",
        ));
    }
    Ok((bytes, metadata))
}

#[cfg(windows)]
fn opened_metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn opened_metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn package_definition_hash(files: &[KernelPackageFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update(file.sha256.as_bytes());
        hasher.update([0]);
        hasher.update(file.size_bytes.to_le_bytes());
        hasher.update([u8::from(file.executable)]);
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn validate_identifier(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(kernel_context_error(format!(
            "kernel context {label} is invalid"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(kernel_context_error(format!(
            "kernel context {label} is invalid"
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn kernel_context_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_kernel_context",
        operation: "kernel_context.export",
        message: message.into(),
        retryable: false,
    }
}

fn kernel_context_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "kernel_context_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests;
