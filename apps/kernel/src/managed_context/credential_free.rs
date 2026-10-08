//! MP-08 / MP-11: fail-closed checks before packaging and before target mutation.
#[path = "credential_free/config.rs"]
mod config;
#[path = "credential_free/json.rs"]
mod json;
#[path = "credential_free/shell.rs"]
mod shell;
#[path = "credential_free/text.rs"]
mod text;
pub(crate) use config::exportable_mcp;
use shell::{credential_text, unsupported_shell};

use super::kernel::KernelContextPayload;
use super::owner_managed::admission_error;
use crate::error::DaemonError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    let text = text::decode(bytes)?;
    if text::credential_format(&text) {
        return Err(refused());
    }
    // Inspect valid JSON structurally: object keys may name dependencies or
    // numeric counters. Scanning JSON syntax as shell assignments loses that role.
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        return json::validate(&value, json::Role::for_path(&name));
    }
    if credential_text(&text) || unsupported_shell(&name, &text) {
        return Err(refused());
    }
    for word in text.split_whitespace() {
        if let Ok(url) = url::Url::parse(word.trim_matches(['\'', '"', ',', ';'])) {
            if credential_url(&url) {
                return Err(refused());
            }
        }
    }
    Ok(())
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

fn git(root: &Path, args: &[&str], limit: u64) -> Result<Vec<u8>, DaemonError> {
    let mut child = Command::new("git")
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
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or_else(refused)?
        .take(limit + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > limit {
        // MP-11: signal only the verified child this scan spawned.
        if child.id() > 1 {
            let _ = child.kill();
        }
        let _ = child.wait();
        return Err(refused());
    }
    if !child.wait().map_err(|_| refused())?.success() {
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
    // Clone can import pack objects without installing their advertised refs.
    // Inspect the entire isolated object store, including unreachable objects;
    // path hints are also incomplete when a blob has multiple historical names.
    let objects = git(
        &repository,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
        MAX_FILE,
    )?;
    let objects = std::str::from_utf8(&objects).map_err(|_| refused())?;
    if objects.lines().count() > 10_000 {
        return Err(refused());
    }
    let mut kinds = BTreeMap::new();
    let mut blob_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut associations = 0usize;
    for oid in objects.lines() {
        let kind = git(&repository, &["cat-file", "-t", oid], 128)?;
        if kind == b"tree\n" {
            // Include every tree in the pack, regardless of ref reachability.
            // NUL framing preserves literal filenames, including tabs/newlines.
            let tree = git(
                &repository,
                &["ls-tree", "-r", "-t", "-z", "--full-tree", oid],
                MAX_FILE,
            )?;
            *budget = budget.saturating_add(tree.len() as u64);
            if *budget > MAX_SCAN {
                return Err(refused());
            }
            for entry in tree
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
            {
                let entry = std::str::from_utf8(entry).map_err(|_| refused())?;
                let (metadata, path) = entry.split_once('\t').ok_or_else(refused)?;
                validate_bytes(path, b"")?;
                let mut metadata = metadata.split_whitespace();
                let _mode = metadata.next().ok_or_else(refused)?;
                let kind = metadata.next().ok_or_else(refused)?;
                let blob = metadata.next().ok_or_else(refused)?;
                if metadata.next().is_some() {
                    return Err(refused());
                }
                if kind == "blob"
                    && blob_paths
                        .entry(blob.to_owned())
                        .or_default()
                        .insert(path.to_owned())
                {
                    associations += 1;
                    if associations > 100_000 {
                        return Err(refused());
                    }
                }
            }
        }
        kinds.insert(oid, kind);
    }
    for (oid, kind) in kinds {
        if matches!(kind.as_slice(), b"blob\n" | b"commit\n" | b"tag\n") {
            let contents = git(&repository, &["cat-file", "-p", oid], MAX_FILE)?;
            let paths = blob_paths.get(oid);
            // Bound both content reads and repeated path-sensitive inspection.
            *budget = budget.saturating_add(
                (contents.len() as u64).saturating_mul(paths.map_or(1, |paths| paths.len() as u64)),
            );
            if *budget > MAX_SCAN {
                return Err(refused());
            }
            if let Some(paths) = paths {
                for path in paths {
                    validate_bytes(path, &contents)?;
                }
            } else {
                validate_bytes("", &contents)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_development_archive(path: &Path) -> Result<(), DaemonError> {
    let root = scan_root(path.parent().ok_or_else(refused)?)?;
    let input = File::open(path).map_err(|_| refused())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(input));
    let mut budget = 0u64;
    let mut bundles = Vec::new();
    let mut manifest_seen = false;
    let mut object_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (index, entry) in archive.entries().map_err(|_| refused())?.enumerate() {
        if index > 100_000 {
            return Err(refused());
        }
        let mut entry = entry.map_err(|_| refused())?;
        let name = entry
            .path()
            .map_err(|_| refused())?
            .to_string_lossy()
            .into_owned();
        // The canonical exporter writes the manifest first. Refuse unknown
        // ordering rather than classify content-addressed overlays without it.
        if index == 0 && name != "manifest.json" {
            return Err(refused());
        }
        let size = entry.size();
        budget = budget.saturating_add(size);
        if budget > MAX_SCAN || !entry.header().entry_type().is_file() {
            return Err(refused());
        }
        if name.ends_with("/repository.bundle") {
            let bundle = root.0.join(format!("bundle-{index}"));
            let mut file = File::create(&bundle).map_err(|_| refused())?;
            std::io::copy(&mut entry, &mut file).map_err(|_| refused())?;
            bundles.push(bundle);
            continue;
        }
        if size > MAX_FILE {
            return Err(refused());
        }
        let mut bytes = Vec::new();
        entry
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| refused())?;
        if name == "manifest.json" {
            if manifest_seen {
                return Err(refused());
            }
            manifest_seen = true;
            let manifest: super::development::DevelopmentContextManifest =
                serde_json::from_slice(&bytes).map_err(|_| refused())?;
            // MP-11: sealed Project environment values could contain source secrets.
            // The owner-managed mode transfers code/setup metadata, never this layer.
            if manifest.project_environment.is_some() {
                return Err(refused());
            }
            for repository in manifest.repositories {
                for overlay in repository.overlay {
                    validate_bytes(&overlay.path, b"")?;
                    for state in [overlay.index, overlay.worktree] {
                        if let super::development::DevelopmentFileState::File {
                            object_path, ..
                        } = state
                        {
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
            }
        } else if let Some(paths) = object_paths.get(&name) {
            // The same object can serve multiple logical files; inspect every
            // role, including both index and worktree overlay references.
            for path in paths {
                validate_bytes(path, &bytes)?;
            }
        } else {
            validate_bytes(&name, &bytes)?;
        }
    }
    if !manifest_seen {
        return Err(refused());
    }
    for (index, bundle) in bundles.iter().enumerate() {
        validate_bundle(&root.0, bundle, index, &mut budget)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "credential_free/tests.rs"]
pub(super) mod tests;
