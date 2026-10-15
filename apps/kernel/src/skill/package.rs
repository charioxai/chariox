//! MP-08 / MP-10 / MP-11: bounded immutable instruction payloads, including modes.
use super::{managed_capability_root, CharioxSkillMetadata};
use crate::{error::DaemonError, mcp::validate_registry_name};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharioxSkillPackage {
    pub metadata: CharioxSkillMetadata,
    pub version_hash: String,
    pub files: Vec<CharioxSkillPackageFile>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharioxSkillPackageFile {
    pub executable: bool,
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
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skill.package",
        message: message.into(),
    }
}

pub fn materialize_skill_package(
    base_dir: &Path,
    package: &CharioxSkillPackage,
) -> Result<PathBuf, DaemonError> {
    materialize_skill_package_for_id(base_dir, &package.metadata.name, package)
}
pub(super) fn materialize_skill_package_for_id(
    base_dir: &Path,
    id: &str,
    package: &CharioxSkillPackage,
) -> Result<PathBuf, DaemonError> {
    validate_registry_name(id, "skill ID")?;
    validate_registry_name(&package.metadata.name, "skill name")?;
    let decoded = verify_package(package)?;
    ensure_directory(base_dir)?;
    let base = fs::canonicalize(base_dir)
        .map_err(|_| failure("cannot resolve skill materialization root"))?;
    let parent = base.join(id);
    ensure_directory(&parent)?;
    let destination = parent.join(&package.version_hash);
    if fs::symlink_metadata(&destination).is_ok() {
        ensure_directory(&destination)?;
        if package_skill_directory(&destination)?.version_hash != package.version_hash {
            return Err(failure(
                "cached skill package failed complete-content verification",
            ));
        }
        return Ok(destination);
    }
    let staging = parent.join(format!(".stage-{:032x}", rand::random::<u128>()));
    fs::create_dir(&staging).map_err(|_| failure("cannot stage skill materialization"))?;
    struct OwnedStage(PathBuf);
    impl Drop for OwnedStage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _owned = OwnedStage(staging.clone());
    for (file, bytes) in decoded {
        let path = staging.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| failure("cannot create skill file directory"))?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(if file.executable { 0o700 } else { 0o600 })
                .custom_flags(libc::O_NOFOLLOW);
        }
        let mut output = options
            .open(&path)
            .map_err(|_| failure("cannot create skill file"))?;
        output
            .write_all(&bytes)
            .and_then(|()| output.sync_all())
            .map_err(|_| failure("cannot persist skill file"))?;
    }
    let materialized = package_skill_directory(&staging)?;
    if materialized.version_hash != package.version_hash
        || materialized.metadata.name != package.metadata.name
    {
        return Err(failure("materialized skill package identity mismatch"));
    }
    sync_directories(&staging)?;
    match fs::rename(&staging, &destination) {
        Ok(()) => {}
        Err(_) if destination.is_dir() => {
            ensure_directory(&destination)?;
            if package_skill_directory(&destination)?.version_hash != package.version_hash {
                return Err(failure(
                    "concurrent skill materialization failed verification",
                ));
            }
        }
        Err(_) => return Err(failure("cannot publish skill materialization")),
    }
    fs::File::open(&parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| failure("cannot persist skill package publication"))?;
    Ok(destination)
}
fn ensure_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path).map_err(|_| failure("cannot create skill package directory"))?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| failure("cannot inspect skill directory"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(failure("skill package directory cannot be a symlink"));
    }
    Ok(())
}
fn sync_directories(root: &Path) -> Result<(), DaemonError> {
    for entry in fs::read_dir(root).map_err(|_| failure("cannot inspect staged skill"))? {
        let entry = entry.map_err(|_| failure("cannot inspect staged skill"))?;
        if entry
            .file_type()
            .map_err(|_| failure("cannot inspect staged skill"))?
            .is_dir()
        {
            sync_directories(&entry.path())?;
        }
    }
    fs::File::open(root)
        .and_then(|file| file.sync_all())
        .map_err(|_| failure("cannot persist skill directory"))
}
pub(crate) fn verify_package(
    package: &CharioxSkillPackage,
) -> Result<Vec<(&CharioxSkillPackageFile, Vec<u8>)>, DaemonError> {
    if package.files.is_empty() || package.files.len() > MAX_SKILL_PACKAGE_FILES {
        return Err(failure("skill package must contain 1 to 512 files"));
    }
    if package.version_hash.len() != 64
        || !package
            .version_hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(failure("invalid skill content digest"));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    let mut decoded = Vec::new();
    for file in &package.files {
        validate_package_relative_path(&file.path)?;
        if !names.insert(file.path.to_lowercase()) {
            return Err(failure("duplicate or case-conflicting skill package path"));
        }
        if file.content_base64.len() as u64 > ((MAX_SKILL_PACKAGE_BYTES + 2) / 3) * 4 {
            return Err(failure("skill package exceeds maximum byte size"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&file.content_base64)
            .map_err(|_| failure("invalid skill package base64"))?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| failure("skill package size overflow"))?;
        if total > MAX_SKILL_PACKAGE_BYTES {
            return Err(failure("skill package exceeds maximum byte size"));
        }
        if crate::extension_package_security::contains_private_key(&bytes) {
            return Err(failure("skill package contains private key material"));
        }
        if sha256_hex(&bytes) != file.sha256 {
            return Err(failure("skill package file hash mismatch"));
        }
        decoded.push((file, bytes));
    }
    if !package.files.iter().any(|file| file.path == "SKILL.md") {
        return Err(failure("skill package requires SKILL.md"));
    }
    if package_digest(&package.files) != package.version_hash {
        return Err(failure("skill package complete-content digest mismatch"));
    }
    Ok(decoded)
}
pub(crate) fn package_digest(files: &[CharioxSkillPackageFile]) -> String {
    let mut files: Vec<_> = files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Sha256::new();
    hasher.update(b"chariox-skill-package-v2\0");
    for file in files {
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update([0, u8::from(file.executable), 0]);
    }
    format!("{:x}", hasher.finalize())
}
pub(crate) fn remote_skill_materialization_base(_workspace: impl AsRef<Path>) -> PathBuf {
    if let Some(root) = managed_capability_root() {
        return root.join("state/skill-materializations");
    }
    crate::config::DaemonConfig::default_user_config_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("state/skill-materializations")
}
pub(crate) fn package_skill_directory(
    skill_dir: &Path,
) -> Result<CharioxSkillPackage, DaemonError> {
    let mut files = Vec::new();
    let mut total = 0;
    let root = fs::canonicalize(skill_dir).map_err(|_| failure("cannot resolve skill source"))?;
    let instructions = fs::symlink_metadata(root.join("SKILL.md"))
        .map_err(|_| failure("skill source requires a regular SKILL.md"))?;
    if !instructions.is_file() || instructions.file_type().is_symlink() {
        return Err(failure(
            "skill source requires a regular, contained SKILL.md",
        ));
    }
    collect_files(&root, &root, &mut files, &mut total)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let body_file = files
        .iter()
        .find(|file| file.path == "SKILL.md")
        .ok_or_else(|| failure("skill package requires SKILL.md"))?;
    let body = base64::engine::general_purpose::STANDARD
        .decode(&body_file.content_base64)
        .map_err(|_| failure("invalid skill body encoding"))?;
    let parsed = super::frontmatter::parse_frontmatter(
        std::str::from_utf8(&body).map_err(|_| failure("SKILL.md must be UTF-8"))?,
    )?;
    let metadata = CharioxSkillMetadata {
        name: parsed.name,
        description: parsed.description,
        short_description: parsed.short_description,
        path: skill_dir.join("SKILL.md"),
    };
    let package = CharioxSkillPackage {
        metadata,
        version_hash: package_digest(&files),
        files,
    };
    verify_package(&package)?;
    Ok(package)
}
fn collect_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<CharioxSkillPackageFile>,
    total: &mut u64,
) -> Result<(), DaemonError> {
    for entry in fs::read_dir(dir).map_err(|_| failure("cannot read skill directory"))? {
        let entry = entry.map_err(|_| failure("cannot inspect skill file"))?;
        if SKILL_PACKAGE_IGNORED_DIRS
            .iter()
            .any(|ignored| entry.file_name() == *ignored)
        {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| failure("cannot inspect skill file"))?;
        if file_type.is_symlink() {
            return Err(failure(
                "skill packages cannot contain symlinks; import regular contained files",
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| failure("skill file escapes source root"))?
            .to_str()
            .ok_or_else(|| failure("skill paths must be UTF-8"))?
            .to_string();
        validate_package_relative_path(&relative)?;
        if file_type.is_dir() {
            collect_files(root, &path, files, total)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(failure("skill packages require regular files"));
        }
        if files.len() >= MAX_SKILL_PACKAGE_FILES {
            return Err(failure("skill package exceeds maximum file count (512)"));
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let input = options
            .open(&path)
            .map_err(|_| failure("cannot open regular skill source"))?;
        let metadata = input
            .metadata()
            .map_err(|_| failure("cannot inspect skill source"))?;
        if !metadata.is_file() || metadata.len() > MAX_SKILL_PACKAGE_BYTES.saturating_sub(*total) {
            return Err(failure("skill package exceeds maximum byte size (10 MiB)"));
        }
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        let mut bytes = Vec::new();
        input
            .take(MAX_SKILL_PACKAGE_BYTES.saturating_sub(*total) + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("cannot read skill source"))?;
        if crate::extension_package_security::contains_private_key(&bytes) {
            return Err(failure("skill package contains private key material"));
        }
        *total += bytes.len() as u64;
        if *total > MAX_SKILL_PACKAGE_BYTES {
            return Err(failure("skill package exceeds maximum byte size (10 MiB)"));
        }
        files.push(CharioxSkillPackageFile {
            executable,
            path: relative,
            sha256: sha256_hex(&bytes),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(())
}
fn validate_package_relative_path(path: &str) -> Result<(), DaemonError> {
    use unicode_normalization::UnicodeNormalization;
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || path
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        || path.nfc().collect::<String>() != path
    {
        return Err(failure(
            "skill package path must be portable, relative and contained",
        ));
    }
    let basename = path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let private_account = path.split('/').any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "auth.json" | ".credentials.json" | "credentials.toml" | "oauth-token"
        )
    });
    let adjacent_config = !path.contains('/')
        && matches!(
            basename.as_str(),
            "config.json"
                | "config.toml"
                | "settings.json"
                | "settings.local.json"
                | "opencode.json"
                | "opencode.jsonc"
                | ".mcp.json"
        );
    if crate::extension_package_security::sensitive_package_path(path)
        || private_account
        || adjacent_config
    {
        return Err(failure("skill packages cannot include account, credential or adjacent private configuration files; choose the skill folder and exclude private files"));
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > 255
            || part.ends_with(['.', ' '])
            || matches!(
                stem.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
        {
            return Err(failure(
                "skill package path must be portable, relative and contained",
            ));
        }
    }
    Ok(())
}
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
