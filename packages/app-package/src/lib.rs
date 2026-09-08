//! Offline, bounded verification for Chariox App packages. No package code is
//! executed and no archive content is written to the filesystem here.

mod archive;
mod declarations;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod developer;
mod error;
mod json;
mod manifest;

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub use declarations::*;
pub use error::{ErrorCode, PackageError, Result};
pub use manifest::*;

const INTEGRITY_SCHEMA: &str = "chariox.integrity.v1";
const SIGNATURE_SCHEMA: &str = "chariox.package-signature.v1";

/// The signed envelope is versioned independently from SDK and kernel protocol.
pub const PACKAGE_CONTRACT_VERSION: u32 = 1;
pub const SUPPORTED_SDK_VERSION: &str = "0.4.0";

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_archive_bytes: usize,
    pub max_file_bytes: usize,
    pub max_metadata_bytes: usize,
    pub max_schema_bytes: usize,
    pub max_entries: usize,
    pub max_path_bytes: usize,
    pub max_path_depth: usize,
    pub max_json_depth: usize,
    pub max_json_nodes: usize,
    pub max_declarations: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 128 * 1024 * 1024,
            max_file_bytes: 64 * 1024 * 1024,
            max_metadata_bytes: 1024 * 1024,
            max_schema_bytes: 128 * 1024,
            max_entries: 4096,
            max_path_bytes: 255,
            max_path_depth: 24,
            max_json_depth: 32,
            max_json_nodes: 32768,
            max_declarations: 256,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrustedPublisher {
    pub publisher_id: String,
    pub key_id: String,
    pub public_key: VerifyingKey,
}

/// Trust is supplied by the kernel's enrolled key store. A key or claim inside
/// a downloaded package can never add itself to this policy.
#[derive(Debug, Clone)]
pub struct VerificationPolicy {
    pub kernel_protocol: u32,
    pub app_contract_version: u32,
    pub sdk_requirement: VersionReq,
    pub resource_policies: Vec<String>,
    pub trusted_publishers: Vec<TrustedPublisher>,
    pub limits: Limits,
}

impl VerificationPolicy {
    pub fn new(kernel_protocol: u32, trusted_publishers: Vec<TrustedPublisher>) -> Self {
        Self {
            kernel_protocol,
            app_contract_version: APP_CONTRACT_VERSION,
            sdk_requirement: VersionReq::parse(&format!("={SUPPORTED_SDK_VERSION}"))
                .expect("constant SDK requirement is valid"),
            resource_policies: vec![RESOURCE_POLICY.to_owned()],
            trusted_publishers,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema: String,
    pub files: Vec<InventoryFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryFile {
    pub path: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherSignature {
    key_id: String,
    algorithm: String,
    digest: String,
    value: String,
}

/// Safe to display as a *claim* during local key enrollment. This type is never
/// accepted as proof of trust and provides no access to installable file bytes.
#[derive(Debug, Clone)]
pub struct UntrustedPackageIdentity {
    pub manifest: Manifest,
    pub package_digest: String,
}

/// File bytes become available only after envelope signature, inventory, policy,
/// declarations and every payload digest have passed verification.
#[derive(Debug)]
pub struct VerifiedPackage<'a> {
    manifest: Manifest,
    signer: TrustedPublisher,
    declarations: Declarations,
    package_digest: String,
    files: BTreeMap<String, &'a [u8]>,
}

impl<'a> VerifiedPackage<'a> {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// Exact key that verified the signature, not merely the manifest's claimed
    /// publisher/key IDs. Installers bind this provenance to enrolled trust.
    pub fn signer(&self) -> &TrustedPublisher {
        &self.signer
    }
    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    pub fn file(&self, path: &str) -> Option<&'a [u8]> {
        self.files.get(path).copied()
    }
    pub fn files(&self) -> impl Iterator<Item = (&str, &'a [u8])> + '_ {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_str(), *bytes))
    }
}

pub fn inspect_untrusted(bytes: &[u8], limits: &Limits) -> Result<UntrustedPackageIdentity> {
    let entries = archive::parse(bytes, limits)?;
    let envelope = read_envelope(&entries, limits)?;
    Ok(UntrustedPackageIdentity {
        manifest: envelope.manifest,
        package_digest: digest(bytes),
    })
}

pub fn verify<'a>(bytes: &'a [u8], policy: &VerificationPolicy) -> Result<VerifiedPackage<'a>> {
    let entries = archive::parse(bytes, &policy.limits)?;
    let envelope = read_envelope(&entries, &policy.limits)?;
    let trusted = policy
        .trusted_publishers
        .iter()
        .find(|trusted| {
            trusted.publisher_id == envelope.manifest.publisher.id
                && trusted.key_id == envelope.manifest.publisher.key_id
        })
        .ok_or_else(|| {
            PackageError::new(
                ErrorCode::UntrustedPublisher,
                "publisher signing key has not been enrolled",
            )
        })?;
    let signature_bytes = BASE64.decode(&envelope.signature.value).map_err(|_| {
        PackageError::new(
            ErrorCode::InvalidSignature,
            "signature is not standard base64",
        )
    })?;
    if BASE64.encode(&signature_bytes) != envelope.signature.value {
        return Err(PackageError::new(
            ErrorCode::InvalidSignature,
            "signature base64 is not canonical",
        ));
    }
    let signature = Signature::from_slice(&signature_bytes).map_err(|_| {
        PackageError::new(
            ErrorCode::InvalidSignature,
            "invalid Ed25519 signature length",
        )
    })?;
    let signed_bytes = signing_bytes(&envelope.manifest_value, &envelope.inventory)?;
    if digest(&signed_bytes) != envelope.signature.digest {
        return Err(PackageError::new(
            ErrorCode::IntegrityMismatch,
            "signed envelope digest mismatch",
        ));
    }
    trusted
        .public_key
        .verify_strict(&signed_bytes, &signature)
        .map_err(|_| {
            PackageError::new(
                ErrorCode::InvalidSignature,
                "publisher signature verification failed",
            )
        })?;
    check_compatibility(&envelope.manifest, policy)?;
    if envelope.inventory.files.len() != entries.len() - 3 {
        return Err(PackageError::new(
            ErrorCode::UnexpectedEntry,
            "archive and signed inventory have different file counts",
        ));
    }
    let mut files = BTreeMap::new();
    for (entry, expected) in entries[3..].iter().zip(&envelope.inventory.files) {
        if expected.path != entry.path {
            return Err(PackageError::new(
                ErrorCode::UnexpectedEntry,
                "archive path is absent from the signed inventory",
            ));
        }
        if expected.size != entry.data.len() as u64 || expected.digest != digest(entry.data) {
            return Err(PackageError::new(
                ErrorCode::IntegrityMismatch,
                "payload size or digest mismatch",
            ));
        }
        files.insert(entry.path.clone(), entry.data);
    }
    check_payload(&envelope.manifest, &files, &policy.limits)?;
    let declarations = declarations::read_declarations(&envelope.manifest, &files, &policy.limits)?;
    Ok(VerifiedPackage {
        manifest: envelope.manifest,
        signer: trusted.clone(),
        declarations,
        package_digest: digest(bytes),
        files,
    })
}

/// Build deterministic bytes. No filesystem reads, npm hooks, compiler, runtime,
/// or network activity is performed; the caller supplies a completed bundle.
pub fn pack(
    manifest: &Manifest,
    files: &BTreeMap<String, Vec<u8>>,
    key: &SigningKey,
    limits: &Limits,
) -> Result<Vec<u8>> {
    if files.len() > limits.max_entries.saturating_sub(3) {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "payload exceeds entry limit",
        ));
    }
    let mut payload_bytes = 0usize;
    for bytes in files.values() {
        payload_bytes = payload_bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= limits.max_archive_bytes)
            .ok_or_else(|| {
                PackageError::new(ErrorCode::ArchiveLimit, "payload exceeds byte limit")
            })?;
    }
    manifest.validate(limits)?;
    let borrowed: BTreeMap<_, _> = files
        .iter()
        .map(|(path, bytes)| (path.clone(), bytes.as_slice()))
        .collect();
    check_payload(manifest, &borrowed, limits)?;
    declarations::read_declarations(manifest, &borrowed, limits)?;
    let inventory = Inventory {
        schema: INTEGRITY_SCHEMA.to_owned(),
        files: files
            .iter()
            .map(|(path, bytes)| InventoryFile {
                path: path.clone(),
                size: bytes.len() as u64,
                digest: digest(bytes),
            })
            .collect(),
    };
    let manifest_value = serde_json::to_value(manifest)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "manifest encoding failed"))?;
    json::check_value(&manifest_value, limits)?;
    let signed_bytes = signing_bytes(&manifest_value, &inventory)?;
    let signature = PublisherSignature {
        key_id: manifest.publisher.key_id.clone(),
        algorithm: "ed25519".to_owned(),
        digest: digest(&signed_bytes),
        value: BASE64.encode(key.sign(&signed_bytes).to_bytes()),
    };
    let manifest_bytes = json::canonical(&manifest_value)?;
    let inventory_bytes = json::canonical(&inventory)?;
    let signature_bytes = json::canonical(&signature)?;
    for bytes in [&manifest_bytes, &inventory_bytes, &signature_bytes] {
        if bytes.len() > limits.max_metadata_bytes {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "control file exceeds byte limit",
            ));
        }
    }
    let mut entries: Vec<(&str, &[u8])> = vec![
        (archive::CONTROL_PATHS[0], &manifest_bytes),
        (archive::CONTROL_PATHS[1], &inventory_bytes),
        (archive::CONTROL_PATHS[2], &signature_bytes),
    ];
    entries.extend(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    archive::encode(&entries, limits)
}

struct Envelope {
    manifest: Manifest,
    manifest_value: Value,
    inventory: Inventory,
    signature: PublisherSignature,
}

fn read_envelope(entries: &[archive::Entry<'_>], limits: &Limits) -> Result<Envelope> {
    let manifest_value = json::read_json(
        entries[0].data,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        true,
    )?;
    let manifest: Manifest =
        serde_json::from_value(Value::clone(&manifest_value)).map_err(|_| {
            PackageError::new(
                ErrorCode::InvalidManifest,
                "manifest does not match chariox.app.v1",
            )
        })?;
    manifest.validate(limits)?;
    let inventory: Inventory = json::read_json(
        entries[1].data,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::IntegrityMismatch,
        true,
    )?;
    if inventory.schema != INTEGRITY_SCHEMA
        || inventory.files.len() > limits.max_entries.saturating_sub(3)
        || inventory
            .files
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
    {
        return Err(PackageError::new(
            ErrorCode::IntegrityMismatch,
            "invalid or unordered inventory",
        ));
    }
    for file in &inventory.files {
        archive::validate_path(&file.path, limits)?;
        if archive::CONTROL_PATHS.contains(&file.path.as_str())
            || file.path.starts_with("signatures/")
            || !valid_digest(&file.digest)
            || file.size > limits.max_file_bytes as u64
        {
            return Err(PackageError::new(
                ErrorCode::IntegrityMismatch,
                "invalid inventory entry",
            ));
        }
    }
    let signature: PublisherSignature = json::read_json(
        entries[2].data,
        limits.max_metadata_bytes.min(4096),
        limits,
        ErrorCode::InvalidSignature,
        true,
    )?;
    if signature.algorithm != "ed25519"
        || signature.key_id != manifest.publisher.key_id
        || !valid_digest(&signature.digest)
    {
        return Err(PackageError::new(
            ErrorCode::InvalidSignature,
            "invalid publisher signature envelope",
        ));
    }
    Ok(Envelope {
        manifest,
        manifest_value,
        inventory,
        signature,
    })
}

fn signing_bytes(manifest: &Value, inventory: &Inventory) -> Result<Vec<u8>> {
    json::canonical(
        &json!({ "schema": SIGNATURE_SCHEMA, "manifest": manifest, "inventory": inventory }),
    )
}

fn check_compatibility(manifest: &Manifest, policy: &VerificationPolicy) -> Result<()> {
    if manifest.min_kernel_protocol > policy.kernel_protocol {
        return Err(PackageError::new(
            ErrorCode::IncompatibleProtocol,
            "package requires a newer kernel protocol",
        ));
    }
    if manifest.app_contract_version != policy.app_contract_version {
        return Err(PackageError::new(
            ErrorCode::IncompatibleContract,
            "unsupported App contract version",
        ));
    }
    let sdk_version = Version::parse(&manifest.sdk_version)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid SDK version"))?;
    if !policy.sdk_requirement.matches(&sdk_version) {
        return Err(PackageError::new(
            ErrorCode::IncompatibleSdk,
            "unsupported SDK version",
        ));
    }
    if !policy.resource_policies.contains(&manifest.resource_policy) {
        return Err(PackageError::new(
            ErrorCode::IncompatibleResourcePolicy,
            "unsupported resource policy",
        ));
    }
    Ok(())
}

fn check_payload(
    manifest: &Manifest,
    files: &BTreeMap<String, &[u8]>,
    limits: &Limits,
) -> Result<()> {
    for path in manifest.required_paths() {
        if !files.contains_key(path) {
            return Err(PackageError::new(
                ErrorCode::MissingEntry,
                format!("declared file is missing: {path}"),
            ));
        }
    }
    for (path, bytes) in files {
        archive::validate_path(path, limits)?;
        if archive::CONTROL_PATHS.contains(&path.as_str()) || path.starts_with("signatures/") {
            return Err(PackageError::new(
                ErrorCode::UnexpectedEntry,
                "payload attempts to replace a package control file",
            ));
        }
        if bytes.len() > limits.max_file_bytes {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "file exceeds byte limit",
            ));
        }
        if [".node", ".so", ".dylib", ".dll", ".exe"]
            .iter()
            .any(|suffix| path.to_ascii_lowercase().ends_with(suffix))
        {
            return Err(PackageError::new(
                ErrorCode::UnsupportedFeature,
                "native add-ons and executable artifacts are outside the v1 package contract",
            ));
        }
        if path.starts_with("migrations/")
            && !manifest
                .migrations
                .as_ref()
                .is_some_and(|migrations| migrations.steps.iter().any(|step| step.entry == *path))
        {
            return Err(PackageError::new(
                ErrorCode::UnexpectedEntry,
                "migration file has no declared migration step",
            ));
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}
