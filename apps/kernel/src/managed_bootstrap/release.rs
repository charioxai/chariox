use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_KEY_BYTES: u64 = 1024;
const MAX_SIGNATURE_BYTES: u64 = 1024;
const MAX_ATTESTATION_BYTES: u64 = 16 * 1024;
const MAX_SERVICE_UNIT_BYTES: u64 = 32 * 1024;
const MANAGED_BOOTSTRAP_PATH: &str = "/usr/local/bin/chariox-managed-bootstrap";
const MANAGED_BOOTSTRAP_SERVICE_PATH: &str =
    "/etc/systemd/system/chariox-managed-bootstrap.service";
const RETIRED_SERVICE_NAMES: &[&str] = &[
    "chariox-disposable-worker-bootstrap.service",
    "chariox-kernel.service",
    "chariox-relay.service",
];
const BUILD_ATTESTATION_PATH: &str = "/usr/lib/chariox/build-attestation.json";
const BUILD_ATTESTATION_SIGNATURE_PATH: &str = "/usr/lib/chariox/build-attestation.sig";
const BUILDER_PUBLIC_KEY_PATH: &str = "/usr/lib/chariox/builder-public-key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedRelease {
    pub(super) digest: String,
    pub(super) kernel_binary: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedReleaseEvidence {
    pub(super) digest: String,
    pub(super) source_commit: String,
    pub(super) source_tree: String,
    pub(super) target: String,
    pub(super) active_release_path: PathBuf,
    pub(super) manifest_signature_verified: bool,
    pub(super) manifest_digest_verified: bool,
    pub(super) kernel_artifact_verified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: u32,
    source_commit: Option<String>,
    source_tree: Option<String>,
    artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseArtifact {
    name: String,
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildAttestation {
    schema_version: u32,
    source_commit: String,
    source_tree: String,
    target: String,
    artifacts: Vec<BuildArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildArtifact {
    name: String,
    sha256: String,
}

pub(super) fn verify_release(
    manifest_path: &Path,
    signature_path: &Path,
    public_key_path: &Path,
    expected_digest: &str,
    expected_kernel_binary: &Path,
) -> Result<VerifiedRelease, DaemonError> {
    let resolved = resolve_release_paths(
        manifest_path,
        signature_path,
        public_key_path,
        expected_kernel_binary,
        expected_digest,
    )?;
    let manifest_bytes =
        read_bounded_regular_file(&resolved.manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    let actual_digest = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
    if actual_digest != expected_digest {
        return Err(release_error(
            "release manifest digest does not match the managed environment",
        ));
    }
    let key_bytes =
        read_bounded_regular_file(&resolved.public_key, MAX_KEY_BYTES, "release public key")?;
    let signature_bytes = read_bounded_regular_file(
        &resolved.signature,
        MAX_SIGNATURE_BYTES,
        "release signature",
    )?;
    let verifying_key = decode_verifying_key(&key_bytes)?;
    let signature = decode_signature(&signature_bytes)?;
    verifying_key
        .verify(&manifest_bytes, &signature)
        .map_err(|_| release_error("release manifest signature is invalid"))?;

    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| release_error("release manifest is invalid"))?;
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > 32 {
        return Err(release_error("release manifest schema is unsupported"));
    }
    match manifest.schema_version {
        1 if manifest.source_commit.is_none() && manifest.source_tree.is_none() => {}
        2 if manifest
            .source_commit
            .as_deref()
            .is_some_and(is_git_object_id)
            && manifest
                .source_tree
                .as_deref()
                .is_some_and(is_git_object_id) => {}
        1 | 2 => {
            return Err(release_error("release manifest source identity is invalid"));
        }
        _ => return Err(release_error("release manifest schema is unsupported")),
    }
    let mut kernels = manifest
        .artifacts
        .iter()
        .filter(|artifact| artifact.name == "chariox-kernel");
    let kernel = kernels
        .next()
        .ok_or_else(|| release_error("release manifest does not contain the kernel artifact"))?;
    if kernels.next().is_some() || kernel.path != expected_kernel_binary {
        return Err(release_error(
            "release manifest kernel artifact is ambiguous",
        ));
    }
    validate_digest(&kernel.sha256)?;
    let actual_kernel_digest = digest_regular_file(&resolved.kernel)?;
    if actual_kernel_digest != kernel.sha256 {
        return Err(release_error(
            "kernel artifact digest does not match the signed release",
        ));
    }
    Ok(VerifiedRelease {
        digest: actual_digest,
        kernel_binary: resolved.kernel,
    })
}

pub(super) fn verify_release_evidence(
    manifest_path: &Path,
    signature_path: &Path,
    public_key_path: &Path,
    expected_digest: &str,
    expected_kernel_binary: &Path,
) -> Result<VerifiedReleaseEvidence, DaemonError> {
    let verified = verify_release(
        manifest_path,
        signature_path,
        public_key_path,
        expected_digest,
        expected_kernel_binary,
    )?;
    let resolved = resolve_release_paths(
        manifest_path,
        signature_path,
        public_key_path,
        expected_kernel_binary,
        expected_digest,
    )?;
    let active_release_path = resolved
        .active_release_root
        .ok_or_else(|| release_error("installed release has no active versioned layout"))?;
    let manifest_bytes =
        read_bounded_regular_file(&resolved.manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| release_error("release manifest is invalid"))?;
    let source_commit = manifest
        .source_commit
        .clone()
        .ok_or_else(|| release_error("release manifest source identity is unavailable"))?;
    let source_tree = manifest
        .source_tree
        .clone()
        .ok_or_else(|| release_error("release manifest source identity is unavailable"))?;
    let attestation = read_pinned_release_artifact(
        &manifest,
        "chariox-build-attestation",
        BUILD_ATTESTATION_PATH,
        &active_release_path,
    )?;
    let attestation_signature = read_pinned_release_artifact(
        &manifest,
        "chariox-build-attestation-signature",
        BUILD_ATTESTATION_SIGNATURE_PATH,
        &active_release_path,
    )?;
    let builder_public_key = read_pinned_release_artifact(
        &manifest,
        "chariox-builder-public-key",
        BUILDER_PUBLIC_KEY_PATH,
        &active_release_path,
    )?;
    let attestation_bytes =
        read_bounded_regular_file(&attestation, MAX_ATTESTATION_BYTES, "build attestation")?;
    let signature_bytes = read_bounded_regular_file(
        &attestation_signature,
        MAX_SIGNATURE_BYTES,
        "build attestation signature",
    )?;
    let public_key_bytes =
        read_bounded_regular_file(&builder_public_key, MAX_KEY_BYTES, "builder public key")?;
    let verifying_key = decode_verifying_key(&public_key_bytes)?;
    let signature = decode_signature(&signature_bytes)?;
    verifying_key
        .verify(&attestation_bytes, &signature)
        .map_err(|_| release_error("build attestation signature is invalid"))?;
    let attestation: BuildAttestation = serde_json::from_slice(&attestation_bytes)
        .map_err(|_| release_error("build attestation is invalid"))?;
    if attestation.schema_version != 1
        || !is_git_object_id(&attestation.source_commit)
        || !is_git_object_id(&attestation.source_tree)
        || attestation.source_commit != source_commit
        || attestation.source_tree != source_tree
        || attestation.target != expected_release_target()
        || attestation.artifacts.len() != 3
    {
        return Err(release_error(
            "build attestation source identity or target is invalid",
        ));
    }
    for (artifact, expected_name) in attestation.artifacts.iter().zip([
        "chariox-kernel",
        "chariox-managed-bootstrap",
        "chariox-relay",
    ]) {
        if artifact.name != expected_name {
            return Err(release_error(
                "build attestation artifact order or identity is invalid",
            ));
        }
        validate_digest(&artifact.sha256)?;
    }
    let kernel_artifact = &attestation.artifacts[0];
    let manifest_kernel = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "chariox-kernel")
        .ok_or_else(|| release_error("release manifest does not contain the kernel artifact"))?;
    if kernel_artifact.sha256 != manifest_kernel.sha256 {
        return Err(release_error(
            "build attestation kernel artifact does not match the signed release",
        ));
    }
    let manifest_supervisor = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "chariox-managed-bootstrap")
        .ok_or_else(|| {
            release_error("release manifest does not contain the managed bootstrap artifact")
        })?;
    let _supervisor = read_pinned_release_artifact(
        &manifest,
        "chariox-managed-bootstrap",
        MANAGED_BOOTSTRAP_PATH,
        &active_release_path,
    )?;
    if manifest_supervisor.path != Path::new(MANAGED_BOOTSTRAP_PATH)
        || manifest_supervisor.sha256 != attestation.artifacts[1].sha256
    {
        return Err(release_error(
            "build attestation supervisor artifact does not match the signed release",
        ));
    }
    Ok(VerifiedReleaseEvidence {
        digest: verified.digest,
        source_commit,
        source_tree,
        target: attestation.target,
        active_release_path,
        manifest_signature_verified: true,
        manifest_digest_verified: true,
        kernel_artifact_verified: true,
    })
}

pub(super) fn verify_managed_bootstrap_service_binding(
    release: &VerifiedReleaseEvidence,
    service_unit_path: &Path,
    service_wants_path: &Path,
) -> Result<(), DaemonError> {
    let manifest_path = release
        .active_release_path
        .join("usr/lib/chariox/release-manifest.json");
    let manifest_bytes =
        read_bounded_regular_file(&manifest_path, MAX_MANIFEST_BYTES, "release manifest")?;
    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| release_error("release manifest is invalid"))?;
    let service = read_pinned_release_artifact(
        &manifest,
        "chariox-managed-bootstrap.service",
        MANAGED_BOOTSTRAP_SERVICE_PATH,
        &release.active_release_path,
    )?;
    let service_bytes = read_bounded_regular_file(
        &service,
        MAX_SERVICE_UNIT_BYTES,
        "managed bootstrap service",
    )?;
    let service_text = std::str::from_utf8(&service_bytes)
        .map_err(|_| release_error("managed bootstrap service is not UTF-8"))?;
    if !service_text
        .lines()
        .any(|line| line.trim() == "ExecStart=/usr/local/bin/chariox-managed-bootstrap")
        || service_text.lines().any(|line| {
            line.trim() == "ExecStart=/usr/local/bin/chariox-managed-bootstrap --disposable-worker"
        })
    {
        return Err(release_error(
            "managed bootstrap service is not bound to the managed bootstrap binary",
        ));
    }
    require_release_symlink(service_unit_path, &service)?;
    require_release_symlink(service_wants_path, &service)?;
    let service_dir = service_unit_path
        .parent()
        .ok_or_else(|| release_error("managed bootstrap service path has no directory"))?;
    let wants_dir = service_wants_path
        .parent()
        .ok_or_else(|| release_error("managed bootstrap wants path has no directory"))?;
    for name in RETIRED_SERVICE_NAMES {
        for path in [service_dir.join(name), wants_dir.join(name)] {
            if fs::symlink_metadata(&path).is_ok() {
                return Err(release_error("retired Chariox service residue is present"));
            }
        }
    }
    Ok(())
}

fn require_release_symlink(path: &Path, expected: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| release_io_error("managed bootstrap service binding", error))?;
    if !metadata.file_type().is_symlink() {
        return Err(release_error(
            "managed bootstrap service binding is not a release symlink",
        ));
    }
    let actual = fs::canonicalize(path)
        .map_err(|error| release_io_error("managed bootstrap service binding", error))?;
    if actual != expected {
        return Err(release_error(
            "managed bootstrap service is not bound to the active release",
        ));
    }
    Ok(())
}

struct ResolvedReleasePaths {
    manifest: PathBuf,
    signature: PathBuf,
    public_key: PathBuf,
    kernel: PathBuf,
    active_release_root: Option<PathBuf>,
}

fn resolve_release_paths(
    manifest: &Path,
    signature: &Path,
    public_key: &Path,
    kernel: &Path,
    expected_digest: &str,
) -> Result<ResolvedReleasePaths, DaemonError> {
    let facades = [manifest, signature, public_key, kernel];
    let symlinked = facades
        .iter()
        .map(|path| {
            fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_symlink())
                .map_err(|error| release_io_error("release path", error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if symlinked.iter().all(|value| !value) {
        return Ok(ResolvedReleasePaths {
            manifest: manifest.to_path_buf(),
            signature: signature.to_path_buf(),
            public_key: public_key.to_path_buf(),
            kernel: kernel.to_path_buf(),
            active_release_root: None,
        });
    }
    if !symlinked.iter().all(|value| *value) {
        return Err(release_error(
            "installed release path layout is inconsistent",
        ));
    }

    let digest = expected_digest
        .strip_prefix("sha256:")
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| release_error("managed release digest is invalid"))?;
    let resolved = facades
        .iter()
        .map(|path| fs::canonicalize(path).map_err(|error| release_io_error("release path", error)))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_release_root = fs::canonicalize(
        manifest
            .parent()
            .ok_or_else(|| release_error("release manifest path has no parent"))?,
    )
    .map_err(|error| release_io_error("release path", error))?
    .join("releases")
    .join(digest);
    let release_root = release_root_for_manifest(&resolved[0])
        .filter(|root| root == &expected_release_root)
        .ok_or_else(|| release_error("installed release path is outside the pinned release"))?;
    let expected = [
        release_root.join("usr/lib/chariox/release-manifest.json"),
        release_root.join("usr/lib/chariox/release-manifest.sig"),
        release_root.join("usr/lib/chariox/release-public-key"),
        release_root.join("usr/local/bin/chariox-kernel"),
    ];
    if resolved != expected {
        return Err(release_error(
            "installed release path is outside the pinned release",
        ));
    }
    Ok(ResolvedReleasePaths {
        manifest: resolved[0].clone(),
        signature: resolved[1].clone(),
        public_key: resolved[2].clone(),
        kernel: resolved[3].clone(),
        active_release_root: Some(release_root),
    })
}

fn read_pinned_release_artifact(
    manifest: &ReleaseManifest,
    name: &str,
    expected_path: &str,
    release_root: &Path,
) -> Result<PathBuf, DaemonError> {
    let mut artifacts = manifest
        .artifacts
        .iter()
        .filter(|artifact| artifact.name == name);
    let artifact = artifacts
        .next()
        .ok_or_else(|| release_error(&format!("release manifest does not contain {name}")))?;
    if artifacts.next().is_some() || artifact.path != Path::new(expected_path) {
        return Err(release_error(&format!(
            "release manifest {name} artifact is ambiguous"
        )));
    }
    validate_digest(&artifact.sha256)?;
    let relative = expected_path
        .strip_prefix('/')
        .ok_or_else(|| release_error("release artifact path is invalid"))?;
    let path = release_root.join(relative);
    if digest_regular_file(&path)? != artifact.sha256 {
        return Err(release_error(&format!(
            "release manifest {name} artifact digest does not match the signed release"
        )));
    }
    Ok(path)
}

fn expected_release_target() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

fn release_root_for_manifest(path: &Path) -> Option<PathBuf> {
    let expected = ["release-manifest.json", "chariox", "lib", "usr"];
    let mut current = path;
    for name in expected {
        if current.file_name()? != name {
            return None;
        }
        current = current.parent()?;
    }
    Some(current.to_path_buf())
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: u64,
    label: &'static str,
) -> Result<Vec<u8>, DaemonError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| release_io_error(label, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(release_error(&format!(
            "{label} is not a bounded regular file"
        )));
    }
    fs::read(path).map_err(|error| release_io_error(label, error))
}

fn digest_regular_file(path: &Path) -> Result<String, DaemonError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| release_io_error("kernel artifact", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(release_error("kernel artifact is not a regular file"));
    }
    let mut file = File::open(path).map_err(|error| release_io_error("kernel artifact", error))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| release_io_error("kernel artifact", error))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn decode_verifying_key(bytes: &[u8]) -> Result<VerifyingKey, DaemonError> {
    let decoded = decode_text_bytes(bytes, 32, "release public key")?;
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| release_error("release public key has the wrong length"))?;
    VerifyingKey::from_bytes(&key).map_err(|_| release_error("release public key is invalid"))
}

fn decode_signature(bytes: &[u8]) -> Result<Signature, DaemonError> {
    let decoded = decode_text_bytes(bytes, 64, "release signature")?;
    Signature::from_slice(&decoded).map_err(|_| release_error("release signature is invalid"))
}

fn decode_text_bytes(bytes: &[u8], expected: usize, label: &str) -> Result<Vec<u8>, DaemonError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| release_error(&format!("{label} is not text")))?
        .trim();
    let decoded =
        if value.len() == expected * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            (0..value.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| release_error(&format!("{label} is invalid hex")))?
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value))
                .map_err(|_| release_error(&format!("{label} is invalid base64")))?
        };
    if decoded.len() != expected {
        return Err(release_error(&format!("{label} has the wrong length")));
    }
    Ok(decoded)
}

fn validate_digest(value: &str) -> Result<(), DaemonError> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Ok(());
    }
    Err(release_error("release artifact digest is invalid"))
}

fn is_git_object_id(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn release_io_error(label: &'static str, error: io::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "verify managed kernel release",
        message: format!("{label}: {error}"),
    }
}

fn release_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "verify managed kernel release",
        message: message.to_string(),
    }
}
