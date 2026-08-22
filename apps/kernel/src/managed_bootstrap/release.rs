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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedRelease {
    pub(super) digest: String,
    pub(super) kernel_binary: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: u32,
    artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseArtifact {
    name: String,
    path: PathBuf,
    sha256: String,
}

pub(super) fn verify_release(
    manifest_path: &Path,
    signature_path: &Path,
    public_key_path: &Path,
    expected_digest: &str,
    expected_kernel_binary: &Path,
) -> Result<VerifiedRelease, DaemonError> {
    let manifest_bytes =
        read_bounded_regular_file(manifest_path, MAX_MANIFEST_BYTES, "release manifest")?;
    let actual_digest = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
    if actual_digest != expected_digest {
        return Err(release_error(
            "release manifest digest does not match the managed environment",
        ));
    }
    let key_bytes =
        read_bounded_regular_file(public_key_path, MAX_KEY_BYTES, "release public key")?;
    let signature_bytes =
        read_bounded_regular_file(signature_path, MAX_SIGNATURE_BYTES, "release signature")?;
    let verifying_key = decode_verifying_key(&key_bytes)?;
    let signature = decode_signature(&signature_bytes)?;
    verifying_key
        .verify(&manifest_bytes, &signature)
        .map_err(|_| release_error("release manifest signature is invalid"))?;

    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| release_error("release manifest is invalid"))?;
    if manifest.schema_version != 1
        || manifest.artifacts.is_empty()
        || manifest.artifacts.len() > 32
    {
        return Err(release_error("release manifest schema is unsupported"));
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
    let actual_kernel_digest = digest_regular_file(&kernel.path)?;
    if actual_kernel_digest != kernel.sha256 {
        return Err(release_error(
            "kernel artifact digest does not match the signed release",
        ));
    }
    Ok(VerifiedRelease {
        digest: actual_digest,
        kernel_binary: kernel.path.clone(),
    })
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
