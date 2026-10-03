//! MP-08: Deterministic resolution. Values never implement Serialize or unredacted Debug.
use super::*;
use crate::error::DaemonError;
use crate::secret::CredentialVaultStore;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub struct ResolvedProjectEnvironment {
    pub(crate) values: BTreeMap<(String, String), Zeroizing<String>>,
    pub unresolved: Vec<ProjectEnvironmentEntry>,
}
impl std::fmt::Debug for ResolvedProjectEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProjectEnvironment")
            .field("value_count", &self.values.len())
            .field("unresolved", &self.unresolved)
            .finish()
    }
}

pub(crate) fn environment_error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Project environment",
        message: message.into(),
    }
}

/// Literal assignments only: neither .envrc nor profiles are executed.
pub fn parse_environment_assignment(line: &str, key: &str) -> Result<Option<String>, &'static str> {
    let line = line
        .trim()
        .strip_prefix("export ")
        .unwrap_or(line.trim())
        .trim();
    let Some((name, value)) = line.split_once('=') else {
        return Ok(None);
    };
    if name.trim() != key {
        return Ok(None);
    }
    let value = value.trim();
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        if inner.contains('\'') {
            return Err("unsupported environment assignment");
        }
        return Ok(Some(inner.into()));
    }
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let mut output = String::new();
        let mut characters = value[1..value.len() - 1].chars();
        while let Some(character) = characters.next() {
            if character == '\\' {
                let escaped = characters
                    .next()
                    .ok_or("unsupported environment assignment")?;
                if !matches!(escaped, '\\' | '"' | '$' | '`') {
                    return Err("unsupported environment escape");
                }
                output.push(escaped);
            } else if matches!(character, '$' | '`' | '"') {
                return Err("environment expansion requires explicit input");
            } else {
                output.push(character);
            }
        }
        return Ok(Some(output));
    }
    if value.contains(['$', '`', '\\', '\n', '\r']) {
        return Err("environment expansion requires explicit input");
    }
    if value.contains(['\'', '"', ';', '&', '|', '<', '>', '(', ')']) {
        return Err("unsupported environment assignment");
    }
    Ok(Some(
        value.split(" #").next().unwrap_or(value).trim_end().into(),
    ))
}

/// Reject symlinks at every component, open relative to directory handles on Unix.
/// This also fences component replacement between validation and the final open.
pub(crate) fn open_workspace_file(root: &Path, relative: &str) -> Result<File, DaemonError> {
    relative_environment_path(relative).map_err(environment_error)?;
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::OpenOptionsExt;
        let mut parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(root)
            .map_err(|_| environment_error("Project environment workspace unavailable"))?;
        let parts: Vec<_> = relative.split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            let component = std::ffi::CString::new(*part)
                .map_err(|_| environment_error("invalid environment path"))?;
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if index + 1 < parts.len() {
                    libc::O_DIRECTORY
                } else {
                    0
                };
            let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags) };
            if fd < 0 {
                return Err(environment_error("Project environment locator unavailable"));
            }
            parent = unsafe { File::from_raw_fd(fd) };
        }
        if !parent
            .metadata()
            .map_err(|_| environment_error("environment file unavailable"))?
            .is_file()
        {
            return Err(environment_error(
                "environment locator must be a regular file",
            ));
        }
        Ok(parent)
    }
    #[cfg(not(unix))]
    {
        let mut path = root.to_path_buf();
        for part in relative.split('/') {
            path.push(part);
            if std::fs::symlink_metadata(&path)
                .map_err(|_| environment_error("environment locator unavailable"))?
                .file_type()
                .is_symlink()
            {
                return Err(environment_error("environment symlink rejected"));
            }
        }
        let file =
            File::open(path).map_err(|_| environment_error("environment locator unavailable"))?;
        if !file
            .metadata()
            .map_err(|_| environment_error("environment locator unavailable"))?
            .is_file()
        {
            return Err(environment_error(
                "environment locator must be a regular file",
            ));
        }
        Ok(file)
    }
}

pub(crate) fn read_environment_file(
    root: &Path,
    relative: &str,
) -> Result<Zeroizing<String>, DaemonError> {
    let mut bytes = Zeroizing::new(Vec::new());
    open_workspace_file(root, relative)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| environment_error("environment file read failed"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(environment_error("environment file exceeds bounds"));
    }
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| environment_error("environment file must be UTF-8"))?;
    Ok(Zeroizing::new(value.to_owned()))
}

pub fn project_environment_vault_service(project_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "project-environment:{:x}",
        Sha256::digest(project_id.as_bytes())
    )
}
pub fn project_environment_vault_key(entry: &ProjectEnvironmentEntry) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", entry.workspace_id, entry.name).as_bytes())
    )
}

pub fn resolve_project_environment(
    manifest: &ProjectEnvironmentManifest,
    workspaces: &BTreeMap<String, PathBuf>,
    workspace_environment: &BTreeMap<String, BTreeMap<String, Zeroizing<String>>>,
    vault: &dyn CredentialVaultStore,
) -> Result<ResolvedProjectEnvironment, DaemonError> {
    manifest.validate().map_err(environment_error)?;
    let service = project_environment_vault_service(&manifest.project_id);
    let mut resolved = ResolvedProjectEnvironment {
        values: BTreeMap::new(),
        unresolved: Vec::new(),
    };
    for entry in &manifest.entries {
        if entry.excluded {
            let mut skipped = entry.clone();
            skipped.status = ProjectEnvironmentEntryStatus::Missing;
            resolved.unresolved.push(skipped);
            continue;
        }
        let root = workspaces
            .get(&entry.workspace_id)
            .ok_or_else(|| environment_error("manifest workspace not selected"))?;
        let key = project_environment_vault_key(entry);
        let result = match &entry.locator {
            ProjectEnvironmentLocator::EnvFile { path, key } => read_environment_file(root, path)
                .and_then(|text| {
                    let mut value = None;
                    for line in text.lines() {
                        if let Some(next) =
                            parse_environment_assignment(line, key).map_err(environment_error)?
                        {
                            // A duplicate is ambiguous rather than a silent last-value win.
                            if value.is_some() {
                                return Err(environment_error("duplicate environment assignment"));
                            }
                            value = Some(Zeroizing::new(next));
                        }
                    }
                    value.ok_or_else(|| environment_error("referenced environment value missing"))
                }),
            ProjectEnvironmentLocator::WorkspaceEnvironment { name } => workspace_environment
                .get(&entry.workspace_id)
                .and_then(|env| env.get(name))
                .cloned()
                .ok_or_else(|| environment_error("referenced workspace environment value missing")),
            ProjectEnvironmentLocator::ConfigFile { path } => read_environment_file(root, path),
            ProjectEnvironmentLocator::Vault { service, key } => {
                vault.get_secret(service, key).map(Zeroizing::new)
            }
            ProjectEnvironmentLocator::Missing => {
                vault.get_secret(&service, &key).map(Zeroizing::new)
            }
        };
        match result {
            Ok(value) if !value.is_empty() && !value.contains('\0') => {
                // Secret values enter the Vault directly. All supplied inputs use the same key.
                if entry.classification == ProjectEnvironmentClassification::Secret {
                    vault.set_secret(&service, &key, &value)?;
                }
                resolved
                    .values
                    .insert((entry.workspace_id.clone(), entry.name.clone()), value);
            }
            Err(error) if crate::secret::is_chariox_vault_locked_error(&error) => {
                return Err(error)
            }
            _ => {
                let mut unresolved = entry.clone();
                unresolved.status = if matches!(
                    entry.locator,
                    ProjectEnvironmentLocator::Missing
                        | ProjectEnvironmentLocator::WorkspaceEnvironment { .. }
                ) {
                    ProjectEnvironmentEntryStatus::Missing
                } else {
                    ProjectEnvironmentEntryStatus::Problem
                };
                resolved.unresolved.push(unresolved);
            }
        }
    }
    Ok(resolved)
}
