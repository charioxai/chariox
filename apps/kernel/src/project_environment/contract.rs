use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::policy::{
    duplicate, enforce_count, normalize_digest, normalize_relative_path, normalize_text,
    ProjectEnvironmentError, CONTRACT_SCHEMA_VERSION, MANIFEST_SCHEMA_VERSION, MAX_LOCKFILES,
    MAX_MANIFEST_BYTES, MAX_MANIFEST_ENTRIES, MAX_STRING_BYTES, MAX_TOOLCHAINS,
    MAX_TOOLCHAIN_COMPONENTS, MAX_UTILITIES, MAX_VALIDATION_PROBES, POLICY_VERSION,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryManifest {
    pub schema_version: u16,
    #[serde(default)]
    pub utilities: Vec<UtilitySpec>,
    #[serde(default)]
    pub toolchains: Vec<ToolchainInput>,
    #[serde(default)]
    pub lockfiles: Vec<String>,
    #[serde(default)]
    pub validation: Vec<ValidationProbe>,
}

impl RepositoryManifest {
    pub fn from_toml(input: &str) -> Result<Self, ProjectEnvironmentError> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(ProjectEnvironmentError::BoundExceeded {
                kind: "manifest bytes",
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let manifest = toml::from_str::<Self>(input).map_err(|error| {
            ProjectEnvironmentError::MalformedManifest {
                message: error.to_string(),
            }
        })?;
        manifest.validate_and_normalize()
    }

    pub(crate) fn from_toml_bytes(bytes: &[u8]) -> Result<Self, ProjectEnvironmentError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ProjectEnvironmentError::BoundExceeded {
                kind: "manifest bytes",
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let input = std::str::from_utf8(bytes).map_err(|error| {
            ProjectEnvironmentError::MalformedManifest {
                message: format!("manifest is not UTF-8: {error}"),
            }
        })?;
        Self::from_toml(input)
    }

    pub fn validate_and_normalize(self) -> Result<Self, ProjectEnvironmentError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ProjectEnvironmentError::UnsupportedManifestVersion(
                self.schema_version,
            ));
        }
        enforce_count("utility entries", self.utilities.len(), MAX_UTILITIES)?;
        enforce_count("toolchain entries", self.toolchains.len(), MAX_TOOLCHAINS)?;
        enforce_count("lockfile entries", self.lockfiles.len(), MAX_LOCKFILES)?;
        enforce_count(
            "validation probe entries",
            self.validation.len(),
            MAX_VALIDATION_PROBES,
        )?;
        let total_entries = self
            .utilities
            .len()
            .saturating_add(self.toolchains.len())
            .saturating_add(self.lockfiles.len())
            .saturating_add(self.validation.len());
        enforce_count("manifest entries", total_entries, MAX_MANIFEST_ENTRIES)?;

        let mut utilities = Vec::with_capacity(self.utilities.len());
        let mut utility_names = BTreeSet::new();
        for (index, utility) in self.utilities.into_iter().enumerate() {
            let name = utility.name;
            if !utility_names.insert(name) {
                return duplicate(format!("utility {}", name.as_str()));
            }
            utilities.push(UtilitySpec {
                name,
                version: normalize_text(
                    &format!("utilities[{index}].version"),
                    &utility.version,
                    MAX_STRING_BYTES,
                )?,
                required: utility.required,
            });
        }
        utilities.sort_by_key(|utility| utility.name);

        let mut toolchains = Vec::with_capacity(self.toolchains.len());
        let mut toolchain_names = BTreeSet::new();
        for (index, toolchain) in self.toolchains.into_iter().enumerate() {
            let normalized = normalize_toolchain(&format!("toolchains[{index}]"), toolchain)?;
            if !toolchain_names.insert(normalized.name) {
                return duplicate(format!("toolchain {}", normalized.name.as_str()));
            }
            toolchains.push(normalized);
        }
        toolchains.sort_by_key(|toolchain| toolchain.name);

        let mut lockfiles = Vec::with_capacity(self.lockfiles.len());
        let mut lockfile_paths = BTreeSet::new();
        for (index, path) in self.lockfiles.into_iter().enumerate() {
            let path = normalize_relative_path(&format!("lockfiles[{index}]"), &path)?;
            if !lockfile_paths.insert(path.clone()) {
                return duplicate(format!("lockfile {path}"));
            }
            lockfiles.push(path);
        }
        lockfiles.sort();

        let mut validation = Vec::with_capacity(self.validation.len());
        let mut probe_names = BTreeSet::new();
        let mut probe_paths = BTreeSet::new();
        for (index, probe) in self.validation.into_iter().enumerate() {
            let normalized = normalize_probe(&format!("validation[{index}]"), probe)?;
            if !probe_names.insert(normalized.name.clone()) {
                return duplicate(format!("validation probe {}", normalized.name));
            }
            if !probe_paths.insert(normalized.path.clone()) {
                return duplicate(format!("validation path {}", normalized.path));
            }
            validation.push(normalized);
        }
        validation.sort_by(|left, right| left.name.cmp(&right.name));

        for probe in &validation {
            if probe.kind == ValidationProbeKind::LockfileDigest
                && !lockfiles.iter().any(|path| path == &probe.path)
            {
                return Err(ProjectEnvironmentError::InvalidField {
                    field: format!("validation.{}.path", probe.name),
                    message: "lockfile_digest probes must name a declared lockfile".to_owned(),
                });
            }
        }

        Ok(Self {
            schema_version: self.schema_version,
            utilities,
            toolchains,
            lockfiles,
            validation,
        })
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_manifest_bytes(self)
    }

    pub fn digest(&self) -> String {
        hex_digest(&Sha256::digest(self.canonical_bytes()))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtilityKind {
    Cargo,
    Git,
    Node,
    Npm,
    Pnpm,
    Python,
    Rustc,
}

impl UtilityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Git => "git",
            Self::Node => "node",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Python => "python",
            Self::Rustc => "rustc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtilitySpec {
    pub name: UtilityKind,
    pub version: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainKind {
    Node,
    Python,
    Rust,
}

impl ToolchainKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
            Self::Rust => "rust",
        }
    }
}

/// A declared toolchain or a resolved toolchain input. It is intentionally
/// descriptive only: no path, command, environment mutation, or installer is
/// represented by this schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainInput {
    pub name: ToolchainKind,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub components: Vec<String>,
}

pub type ToolchainSpec = ToolchainInput;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationProbeKind {
    FileDigest,
    LockfileDigest,
    PathExists,
}

impl ValidationProbeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileDigest => "file_digest",
            Self::LockfileDigest => "lockfile_digest",
            Self::PathExists => "path_exists",
        }
    }
}

/// A validation observation to be evaluated by a later slice. This is data,
/// never a command or a script invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationProbe {
    pub name: String,
    pub kind: ValidationProbeKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetKernelRealm {
    pub kernel_id: String,
    pub realm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentIdentity {
    pub owner_id: String,
    pub project_id: String,
    pub workspace_id: String,
    pub worktree_id: String,
    pub target: TargetKernelRealm,
    pub repository_digest: String,
    pub policy_version: String,
    #[serde(default)]
    pub toolchain_inputs: Vec<ToolchainInput>,
}

impl EnvironmentIdentity {
    pub fn validate_and_normalize(self) -> Result<Self, ProjectEnvironmentError> {
        let policy_version =
            normalize_text("policy_version", &self.policy_version, MAX_STRING_BYTES)?;
        if policy_version != POLICY_VERSION {
            return Err(ProjectEnvironmentError::UnsupportedPolicyVersion(
                policy_version,
            ));
        }
        enforce_count(
            "toolchain input entries",
            self.toolchain_inputs.len(),
            MAX_TOOLCHAINS,
        )?;
        let mut toolchain_inputs = Vec::with_capacity(self.toolchain_inputs.len());
        let mut names = BTreeSet::new();
        for (index, toolchain) in self.toolchain_inputs.into_iter().enumerate() {
            let toolchain = normalize_toolchain(&format!("toolchain_inputs[{index}]"), toolchain)?;
            if !names.insert(toolchain.name) {
                return duplicate(format!("toolchain input {}", toolchain.name.as_str()));
            }
            toolchain_inputs.push(toolchain);
        }
        toolchain_inputs.sort_by_key(|toolchain| toolchain.name);

        Ok(Self {
            owner_id: normalize_text("owner_id", &self.owner_id, MAX_STRING_BYTES)?,
            project_id: normalize_text("project_id", &self.project_id, MAX_STRING_BYTES)?,
            workspace_id: normalize_text("workspace_id", &self.workspace_id, MAX_STRING_BYTES)?,
            worktree_id: normalize_text("worktree_id", &self.worktree_id, MAX_STRING_BYTES)?,
            target: TargetKernelRealm {
                kernel_id: normalize_text(
                    "target.kernel_id",
                    &self.target.kernel_id,
                    MAX_STRING_BYTES,
                )?,
                realm: normalize_text("target.realm", &self.target.realm, MAX_STRING_BYTES)?,
            },
            repository_digest: normalize_digest("repository_digest", &self.repository_digest)?,
            policy_version,
            toolchain_inputs,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockfileDigest {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentContract {
    pub schema_version: u16,
    pub identity: EnvironmentIdentity,
    pub manifest: RepositoryManifest,
    pub manifest_digest: String,
    pub lockfile_digests: Vec<LockfileDigest>,
    pub fingerprint: String,
}

impl EnvironmentContract {
    pub(crate) fn assemble(
        identity: EnvironmentIdentity,
        manifest: RepositoryManifest,
        lockfile_digests: Vec<LockfileDigest>,
    ) -> Self {
        let manifest_digest = manifest.digest();
        let fingerprint = fingerprint_for(&identity, &manifest_digest, &lockfile_digests);
        Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            identity,
            manifest,
            manifest_digest,
            lockfile_digests,
            fingerprint,
        }
    }
}

fn normalize_toolchain(
    field: &str,
    toolchain: ToolchainInput,
) -> Result<ToolchainInput, ProjectEnvironmentError> {
    enforce_count(
        "toolchain component entries",
        toolchain.components.len(),
        MAX_TOOLCHAIN_COMPONENTS,
    )?;
    let version = normalize_text(
        &format!("{field}.version"),
        &toolchain.version,
        MAX_STRING_BYTES,
    )?;
    let target = toolchain
        .target
        .as_deref()
        .map(|target| normalize_text(&format!("{field}.target"), target, MAX_STRING_BYTES))
        .transpose()?;
    let mut components = Vec::with_capacity(toolchain.components.len());
    let mut names = BTreeSet::new();
    for (index, component) in toolchain.components.into_iter().enumerate() {
        let component = normalize_text(
            &format!("{field}.components[{index}]"),
            &component,
            MAX_STRING_BYTES,
        )?;
        if !names.insert(component.clone()) {
            return duplicate(format!("{field} component {component}"));
        }
        components.push(component);
    }
    components.sort();
    Ok(ToolchainInput {
        name: toolchain.name,
        version,
        target,
        components,
    })
}

fn normalize_probe(
    field: &str,
    probe: ValidationProbe,
) -> Result<ValidationProbe, ProjectEnvironmentError> {
    let name = normalize_text(&format!("{field}.name"), &probe.name, MAX_STRING_BYTES)?;
    let path = normalize_relative_path(&format!("{field}.path"), &probe.path)?;
    let expected_digest = probe
        .expected_digest
        .as_deref()
        .map(|digest| normalize_digest(&format!("{field}.expected_digest"), digest))
        .transpose()?;
    match (probe.kind, expected_digest.is_some()) {
        (ValidationProbeKind::PathExists, true) => {
            return Err(ProjectEnvironmentError::InvalidField {
                field: format!("{field}.expected_digest"),
                message: "path_exists probes must not provide an expected digest".to_owned(),
            });
        }
        (ValidationProbeKind::FileDigest | ValidationProbeKind::LockfileDigest, false) => {
            return Err(ProjectEnvironmentError::InvalidField {
                field: format!("{field}.expected_digest"),
                message: "digest probes require an expected digest".to_owned(),
            });
        }
        _ => {}
    }
    Ok(ValidationProbe {
        name,
        kind: probe.kind,
        path,
        expected_digest,
    })
}

pub(crate) fn canonical_manifest_bytes(manifest: &RepositoryManifest) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_bytes(&mut bytes, b"chariox.project_environment.manifest.v1");
    put_u16(&mut bytes, manifest.schema_version);
    put_len(&mut bytes, manifest.utilities.len());
    for utility in &manifest.utilities {
        put_str(&mut bytes, utility.name.as_str());
        put_str(&mut bytes, &utility.version);
        put_bool(&mut bytes, utility.required);
    }
    put_len(&mut bytes, manifest.toolchains.len());
    for toolchain in &manifest.toolchains {
        put_toolchain(&mut bytes, toolchain);
    }
    put_len(&mut bytes, manifest.lockfiles.len());
    for path in &manifest.lockfiles {
        put_str(&mut bytes, path);
    }
    put_len(&mut bytes, manifest.validation.len());
    for probe in &manifest.validation {
        put_str(&mut bytes, &probe.name);
        put_str(&mut bytes, probe.kind.as_str());
        put_str(&mut bytes, &probe.path);
        match &probe.expected_digest {
            Some(digest) => {
                put_bool(&mut bytes, true);
                put_str(&mut bytes, digest);
            }
            None => put_bool(&mut bytes, false),
        }
    }
    bytes
}

pub(crate) fn fingerprint_for(
    identity: &EnvironmentIdentity,
    manifest_digest: &str,
    lockfile_digests: &[LockfileDigest],
) -> String {
    let mut bytes = Vec::new();
    put_bytes(&mut bytes, b"chariox.project_environment.fingerprint.v1");
    put_u16(&mut bytes, CONTRACT_SCHEMA_VERSION);
    put_str(&mut bytes, &identity.policy_version);
    put_str(&mut bytes, &identity.owner_id);
    put_str(&mut bytes, &identity.project_id);
    put_str(&mut bytes, &identity.workspace_id);
    put_str(&mut bytes, &identity.worktree_id);
    put_str(&mut bytes, &identity.target.kernel_id);
    put_str(&mut bytes, &identity.target.realm);
    put_str(&mut bytes, &identity.repository_digest);
    put_str(&mut bytes, manifest_digest);
    put_len(&mut bytes, lockfile_digests.len());
    for lockfile in lockfile_digests {
        put_str(&mut bytes, &lockfile.path);
        put_str(&mut bytes, &lockfile.digest);
    }
    put_len(&mut bytes, identity.toolchain_inputs.len());
    for toolchain in &identity.toolchain_inputs {
        put_toolchain(&mut bytes, toolchain);
    }
    hex_digest(&Sha256::digest(bytes))
}

fn put_toolchain(bytes: &mut Vec<u8>, toolchain: &ToolchainInput) {
    put_str(bytes, toolchain.name.as_str());
    put_str(bytes, &toolchain.version);
    match &toolchain.target {
        Some(target) => {
            put_bool(bytes, true);
            put_str(bytes, target);
        }
        None => put_bool(bytes, false),
    }
    put_len(bytes, toolchain.components.len());
    for component in &toolchain.components {
        put_str(bytes, component);
    }
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(value);
    output.push(0);
}

fn put_str(output: &mut Vec<u8>, value: &str) {
    put_len(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

fn put_len(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u64).to_be_bytes());
}

fn put_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_bool(output: &mut Vec<u8>, value: bool) {
    output.push(u8::from(value));
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
