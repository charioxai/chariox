use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

use super::{managed_capability_root, parse_skill_metadata, CharioxSkillMetadata};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxSkillPackage {
    pub metadata: CharioxSkillMetadata,
    pub version_hash: String,
    pub files: Vec<CharioxSkillPackageFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxSkillPackageFile {
    pub path: String,
    pub sha256: String,
    pub content_base64: String,
}

const MAX_SKILL_PACKAGE_FILES: usize = 512;
const MAX_SKILL_PACKAGE_BYTES: u64 = 10 * 1024 * 1024;
const SKILL_PACKAGE_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".DS_Store",
    "node_modules",
    ".venv",
    "venv",
    "target",
    "dist",
    "build",
];

pub fn materialize_skill_package(
    base_dir: &Path,
    package: &CharioxSkillPackage,
) -> Result<PathBuf, DaemonError> {
    validate_registry_name(&package.metadata.name, "skill name")?;
    let destination = base_dir
        .join(&package.metadata.name)
        .join(&package.version_hash);
    if destination.join("SKILL.md").exists() {
        return Ok(destination);
    }
    let temp_destination = base_dir.join(format!(
        ".{}.{}.tmp",
        package.metadata.name,
        std::process::id()
    ));
    if temp_destination.exists() {
        fs::remove_dir_all(&temp_destination).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "failed to clear stale skill materialization temp dir `{}`: {error}",
                temp_destination.display()
            ),
        })?;
    }
    fs::create_dir_all(&temp_destination).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.materialize",
        message: format!(
            "failed to create skill materialization temp dir `{}`: {error}",
            temp_destination.display()
        ),
    })?;
    for file in &package.files {
        let relative_path = validate_package_relative_path(&file.path, "skill.materialize")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&file.content_base64)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "skill package file `{}` has invalid base64 content: {error}",
                    file.path
                ),
            })?;
        let actual_hash = sha256_hex(&bytes);
        if actual_hash != file.sha256 {
            return Err(DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "skill package file `{}` hash mismatch: expected {}, got {}",
                    file.path, file.sha256, actual_hash
                ),
            });
        }
        let path = temp_destination.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "failed to create skill materialization dir `{}`: {error}",
                    parent.display()
                ),
            })?;
        }
        fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!("failed to write skill file `{}`: {error}", path.display()),
        })?;
    }
    let materialized = package_skill_directory(&temp_destination)?;
    if materialized.version_hash != package.version_hash {
        return Err(DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "materialized skill package hash mismatch: expected {}, got {}",
                package.version_hash, materialized.version_hash
            ),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "failed to create skill materialization parent `{}`: {error}",
                parent.display()
            ),
        })?;
    }
    if destination.exists() {
        fs::remove_dir_all(&temp_destination).ok();
        return Ok(destination);
    }
    fs::rename(&temp_destination, &destination).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.materialize",
        message: format!(
            "failed to publish materialized skill `{}`: {error}",
            destination.display()
        ),
    })?;
    Ok(destination)
}

pub(crate) fn remote_skill_materialization_base(workspace: impl AsRef<Path>) -> PathBuf {
    if let Some(root) = managed_capability_root() {
        return root.join("remote").join("skills");
    }
    workspace
        .as_ref()
        .join(".chariox")
        .join("remote")
        .join("skills")
}

pub(super) fn package_skill_directory(
    skill_dir: &Path,
) -> Result<CharioxSkillPackage, DaemonError> {
    let metadata = parse_skill_metadata(&skill_dir.join("SKILL.md"))?;
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    collect_skill_package_files(skill_dir, skill_dir, &mut files, &mut total_bytes)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    for file in &files {
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\0");
    }
    let version_hash = hex_digest(hasher.finalize().as_slice());
    Ok(CharioxSkillPackage {
        metadata,
        version_hash,
        files,
    })
}

fn collect_skill_package_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<CharioxSkillPackageFile>,
    total_bytes: &mut u64,
) -> Result<(), DaemonError> {
    for entry in fs::read_dir(dir).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.package",
        message: format!(
            "failed to read skill directory `{}`: {error}",
            dir.display()
        ),
    })? {
        let entry = entry.map_err(|error| DaemonError::LocalTransport {
            operation: "skill.package",
            message: format!("failed to read skill directory entry: {error}"),
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!("failed to inspect skill path `{}`: {error}", path.display()),
            })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if SKILL_PACKAGE_IGNORED_DIRS
                .iter()
                .any(|ignored| *ignored == file_name)
            {
                continue;
            }
            collect_skill_package_files(root, &path, files, total_bytes)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if files.len() >= MAX_SKILL_PACKAGE_FILES {
            return Err(DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!(
                    "skill package exceeds maximum file count ({MAX_SKILL_PACKAGE_FILES})"
                ),
            });
        }
        let relative_path =
            path.strip_prefix(root)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "skill.package",
                    message: format!(
                        "skill path `{}` is outside package root `{}`: {error}",
                        path.display(),
                        root.display()
                    ),
                })?;
        let relative_path =
            validate_package_relative_path(&relative_path.to_string_lossy(), "skill.package")?;
        let bytes = fs::read(&path).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.package",
            message: format!("failed to read skill file `{}`: {error}", path.display()),
        })?;
        *total_bytes += bytes.len() as u64;
        if *total_bytes > MAX_SKILL_PACKAGE_BYTES {
            return Err(DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!(
                    "skill package exceeds maximum byte size ({MAX_SKILL_PACKAGE_BYTES})"
                ),
            });
        }
        files.push(CharioxSkillPackageFile {
            path: relative_path.to_string_lossy().replace('\\', "/"),
            sha256: sha256_hex(&bytes),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(())
}

fn validate_package_relative_path(
    path: &str,
    operation: &'static str,
) -> Result<PathBuf, DaemonError> {
    let relative_path = PathBuf::from(path);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("skill package path `{path}` must be relative and contained"),
        });
    }
    Ok(relative_path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(digest.as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
