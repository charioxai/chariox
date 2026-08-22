use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::package::ManagedContextGitCredentialSelection;
use crate::error::DaemonError;

pub(crate) const GITHUB_CREDENTIAL_ID: &str = "github";
const GITHUB_HOSTNAME: &str = "github.com";
const MATERIALIZATION_SCHEMA_VERSION: u32 = 1;
const RECEIPT_SCHEMA_VERSION: u32 = 1;
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_GH_CONFIG_BYTES: usize = 64 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const INVENTORY_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const INVENTORY_CACHE_TTL: Duration = Duration::from_secs(15);
const MAX_BINDING_BYTES: usize = 8 * 1024;
static INVENTORY_CACHE: OnceLock<Mutex<Option<GitCredentialInventoryCache>>> = OnceLock::new();

struct GitCredentialInventoryCache {
    context: GitCredentialCommandContext,
    checked_at: Instant,
    available: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCredentialMaterialization {
    pub schema_version: u32,
    pub credential_id: String,
    pub hostname: String,
    token: String,
}

impl std::fmt::Debug for GitCredentialMaterialization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitCredentialMaterialization")
            .field("schema_version", &self.schema_version)
            .field("credential_id", &self.credential_id)
            .field("hostname", &self.hostname)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedContextGitCredentialReceipt {
    pub schema_version: u32,
    pub context_id: String,
    pub package_sha256: String,
    pub materialization_sha256: String,
    pub credential_id: String,
    pub hostname: String,
    pub token_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GitCredentialBindingPhase {
    Installing,
    Installed,
    Removing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GitCredentialBinding {
    schema_version: u32,
    context_id: String,
    package_sha256: String,
    materialization_sha256: String,
    credential_id: String,
    hostname: String,
    token_sha256: String,
    phase: GitCredentialBindingPhase,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct GitCredentialCommandContext {
    home: PathBuf,
    path: OsString,
    xdg_config_home: Option<PathBuf>,
    gh_config_dir: Option<PathBuf>,
}

impl std::fmt::Debug for GitCredentialCommandContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitCredentialCommandContext")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl GitCredentialCommandContext {
    pub(crate) fn source_from_process() -> Result<Self, DaemonError> {
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| scm_error("source kernel HOME is not configured"))?;
        let path = std::env::var_os("PATH")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| scm_error("source kernel PATH is not configured"))?;
        Ok(Self {
            home,
            path,
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            gh_config_dir: std::env::var_os("GH_CONFIG_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        })
    }

    pub(crate) fn managed_target(home: PathBuf) -> Result<Self, DaemonError> {
        if !home.is_absolute() {
            return Err(scm_error("managed Git credential HOME is not absolute"));
        }
        fs::create_dir_all(&home)
            .map_err(|error| scm_io_error("create managed Git credential HOME", error))?;
        let metadata = fs::symlink_metadata(&home)
            .map_err(|error| scm_io_error("inspect managed Git credential HOME", error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(scm_error(
                "managed Git credential HOME is not a real directory",
            ));
        }
        let path = std::env::var_os("PATH")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| scm_error("target kernel PATH is not configured"))?;
        let xdg_config_home = home.join(".config");
        ensure_private_directory_chain(&home, &xdg_config_home.join("gh"))?;
        Ok(Self {
            home,
            path,
            xdg_config_home: Some(xdg_config_home.clone()),
            gh_config_dir: Some(xdg_config_home.join("gh")),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests(home: PathBuf, path: OsString) -> Self {
        let xdg_config_home = home.join(".config");
        Self {
            home,
            path,
            xdg_config_home: Some(xdg_config_home.clone()),
            gh_config_dir: Some(xdg_config_home.join("gh")),
        }
    }
}

pub(crate) fn export_selected_git_credentials(
    selection: &ManagedContextGitCredentialSelection,
    context: &GitCredentialCommandContext,
) -> Result<Vec<GitCredentialMaterialization>, DaemonError> {
    let ManagedContextGitCredentialSelection::Selected { credential_ids } = selection else {
        return Ok(Vec::new());
    };
    validate_selected_ids(credential_ids)?;
    let token = read_github_token(context)?.ok_or_else(|| {
        scm_error("selected GitHub credential is not available on the source kernel")
    })?;
    Ok(vec![GitCredentialMaterialization {
        schema_version: MATERIALIZATION_SCHEMA_VERSION,
        credential_id: GITHUB_CREDENTIAL_ID.to_string(),
        hostname: GITHUB_HOSTNAME.to_string(),
        token,
    }])
}

pub(crate) fn github_credential_is_available(context: &GitCredentialCommandContext) -> bool {
    let cache = INVENTORY_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.context == *context && cached.checked_at.elapsed() < INVENTORY_CACHE_TTL {
                return cached.available;
            }
        }
    }
    let Ok(output) = run_command_with_timeout(
        context,
        "gh",
        &["auth", "token", "--hostname", GITHUB_HOSTNAME],
        None,
        true,
        INVENTORY_PROBE_TIMEOUT,
    ) else {
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(GitCredentialInventoryCache {
                context: context.clone(),
                checked_at: Instant::now(),
                available: false,
            });
        }
        return false;
    };
    let available = output.status.success()
        && String::from_utf8(output.stdout)
            .ok()
            .map(|token| token.trim_end_matches(['\r', '\n']).to_string())
            .is_some_and(|token| validate_token(&token).is_ok());
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(GitCredentialInventoryCache {
            context: context.clone(),
            checked_at: Instant::now(),
            available,
        });
    }
    available
}

pub(crate) fn validate_materializations(
    selection: &ManagedContextGitCredentialSelection,
    materializations: &[GitCredentialMaterialization],
) -> Result<(), DaemonError> {
    match selection {
        ManagedContextGitCredentialSelection::None if materializations.is_empty() => Ok(()),
        ManagedContextGitCredentialSelection::Selected { credential_ids } => {
            validate_selected_ids(credential_ids)?;
            if materializations.len() != 1 {
                return Err(scm_error(
                    "Git credential payload does not match the selected credentials",
                ));
            }
            let materialization = &materializations[0];
            if materialization.schema_version != MATERIALIZATION_SCHEMA_VERSION
                || materialization.credential_id != GITHUB_CREDENTIAL_ID
                || materialization.hostname != GITHUB_HOSTNAME
            {
                return Err(scm_error("Git credential payload is not supported"));
            }
            validate_token(&materialization.token)?;
            materialization_sha256(materialization)?;
            Ok(())
        }
        _ => Err(scm_error(
            "Git credential payload does not match the launch plan",
        )),
    }
}

pub(crate) fn materialize_git_credentials(
    context: &GitCredentialCommandContext,
    context_id: &str,
    package_sha256: &str,
    selection: &ManagedContextGitCredentialSelection,
    materializations: &[GitCredentialMaterialization],
) -> Result<Vec<ManagedContextGitCredentialReceipt>, DaemonError> {
    validate_materializations(selection, materializations)?;
    if materializations.is_empty() {
        return Ok(Vec::new());
    }
    let materialization = &materializations[0];
    let expected_token_sha256 = token_sha256(&materialization.token);
    let receipt = receipt_for_materialization(context_id, package_sha256, materialization)?;
    let mut binding = binding_for_receipt(&receipt, GitCredentialBindingPhase::Installing);
    let recovering_install = match read_binding(context)? {
        Some(persisted) if !binding_matches_receipt(&persisted, &receipt) => {
            return Err(scm_error(
                "GitHub credential is already bound to another managed context",
            ));
        }
        Some(persisted) if persisted.phase == GitCredentialBindingPhase::Installed => {
            return Ok(vec![receipt]);
        }
        Some(persisted) if persisted.phase == GitCredentialBindingPhase::Removing => {
            return Err(scm_unavailable(
                "GitHub credential removal must finish before import can resume",
            ));
        }
        Some(_) => true,
        None => {
            if read_github_token(context)?.is_some() {
                return Err(scm_error(
                    "refusing to replace an existing GitHub credential on the target kernel",
                ));
            }
            write_binding(context, &binding)?;
            false
        }
    };

    let existing = if recovering_install {
        read_or_reconcile_installing_github_token(context)?
    } else {
        read_github_token(context)?
    };
    if let Some(existing) = existing {
        if token_sha256(&existing) != expected_token_sha256 {
            return Err(scm_error(
                "GitHub credential changed during managed-context import",
            ));
        }
    } else {
        let login = run_command(
            context,
            "gh",
            &[
                "auth",
                "login",
                "--hostname",
                GITHUB_HOSTNAME,
                "--git-protocol",
                "https",
                "--with-token",
            ],
            Some(materialization.token.as_bytes()),
            false,
        )?;
        if !login.status.success() {
            cleanup_failed_install(context, &receipt, &expected_token_sha256)?;
            return Err(scm_error(
                "install GitHub credential on the target kernel failed",
            ));
        }
    }
    let installed = read_github_token(context)?;
    if installed.as_deref().map(token_sha256).as_deref() != Some(expected_token_sha256.as_str()) {
        cleanup_failed_install(context, &receipt, &expected_token_sha256)?;
        return Err(scm_error(
            "installed GitHub credential does not match the transferred credential",
        ));
    }
    if let Err(error) = ensure_github_git_helper(context) {
        cleanup_failed_install(context, &receipt, &expected_token_sha256)?;
        return Err(error);
    }
    binding.phase = GitCredentialBindingPhase::Installed;
    write_binding(context, &binding)?;
    Ok(vec![receipt])
}

pub(crate) fn rollback_git_credentials(
    context: &GitCredentialCommandContext,
    receipts: &[ManagedContextGitCredentialReceipt],
) -> Result<(), DaemonError> {
    let mut failures = Vec::new();
    for receipt in receipts.iter().rev() {
        if receipt.schema_version != RECEIPT_SCHEMA_VERSION
            || receipt.credential_id != GITHUB_CREDENTIAL_ID
            || receipt.hostname != GITHUB_HOSTNAME
        {
            failures.push(scm_error("managed Git credential receipt is invalid"));
            continue;
        }
        let mut binding = match read_binding(context) {
            Ok(Some(binding)) if binding_matches_receipt(&binding, receipt) => binding,
            Ok(None) => match read_github_token(context) {
                Ok(None) => continue,
                Ok(Some(_)) => {
                    failures.push(scm_error(
                        "GitHub credential has no managed-context ownership binding",
                    ));
                    continue;
                }
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            },
            Ok(Some(_)) => {
                failures.push(scm_error(
                    "GitHub credential belongs to another target context",
                ));
                continue;
            }
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        if binding.phase == GitCredentialBindingPhase::Installed {
            match read_github_token(context) {
                Ok(Some(existing)) if token_sha256(&existing) != receipt.token_sha256 => {
                    failures.push(scm_error(
                        "GitHub credential changed after managed-context import",
                    ));
                    continue;
                }
                Ok(_) => {
                    binding.phase = GitCredentialBindingPhase::Removing;
                    if let Err(error) = write_binding(context, &binding) {
                        failures.push(error);
                        continue;
                    }
                }
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            }
        }
        if let Err(error) = remove_owned_github_credential(context, &receipt.token_sha256) {
            failures.push(error);
            continue;
        }
        if let Err(error) = remove_binding(context, receipt) {
            failures.push(error);
        }
    }
    if let Some(first) = failures.first() {
        return Err(scm_unavailable(format!(
            "{} Git credential rollback(s) failed; first failure: {first}",
            failures.len()
        )));
    }
    Ok(())
}

pub(crate) fn receipt_for_materialization(
    context_id: &str,
    package_sha256: &str,
    materialization: &GitCredentialMaterialization,
) -> Result<ManagedContextGitCredentialReceipt, DaemonError> {
    validate_token(&materialization.token)?;
    Ok(ManagedContextGitCredentialReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        context_id: context_id.to_string(),
        package_sha256: package_sha256.to_string(),
        materialization_sha256: materialization_sha256(materialization)?,
        credential_id: materialization.credential_id.clone(),
        hostname: materialization.hostname.clone(),
        token_sha256: token_sha256(&materialization.token),
    })
}

pub(crate) fn materialization_sha256(
    materialization: &GitCredentialMaterialization,
) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(materialization)
        .map_err(|error| scm_error(format!("serialize Git credential materialization: {error}")))?;
    Ok(sha256_bytes(&bytes))
}

fn binding_for_receipt(
    receipt: &ManagedContextGitCredentialReceipt,
    phase: GitCredentialBindingPhase,
) -> GitCredentialBinding {
    GitCredentialBinding {
        schema_version: receipt.schema_version,
        context_id: receipt.context_id.clone(),
        package_sha256: receipt.package_sha256.clone(),
        materialization_sha256: receipt.materialization_sha256.clone(),
        credential_id: receipt.credential_id.clone(),
        hostname: receipt.hostname.clone(),
        token_sha256: receipt.token_sha256.clone(),
        phase,
    }
}

fn binding_matches_receipt(
    binding: &GitCredentialBinding,
    receipt: &ManagedContextGitCredentialReceipt,
) -> bool {
    binding.schema_version == receipt.schema_version
        && binding.context_id == receipt.context_id
        && binding.package_sha256 == receipt.package_sha256
        && binding.materialization_sha256 == receipt.materialization_sha256
        && binding.credential_id == receipt.credential_id
        && binding.hostname == receipt.hostname
        && binding.token_sha256 == receipt.token_sha256
}

fn binding_path(context: &GitCredentialCommandContext) -> PathBuf {
    context
        .home
        .join(".config")
        .join("chariox")
        .join("managed-context-git")
        .join("github.json")
}

fn read_binding(
    context: &GitCredentialCommandContext,
) -> Result<Option<GitCredentialBinding>, DaemonError> {
    let path = binding_path(context);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(scm_io_error(
                "inspect managed Git credential binding",
                error,
            ))
        }
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() as usize > MAX_BINDING_BYTES
    {
        return Err(scm_error("managed Git credential binding is invalid"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| scm_io_error("open managed Git credential binding", error))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_BINDING_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| scm_io_error("read managed Git credential binding", error))?;
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(scm_error("managed Git credential binding is too large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| scm_error("managed Git credential binding is invalid"))
}

fn write_binding(
    context: &GitCredentialCommandContext,
    binding: &GitCredentialBinding,
) -> Result<(), DaemonError> {
    let path = binding_path(context);
    let parent = path
        .parent()
        .ok_or_else(|| scm_error("managed Git credential binding has no parent"))?;
    ensure_private_directory_chain(&context.home, parent)?;
    let bytes = serde_json::to_vec(binding)
        .map_err(|error| scm_error(format!("serialize managed Git credential binding: {error}")))?;
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(scm_error("managed Git credential binding is too large"));
    }
    let temporary = parent.join(".github.json.tmp");
    match fs::symlink_metadata(&temporary) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(&temporary)
                .map_err(|error| scm_io_error("remove stale Git credential binding", error))?;
        }
        Ok(_) => return Err(scm_error("stale Git credential binding is unsafe")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(scm_io_error("inspect stale Git credential binding", error)),
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| scm_io_error("create managed Git credential binding", error))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| scm_io_error("sync managed Git credential binding", error))?;
    fs::rename(&temporary, &path)
        .map_err(|error| scm_io_error("publish managed Git credential binding", error))?;
    sync_directory(parent)
}

fn remove_binding(
    context: &GitCredentialCommandContext,
    receipt: &ManagedContextGitCredentialReceipt,
) -> Result<(), DaemonError> {
    let Some(binding) = read_binding(context)? else {
        return Ok(());
    };
    if !binding_matches_receipt(&binding, receipt) {
        return Err(scm_error(
            "managed Git credential binding belongs to another context",
        ));
    }
    let path = binding_path(context);
    fs::remove_file(&path)
        .map_err(|error| scm_io_error("remove managed Git credential binding", error))?;
    sync_directory(
        path.parent()
            .ok_or_else(|| scm_error("managed Git credential binding has no parent"))?,
    )
}

fn ensure_private_directory_chain(root: &Path, destination: &Path) -> Result<(), DaemonError> {
    if !destination.starts_with(root) {
        return Err(scm_error(
            "managed Git credential binding path escapes HOME",
        ));
    }
    let mut current = root.to_path_buf();
    for component in destination
        .strip_prefix(root)
        .map_err(|_| scm_error("managed Git credential binding path is invalid"))?
        .components()
    {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(scm_error("managed Git credential directory is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    scm_io_error("create managed Git credential directory", error)
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).map_err(
                        |error| scm_io_error("protect managed Git credential directory", error),
                    )?;
                }
                sync_directory(
                    current.parent().ok_or_else(|| {
                        scm_error("managed Git credential directory has no parent")
                    })?,
                )?;
            }
            Err(error) => {
                return Err(scm_io_error(
                    "inspect managed Git credential directory",
                    error,
                ))
            }
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| scm_io_error("sync managed Git credential directory", error))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn validate_selected_ids(credential_ids: &[String]) -> Result<(), DaemonError> {
    if credential_ids != [GITHUB_CREDENTIAL_ID] {
        return Err(scm_error(
            "only the GitHub CLI credential is transferable in this release",
        ));
    }
    Ok(())
}

pub(crate) fn validate_selection(
    selection: &ManagedContextGitCredentialSelection,
) -> Result<(), DaemonError> {
    match selection {
        ManagedContextGitCredentialSelection::None => Ok(()),
        ManagedContextGitCredentialSelection::Selected { credential_ids } => {
            validate_selected_ids(credential_ids)
        }
    }
}

fn validate_token(token: &str) -> Result<(), DaemonError> {
    if token.is_empty()
        || token.len() > MAX_TOKEN_BYTES
        || token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(scm_error("GitHub credential token is invalid"));
    }
    Ok(())
}

fn token_sha256(token: &str) -> String {
    sha256_bytes(token.as_bytes())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_github_token(context: &GitCredentialCommandContext) -> Result<Option<String>, DaemonError> {
    let output = run_command(
        context,
        "gh",
        &["auth", "token", "--hostname", GITHUB_HOSTNAME],
        None,
        true,
    )?;
    if !output.status.success() {
        if github_auth_configuration_is_absent(context)? {
            return Ok(None);
        }
        return Err(scm_unavailable(
            "GitHub credential status could not be verified",
        ));
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| scm_error("GitHub CLI returned a non-text credential"))?;
    let token = token.trim_end_matches(['\r', '\n']).to_string();
    validate_token(&token)?;
    Ok(Some(token))
}

fn github_hosts_path(context: &GitCredentialCommandContext) -> PathBuf {
    context
        .gh_config_dir
        .clone()
        .or_else(|| context.xdg_config_home.as_ref().map(|root| root.join("gh")))
        .unwrap_or_else(|| context.home.join(".config").join("gh"))
        .join("hosts.yml")
}

fn github_auth_configuration_is_absent(
    context: &GitCredentialCommandContext,
) -> Result<bool, DaemonError> {
    let path = github_hosts_path(context);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(scm_io_error("inspect GitHub CLI configuration", error)),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() as usize > MAX_GH_CONFIG_BYTES
    {
        return Err(scm_unavailable(
            "GitHub credential status could not be verified",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| scm_io_error("open GitHub CLI configuration", error))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_GH_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| scm_io_error("read GitHub CLI configuration", error))?;
    if bytes.len() > MAX_GH_CONFIG_BYTES {
        return Err(scm_unavailable(
            "GitHub credential status could not be verified",
        ));
    }
    let document = serde_yaml::from_slice::<serde_yaml::Value>(&bytes)
        .map_err(|_| scm_unavailable("GitHub credential status could not be verified"))?;
    let Some(hosts) = document.as_mapping() else {
        return Err(scm_unavailable(
            "GitHub credential status could not be verified",
        ));
    };
    Ok(!hosts.contains_key(&serde_yaml::Value::String(GITHUB_HOSTNAME.to_string())))
}

fn ensure_github_git_helper(context: &GitCredentialCommandContext) -> Result<(), DaemonError> {
    let output = run_command(
        context,
        "gh",
        &["auth", "setup-git", "--hostname", GITHUB_HOSTNAME],
        None,
        false,
    )?;
    if !output.status.success() {
        return Err(scm_error(
            "configure the GitHub Git credential helper failed",
        ));
    }
    Ok(())
}

fn read_or_reconcile_installing_github_token(
    context: &GitCredentialCommandContext,
) -> Result<Option<String>, DaemonError> {
    match read_github_token(context) {
        Ok(token) => Ok(token),
        Err(_) => {
            logout_owned_github_credential(context)?;
            remove_github_git_helper(context)?;
            match read_github_token(context)? {
                None => Ok(None),
                Some(_) => Err(scm_unavailable(
                    "interrupted GitHub credential installation could not be reconciled",
                )),
            }
        }
    }
}

fn remove_owned_github_credential(
    context: &GitCredentialCommandContext,
    expected_token_sha256: &str,
) -> Result<(), DaemonError> {
    match read_github_token(context) {
        Ok(Some(existing)) => {
            if token_sha256(&existing) != expected_token_sha256 {
                return Err(scm_error(
                    "GitHub credential changed after managed-context import",
                ));
            }
            logout_owned_github_credential(context)?;
        }
        Ok(None) => {}
        Err(_) => {
            logout_owned_github_credential(context)?;
        }
    }
    remove_github_git_helper(context)?;
    if read_github_token(context)?.is_some() {
        return Err(scm_unavailable(
            "transferred GitHub credential remains after rollback",
        ));
    }
    Ok(())
}

fn logout_owned_github_credential(
    context: &GitCredentialCommandContext,
) -> Result<(), DaemonError> {
    let logout = run_command(
        context,
        "gh",
        &["auth", "logout", "--hostname", GITHUB_HOSTNAME],
        None,
        false,
    )?;
    if !logout.status.success() {
        return Err(scm_unavailable(
            "remove transferred GitHub credential failed",
        ));
    }
    Ok(())
}

fn remove_github_git_helper(context: &GitCredentialCommandContext) -> Result<(), DaemonError> {
    for (section, pattern) in [
        (
            "credential.https://github.com",
            r"^credential\.https://github\.com\.",
        ),
        (
            "credential.https://gist.github.com",
            r"^credential\.https://gist\.github\.com\.",
        ),
    ] {
        if !git_config_pattern_exists(context, pattern)? {
            continue;
        }
        let output = run_command(
            context,
            "git",
            &["config", "--global", "--remove-section", section],
            None,
            false,
        )?;
        if !output.status.success() || git_config_pattern_exists(context, pattern)? {
            return Err(scm_unavailable(
                "remove managed GitHub Git credential helper failed",
            ));
        }
    }
    Ok(())
}

fn git_config_pattern_exists(
    context: &GitCredentialCommandContext,
    pattern: &str,
) -> Result<bool, DaemonError> {
    let output = run_command(
        context,
        "git",
        &["config", "--global", "--get-regexp", pattern],
        None,
        false,
    )?;
    if output.status.success() {
        Ok(true)
    } else if output.status.code() == Some(1) {
        Ok(false)
    } else {
        Err(scm_unavailable(
            "inspect managed GitHub Git credential helper failed",
        ))
    }
}

fn cleanup_failed_install(
    context: &GitCredentialCommandContext,
    receipt: &ManagedContextGitCredentialReceipt,
    expected_token_sha256: &str,
) -> Result<(), DaemonError> {
    match read_github_token(context) {
        Ok(Some(existing)) => {
            if token_sha256(&existing) != expected_token_sha256 {
                return Err(scm_unavailable(
                    "GitHub credential changed while a failed import was being recovered",
                ));
            }
            remove_owned_github_credential(context, expected_token_sha256)?;
        }
        Ok(None) => remove_github_git_helper(context)?,
        Err(_) => remove_owned_github_credential(context, expected_token_sha256)?,
    }
    remove_binding(context, receipt)
}

struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
}

struct StdinWriter {
    result: Receiver<std::io::Result<()>>,
    handle: Option<JoinHandle<()>>,
    completed: bool,
}

impl StdinWriter {
    fn start(mut child_stdin: std::process::ChildStdin, input: &[u8]) -> Self {
        let input = Zeroizing::new(input.to_vec());
        let (sender, result) = mpsc::sync_channel(1);
        let handle = std::thread::spawn(move || {
            let outcome = child_stdin
                .write_all(input.as_slice())
                .and_then(|_| child_stdin.write_all(b"\n"));
            let _ = sender.send(outcome);
        });
        Self {
            result,
            handle: Some(handle),
            completed: false,
        }
    }

    fn poll(&mut self) -> Result<(), DaemonError> {
        if self.completed {
            return Ok(());
        }
        match self.result.try_recv() {
            Ok(Ok(())) => {
                self.completed = true;
                Ok(())
            }
            Ok(Err(error)) => Err(scm_io_error(
                "write managed Git credential command input",
                error,
            )),
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                Err(scm_error("managed Git credential input writer failed"))
            }
        }
    }

    fn finish(mut self) -> Result<(), DaemonError> {
        let join_result = self
            .handle
            .take()
            .expect("Git credential input writer handle must exist")
            .join();
        if join_result.is_err() {
            return Err(scm_error("managed Git credential input writer failed"));
        }
        if self.completed {
            return Ok(());
        }
        match self.result.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(scm_io_error(
                "write managed Git credential command input",
                error,
            )),
            Err(_) => Err(scm_error("managed Git credential input writer failed")),
        }
    }
}

fn run_command(
    context: &GitCredentialCommandContext,
    program: &str,
    arguments: &[&str],
    stdin: Option<&[u8]>,
    capture_stdout: bool,
) -> Result<CommandOutput, DaemonError> {
    run_command_with_timeout(
        context,
        program,
        arguments,
        stdin,
        capture_stdout,
        COMMAND_TIMEOUT,
    )
}

fn run_command_with_timeout(
    context: &GitCredentialCommandContext,
    program: &str,
    arguments: &[&str],
    stdin: Option<&[u8]>,
    capture_stdout: bool,
    timeout: Duration,
) -> Result<CommandOutput, DaemonError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let mut command = Command::new(program);
    command.args(arguments);
    configure_command_environment(&mut command, context);
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null());
    configure_child_process_group(&mut command);
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            scm_error(format!(
                "required Git credential command {program} is not installed"
            ))
        } else {
            scm_io_error("start managed Git credential command", error)
        }
    })?;
    let mut writer = if let Some(input) = stdin {
        let Some(child_stdin) = child.stdin.take() else {
            terminate_child_tree(&mut child);
            return Err(scm_error(
                "managed Git credential command has no input pipe",
            ));
        };
        Some(StdinWriter::start(child_stdin, input))
    } else {
        None
    };
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut captured = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut oversized = false;
            loop {
                let read = stdout.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                if captured.len().saturating_add(read) <= MAX_COMMAND_OUTPUT_BYTES {
                    captured.extend_from_slice(&buffer[..read]);
                } else {
                    oversized = true;
                }
            }
            Ok::<_, std::io::Error>((captured, oversized))
        })
    });
    let status = wait_for_child(&mut child, deadline, writer.as_mut());
    let writer_result = writer.map(StdinWriter::finish).unwrap_or(Ok(()));
    let stdout = match reader {
        Some(reader) => {
            let (stdout, oversized) = reader
                .join()
                .map_err(|_| scm_error("managed Git credential output reader failed"))?
                .map_err(|error| {
                    scm_io_error("read managed Git credential command output", error)
                })?;
            if oversized {
                return Err(scm_error(
                    "managed Git credential command output is too large",
                ));
            }
            Ok(stdout)
        }
        None => Ok(Vec::new()),
    };
    let status = status?;
    writer_result?;
    Ok(CommandOutput {
        status,
        stdout: stdout?,
    })
}

fn configure_command_environment(command: &mut Command, context: &GitCredentialCommandContext) {
    command
        .env_clear()
        .env("PATH", &context.path)
        .env("HOME", &context.home)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(path) = &context.xdg_config_home {
        command.env("XDG_CONFIG_HOME", path);
    }
    if let Some(path) = &context.gh_config_dir {
        command.env("GH_CONFIG_DIR", path);
    }
}

fn wait_for_child(
    child: &mut Child,
    deadline: Instant,
    mut writer: Option<&mut StdinWriter>,
) -> Result<ExitStatus, DaemonError> {
    loop {
        if let Some(writer) = writer.as_mut() {
            if let Err(error) = writer.poll() {
                terminate_child_tree(child);
                return Err(error);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_child_descendants(child.id());
                return Ok(status);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                );
            }
            Ok(None) => {
                terminate_child_tree(child);
                return Err(scm_unavailable("managed Git credential command timed out"));
            }
            Err(error) => {
                terminate_child_tree(child);
                return Err(scm_io_error(
                    "wait for managed Git credential command",
                    error,
                ));
            }
        }
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

fn scm_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_managed_context",
        operation: "transfer Git credentials",
        message: message.into(),
        retryable: false,
    }
}

fn scm_unavailable(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_unavailable",
        operation: "transfer Git credentials",
        message: message.into(),
        retryable: true,
    }
}

fn scm_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    scm_unavailable(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_target(
        label: &str,
        initial_token: Option<&str>,
    ) -> (PathBuf, PathBuf, GitCredentialCommandContext) {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-managed-scm-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let bin = root.join("bin");
        let target_home = root.join("target");
        fs::create_dir_all(&bin).expect("create fake command directory");
        fs::create_dir_all(&target_home).expect("create target home");
        if let Some(token) = initial_token {
            fs::write(target_home.join("token"), format!("{token}\n"))
                .expect("write initial target token");
            let gh_config = target_home.join(".config").join("gh");
            fs::create_dir_all(&gh_config).expect("create initial GitHub config");
            fs::write(gh_config.join("hosts.yml"), "github.com:\n  user: test\n")
                .expect("write initial GitHub config");
        }
        let gh = bin.join("gh");
        fs::write(
            &gh,
            r#"#!/bin/sh
set -eu
case "$1 $2" in
  "auth token")
    test ! -f "$HOME/token-error" || exit 2
    test -f "$HOME/token" && /bin/cat "$HOME/token"
    ;;
  "auth login")
    if test -f "$HOME/partial-login"; then
      /bin/cat > /dev/null
      /bin/mkdir -p "$GH_CONFIG_DIR"
      /usr/bin/printf 'github.com:\n  user: test\n' > "$GH_CONFIG_DIR/hosts.yml"
      exit 2
    fi
    /bin/cat > "$HOME/token"
    /bin/mkdir -p "$GH_CONFIG_DIR"
    /usr/bin/printf 'github.com:\n  user: test\n' > "$GH_CONFIG_DIR/hosts.yml"
    test ! -f "$HOME/fail-token-probe-after-login" || /usr/bin/touch "$HOME/token-error"
    ;;
  "auth setup-git") /usr/bin/touch "$HOME/git-helper" ;;
  "auth logout")
    /bin/rm -f "$HOME/token"
    if test -f "$HOME/partial-logout"; then
      /bin/rm -f "$HOME/partial-logout"
      exit 2
    fi
    /bin/rm -f "$GH_CONFIG_DIR/hosts.yml"
    ;;
  *) exit 2 ;;
esac
"#,
        )
        .expect("write fake gh");
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o700))
            .expect("make fake gh executable");
        let git = bin.join("git");
        fs::write(
            &git,
            r#"#!/bin/sh
set -eu
case "$1 $2 $3" in
  "config --global --get-regexp") test -f "$HOME/git-helper" ;;
  "config --global --remove-section")
    test ! -f "$HOME/git-remove-error" || exit 2
    /bin/rm -f "$HOME/git-helper"
    ;;
  *) exit 2 ;;
esac
"#,
        )
        .expect("write fake git");
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700))
            .expect("make fake git executable");
        let context =
            GitCredentialCommandContext::for_tests(target_home.clone(), bin.into_os_string());
        (root, target_home, context)
    }

    fn github_materialization(token: &str) -> GitCredentialMaterialization {
        GitCredentialMaterialization {
            schema_version: MATERIALIZATION_SCHEMA_VERSION,
            credential_id: GITHUB_CREDENTIAL_ID.to_string(),
            hostname: GITHUB_HOSTNAME.to_string(),
            token: token.to_string(),
        }
    }

    fn github_selection() -> ManagedContextGitCredentialSelection {
        ManagedContextGitCredentialSelection::Selected {
            credential_ids: vec![GITHUB_CREDENTIAL_ID.to_string()],
        }
    }

    #[test]
    fn github_materialization_is_redacted_and_exactly_bound() {
        let materialization = github_materialization("github-secret-canary");
        assert!(!format!("{materialization:?}").contains("github-secret-canary"));
        validate_materializations(&github_selection(), &[materialization])
            .expect("validate GitHub materialization");
    }

    #[cfg(unix)]
    #[test]
    fn github_materialization_refuses_a_preexisting_unowned_token() {
        let (root, target_home, target) = fake_target("preexisting", Some("github-secret-canary"));
        let materializations = vec![github_materialization("github-secret-canary")];
        let error = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect_err("preexisting token must not be adopted");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "invalid_managed_context",
                retryable: false,
                ..
            }
        ));
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read preserved token"),
            "github-secret-canary\n"
        );
        assert!(!binding_path(&target).exists());
        fs::remove_dir_all(root).expect("remove preexisting-token fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_materialization_recovers_an_installing_binding() {
        let (root, target_home, target) = fake_target("installing", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipt =
            receipt_for_materialization("context-github", &"a".repeat(64), &materializations[0])
                .expect("build GitHub receipt");
        write_binding(
            &target,
            &binding_for_receipt(&receipt, GitCredentialBindingPhase::Installing),
        )
        .expect("write interrupted install binding");
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("recover GitHub installation");
        assert_eq!(receipts, vec![receipt]);
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read recovered token"),
            "github-secret-canary\n"
        );
        assert_eq!(
            read_binding(&target)
                .expect("read installed binding")
                .map(|binding| binding.phase),
            Some(GitCredentialBindingPhase::Installed)
        );
        fs::remove_dir_all(root).expect("remove interrupted-install fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_materialization_preserves_ownership_when_post_login_probe_fails() {
        let (root, target_home, target) = fake_target("post-login-probe", None);
        fs::write(target_home.join("fail-token-probe-after-login"), b"")
            .expect("arm post-login token probe failure");
        let materializations = vec![github_materialization("github-secret-canary")];
        let error = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect_err("ambiguous post-login probe must remain retryable");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "managed_context_unavailable",
                retryable: true,
                ..
            }
        ));
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read retained token"),
            "github-secret-canary\n"
        );
        assert_eq!(
            read_binding(&target)
                .expect("read retained binding")
                .map(|binding| binding.phase),
            Some(GitCredentialBindingPhase::Installing)
        );

        fs::remove_file(target_home.join("fail-token-probe-after-login"))
            .expect("disarm post-login failure");
        fs::remove_file(target_home.join("token-error")).expect("restore token probe");
        materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("recover retained GitHub credential");
        assert_eq!(
            read_binding(&target)
                .expect("read recovered binding")
                .map(|binding| binding.phase),
            Some(GitCredentialBindingPhase::Installed)
        );
        fs::remove_dir_all(root).expect("remove post-login probe fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_materialization_recovers_partial_login_configuration() {
        let (root, target_home, target) = fake_target("partial-login", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipt =
            receipt_for_materialization("context-github", &"a".repeat(64), &materializations[0])
                .expect("build GitHub receipt");
        write_binding(
            &target,
            &binding_for_receipt(&receipt, GitCredentialBindingPhase::Installing),
        )
        .expect("write interrupted install binding");
        fs::create_dir_all(
            github_hosts_path(&target)
                .parent()
                .expect("GitHub config parent"),
        )
        .expect("create partial GitHub config");
        fs::write(github_hosts_path(&target), "github.com:\n  user: partial\n")
            .expect("write partial GitHub config");

        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("recover partial GitHub login");
        assert_eq!(receipts, vec![receipt]);
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read recovered token"),
            "github-secret-canary\n"
        );
        assert_eq!(
            read_binding(&target)
                .expect("read recovered binding")
                .map(|binding| binding.phase),
            Some(GitCredentialBindingPhase::Installed)
        );
        fs::remove_dir_all(root).expect("remove partial-login fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_materialization_replay_does_not_replace_a_later_token() {
        let (root, target_home, target) = fake_target("rotated", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("materialize GitHub credential");
        fs::write(target_home.join("token"), "user-rotated-token\n").expect("rotate target token");
        assert_eq!(
            materialize_git_credentials(
                &target,
                "context-github",
                &"a".repeat(64),
                &github_selection(),
                &materializations,
            )
            .expect("replay installed GitHub binding"),
            receipts
        );
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read rotated token"),
            "user-rotated-token\n"
        );
        fs::remove_dir_all(root).expect("remove rotated-token fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_rollback_preserves_binding_when_token_probe_fails() {
        let (root, target_home, target) = fake_target("rollback-token-probe", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("materialize GitHub credential");
        fs::write(target_home.join("token-error"), b"").expect("break token probe");

        let error = rollback_git_credentials(&target, &receipts)
            .expect_err("ambiguous token status must keep rollback retryable");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "managed_context_unavailable",
                retryable: true,
                ..
            }
        ));
        assert!(binding_path(&target).exists());
        assert!(target_home.join("token").exists());
        assert!(target_home.join("git-helper").exists());

        fs::remove_file(target_home.join("token-error")).expect("restore token probe");
        rollback_git_credentials(&target, &receipts).expect("retry GitHub rollback");
        assert!(!binding_path(&target).exists());
        assert!(!target_home.join("token").exists());
        assert!(!target_home.join("git-helper").exists());
        fs::remove_dir_all(root).expect("remove rollback token-probe fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_rollback_finishes_helper_cleanup_after_logout_crash() {
        let (root, target_home, target) = fake_target("rollback-after-logout", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("materialize GitHub credential");
        fs::remove_file(target_home.join("token")).expect("simulate completed logout");
        fs::remove_file(github_hosts_path(&target)).expect("remove logged-out host config");
        assert!(target_home.join("git-helper").exists());

        rollback_git_credentials(&target, &receipts).expect("resume GitHub rollback");
        assert!(!target_home.join("git-helper").exists());
        assert!(!binding_path(&target).exists());
        fs::remove_dir_all(root).expect("remove crash-after-logout fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_rollback_recovers_partial_logout_configuration() {
        let (root, target_home, target) = fake_target("partial-logout", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("materialize GitHub credential");
        fs::write(target_home.join("partial-logout"), b"").expect("arm partial logout");

        rollback_git_credentials(&target, &receipts)
            .expect_err("partial logout must retain owned removal state");
        assert!(!target_home.join("token").exists());
        assert!(github_hosts_path(&target).exists());
        assert!(target_home.join("git-helper").exists());
        assert_eq!(
            read_binding(&target)
                .expect("read retained removal binding")
                .map(|binding| binding.phase),
            Some(GitCredentialBindingPhase::Removing)
        );

        rollback_git_credentials(&target, &receipts).expect("resume partial logout");
        assert!(!github_hosts_path(&target).exists());
        assert!(!target_home.join("git-helper").exists());
        assert!(!binding_path(&target).exists());
        fs::remove_dir_all(root).expect("remove partial-logout fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_rollback_keeps_binding_when_helper_cleanup_fails() {
        let (root, target_home, target) = fake_target("rollback-helper-error", None);
        let materializations = vec![github_materialization("github-secret-canary")];
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &github_selection(),
            &materializations,
        )
        .expect("materialize GitHub credential");
        fs::remove_file(target_home.join("token")).expect("simulate completed logout");
        fs::remove_file(github_hosts_path(&target)).expect("remove logged-out host config");
        fs::write(target_home.join("git-remove-error"), b"").expect("break Git helper cleanup");

        rollback_git_credentials(&target, &receipts)
            .expect_err("Git helper failure must keep rollback retryable");
        assert!(target_home.join("git-helper").exists());
        assert!(binding_path(&target).exists());

        fs::remove_file(target_home.join("git-remove-error")).expect("restore Git helper cleanup");
        rollback_git_credentials(&target, &receipts).expect("retry Git helper cleanup");
        assert!(!target_home.join("git-helper").exists());
        assert!(!binding_path(&target).exists());
        fs::remove_dir_all(root).expect("remove helper-cleanup failure fixture");
    }

    #[cfg(unix)]
    #[test]
    fn command_timeout_covers_a_blocked_maximum_stdin_write() {
        use std::os::unix::fs::PermissionsExt;

        let (root, _target_home, target) = fake_target("blocked-stdin", None);
        let program = root.join("bin").join("non-reading");
        fs::write(&program, "#!/bin/sh\n/bin/sleep 10\n").expect("write non-reading command");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700))
            .expect("make non-reading command executable");
        let input = vec![b'x'; MAX_TOKEN_BYTES];
        let started = Instant::now();
        let error = run_command_with_timeout(
            &target,
            "non-reading",
            &[],
            Some(&input),
            false,
            Duration::from_millis(150),
        )
        .err()
        .expect("non-reading command must time out");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "managed_context_unavailable",
                retryable: true,
                ..
            }
        ));
        fs::remove_dir_all(root).expect("remove blocked-stdin fixture");
    }

    #[cfg(unix)]
    #[test]
    fn github_cli_materialization_replays_and_rolls_back_exact_token() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-managed-scm-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let bin = root.join("bin");
        let source_home = root.join("source");
        let target_home = root.join("target");
        fs::create_dir_all(&bin).expect("create fake command directory");
        fs::create_dir_all(&source_home).expect("create source home");
        fs::create_dir_all(&target_home).expect("create target home");
        fs::write(source_home.join("token"), "github-secret-canary\n").expect("write source token");
        let gh = bin.join("gh");
        fs::write(
            &gh,
            r#"#!/bin/sh
set -eu
case "$1 $2" in
  "auth token")
    test -f "$HOME/token" && /bin/cat "$HOME/token"
    ;;
  "auth login")
    /bin/cat > "$HOME/token"
    /bin/mkdir -p "$GH_CONFIG_DIR"
    /usr/bin/printf 'github.com:\n  user: test\n' > "$GH_CONFIG_DIR/hosts.yml"
    ;;
  "auth setup-git") /usr/bin/touch "$HOME/git-helper" ;;
  "auth logout")
    /bin/rm -f "$HOME/token" "$GH_CONFIG_DIR/hosts.yml"
    ;;
  *) exit 2 ;;
esac
"#,
        )
        .expect("write fake gh");
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o700))
            .expect("make fake gh executable");
        let git = bin.join("git");
        fs::write(
            &git,
            r#"#!/bin/sh
set -eu
case "$1 $2 $3" in
  "config --global --get-regexp") test -f "$HOME/git-helper" ;;
  "config --global --remove-section") /bin/rm -f "$HOME/git-helper" ;;
  *) exit 2 ;;
esac
"#,
        )
        .expect("write fake git");
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700))
            .expect("make fake git executable");
        let path = bin.into_os_string();
        let source = GitCredentialCommandContext::for_tests(source_home, path.clone());
        let target = GitCredentialCommandContext::for_tests(target_home.clone(), path);
        let selection = ManagedContextGitCredentialSelection::Selected {
            credential_ids: vec![GITHUB_CREDENTIAL_ID.to_string()],
        };
        let materializations =
            export_selected_git_credentials(&selection, &source).expect("export GitHub credential");
        let receipts = materialize_git_credentials(
            &target,
            "context-github",
            &"a".repeat(64),
            &selection,
            &materializations,
        )
        .expect("materialize GitHub credential");
        assert_eq!(
            fs::read_to_string(target_home.join("token")).expect("read imported token"),
            "github-secret-canary\n"
        );
        assert_eq!(
            materialize_git_credentials(
                &target,
                "context-github",
                &"a".repeat(64),
                &selection,
                &materializations,
            )
            .expect("replay GitHub credential"),
            receipts
        );
        rollback_git_credentials(&target, &receipts).expect("roll back GitHub credential");
        assert!(!target_home.join("token").exists());
        rollback_git_credentials(&target, &receipts).expect("repeat GitHub rollback");
        fs::remove_dir_all(root).expect("remove GitHub fixture");
    }
}
