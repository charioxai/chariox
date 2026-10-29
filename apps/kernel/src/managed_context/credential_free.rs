//! MP-08 / MP-11: detect Vault-routed content; reject plaintext secrets before target mutation.
#[path = "credential_free/config.rs"]
mod config;
mod history;
#[path = "credential_free/json.rs"]
mod json;
#[path = "credential_free/shell.rs"]
mod shell;
#[path = "credential_free/text.rs"]
mod text;
pub(crate) use config::exportable_mcp;
pub(crate) use history::scan_repository;
use shell::{credential_text, unsupported_shell};

use super::development::{
    extract_verified_development_archive, DevelopmentFileState, DevelopmentSourceRepositoryBinding,
};
use super::kernel::KernelContextPayload;
use super::owner_managed::admission_error;
use crate::error::DaemonError;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_SCAN: u64 = 1024 * 1024 * 1024;

fn refused() -> DaemonError {
    admission_error(
        "credential-free context contains credential-bearing or unclassifiable material",
    )
}

fn credential_url(url: &url::Url) -> bool {
    let public_ssh_user = url.scheme() == "ssh" && url.username() == "git";
    (!url.username().is_empty() && !public_ssh_user)
        || url.password().is_some()
        || url.query_pairs().any(|(name, _)| {
            let name = name.to_ascii_lowercase();
            [
                "token",
                "password",
                "secret",
                "credential",
                "api_key",
                "apikey",
                "authorization",
            ]
            .iter()
            .any(|part| name.contains(part))
        })
}

fn credential_urls(text: &str) -> bool {
    text.split_whitespace().any(|word| {
        url::Url::parse(word.trim_matches(['\'', '"', ',', ';']))
            .is_ok_and(|url| credential_url(&url))
    })
}

// MP-08/MP-11: every readable asset run goes through the full text policy
// for this exact path. No asset-only detector list or minimum run length.
fn credential_runs(path: &str, bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .split(|ch: char| {
            ch == char::REPLACEMENT_CHARACTER
                || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\x0c'))
        })
        .any(|run| validate_text(path, run).is_err())
}

// MP-08/MP-11: the free-text policy. Escaped JSON string tokens are decoded
// and checked too, so escaped names in non-JSON text are inspected as well.
fn free_text(text: &str) -> Result<(), DaemonError> {
    if text::credential_format(text) || credential_text(text) || credential_urls(text) {
        return Err(refused());
    }
    json::validate_tokens(text)
}

/// MP-08/MP-11: the one guard for source export, target admission, Git history
/// and packaged Extensions. `Err` means Vault-routed (or refused where no Vault
/// route exists). Content is copied as data only when it is fully classified:
/// strict text that passes the policy for its path, or an allow-listed asset.
pub(crate) fn validate_bytes(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    let name = path.to_ascii_lowercase();
    if text::credential_format(path)
        || credential_text(path)
        || crate::project_environment::secret_looking_project_path(&name)
        || name.split('/').any(|part| {
            matches!(
                part,
                "auth.json" | ".git-credentials" | ".netrc" | "vault.json" | "vault" | "keys"
            )
        })
    {
        return Err(refused());
    }
    match text::classify(&name, bytes) {
        text::Content::Text(text) => validate_text(&name, text),
        text::Content::Asset if !credential_runs(&name, bytes) => Ok(()),
        text::Content::Asset | text::Content::Unclassified => Err(refused()),
    }
}

// MP-08/MP-11: the one path-aware classifier for plain text and asset runs.
fn validate_text(name: &str, text: &str) -> Result<(), DaemonError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let role = json::Role::for_path(name);
    if text::credential_format(text) {
        return Err(refused());
    }
    // Inspect valid JSON structurally: object keys may name dependencies or
    // numeric counters. Scanning JSON syntax as shell assignments loses that role.
    if let Some(value) = json::parse(text)? {
        return json::validate(&value, role);
    }
    // A JSON file is classified only when it parses (comments and trailing
    // commas allowed); its comments get the free-text policy.
    if json::named(name) && !text.trim().is_empty() {
        let (json, comments) = json::strip_comments(text).ok_or_else(refused)?;
        json::validate(&json::parse(&json)?.ok_or_else(refused)?, role)?;
        return free_text(&comments);
    }
    if unsupported_shell(name, text) {
        return Err(refused());
    }
    free_text(text)
}

pub(crate) fn validate_kernel_payload(payload: &KernelContextPayload) -> Result<(), DaemonError> {
    config::validate(payload)
}

struct ScanRoot(PathBuf);
impl Drop for ScanRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scan_root(parent: &Path) -> Result<ScanRoot, DaemonError> {
    let path = parent.join(format!(
        ".owner-context-scan-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&path).map_err(|_| refused())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|_| refused())?;
    }
    Ok(ScanRoot(path))
}

fn scan_unavailable() -> DaemonError {
    DaemonError::ManagedContext {
        operation: "scan Copy history",
        code: "managed_context_unavailable",
        retryable: true,
        message: "Copy admission scan was interrupted; retry the transfer".into(),
    }
}
fn scan_git_status(
    status: std::process::ExitStatus,
    killed_for_refusal: bool,
) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal().is_some() && (!killed_for_refusal || status.signal() != Some(9)) {
            crate::logging::warn_with_fields(
                "daemon.managed_context.owner",
                "MP-08 Copy admission scan interrupted",
                serde_json::json!({"signal": status.signal()}),
            );
            return Err(scan_unavailable());
        }
    }
    #[cfg(not(unix))]
    let _ = (status, killed_for_refusal);
    Ok(())
}
fn git(root: &Path, args: &[&str], limit: u64) -> Result<Vec<u8>, DaemonError> {
    let mut child = crate::process_spawn::command("git")
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| refused())?;
    #[cfg(test)]
    tests::interrupt_scan_child(&mut child, args);
    let mut bytes = Vec::new();
    // Keep the pipe open until the policy kill, so an over-limit scan ends by
    // our SIGKILL rather than a SIGPIPE that would look like an interruption.
    let mut stdout = child.stdout.take().ok_or_else(refused)?;
    let read = (&mut stdout).take(limit + 1).read_to_end(&mut bytes);
    let refused_output = read.is_err() || bytes.len() as u64 > limit;
    // MP-11: signal only this scan's child; distinguish our policy kill
    // from a target shutdown interrupting an otherwise retryable import.
    let killed_for_refusal = refused_output && child.id() > 1 && child.kill().is_ok();
    let status = child.wait().map_err(|_| scan_unavailable())?;
    drop(stdout);
    scan_git_status(status, killed_for_refusal)?;
    if refused_output {
        return Err(refused());
    }
    if !status.success() {
        return Err(refused());
    }
    Ok(bytes)
}

fn validate_bundle(
    root: &Path,
    bundle: &Path,
    index: usize,
    budget: &mut u64,
) -> Result<(), DaemonError> {
    let repository = root.join(format!("repository-{index}"));
    git(
        root,
        &[
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--bare",
            "--no-hardlinks",
            bundle.to_str().ok_or_else(refused)?,
            repository.to_str().ok_or_else(refused)?,
        ],
        MAX_FILE,
    )?;
    if !scan_repository(&repository)?.is_empty() {
        return Err(refused());
    }
    *budget = budget.saturating_add(fs::metadata(bundle).map_err(|_| refused())?.len());
    if *budget > MAX_SCAN {
        return Err(admission_error("Copy exceeds the 1 GiB scan-byte limit"));
    }
    Ok(())
}

pub(crate) fn validate_development_archive(
    path: &Path,
    expected_project_id: &str,
    expected_source_repositories: Option<&[DevelopmentSourceRepositoryBinding]>,
) -> Result<(), DaemonError> {
    let root = scan_root(path.parent().ok_or_else(refused)?)?;
    let artifacts = root.0.join("artifacts");
    // MP-08 / MP-11: decode with the importer itself so every accepted gzip
    // member, declared artifact and limit is enforced before any scan.
    let manifest = extract_verified_development_archive(
        path,
        expected_project_id,
        expected_source_repositories,
        &artifacts,
    )
    .map_err(|_| refused())?;
    // MP-11: sealed Project environment values could contain source secrets.
    // The owner-managed mode transfers code/setup metadata, never this layer.
    if manifest.project_environment.is_some() {
        return Err(refused());
    }
    let sealed = manifest
        .vault_artifacts
        .iter()
        .map(|artifact| artifact.binding.object_path.clone())
        .collect::<BTreeSet<_>>();
    let mut budget = 0u64;
    let mut bundles = Vec::new();
    let mut object_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for repository in manifest.repositories {
        for overlay in repository.overlay {
            for state in [overlay.index, overlay.worktree] {
                if let DevelopmentFileState::File { object_path, .. } = state {
                    object_paths
                        .entry(object_path)
                        .or_default()
                        .insert(overlay.path.clone());
                }
            }
        }
        if let Some(origin) = repository.origin_url {
            validate_bytes("origin-url", origin.as_bytes())?;
        }
        if !repository.bundle_path.is_empty() && !sealed.contains(&repository.bundle_path) {
            bundles.push(artifacts.join(repository.bundle_path));
        }
    }
    for (object, paths) in object_paths {
        if sealed.contains(&object) {
            continue;
        }
        let mut bytes = Vec::new();
        File::open(artifacts.join(object))
            .map_err(|_| refused())?
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| refused())?;
        budget = budget.saturating_add(bytes.len() as u64);
        if bytes.len() as u64 > MAX_FILE || budget > MAX_SCAN {
            return Err(refused());
        }
        // The same object can serve multiple logical files; inspect every
        // role, including both index and worktree overlay references.
        for path in paths {
            validate_bytes(&path, &bytes)?;
        }
    }
    for (index, bundle) in bundles.iter().enumerate() {
        validate_bundle(&root.0, bundle, index, &mut budget)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "credential_free/tests.rs"]
pub(super) mod tests;
