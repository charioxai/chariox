//! MP-08 / MP-11: fail-closed checks before packaging and before target mutation.
use super::kernel::{KernelContextPayload, KernelExtensionDependency};
use super::owner_managed::admission_error;
use crate::error::DaemonError;
use base64::Engine;
use serde_json::Value;
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

pub(crate) fn validate_bytes(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    let name = path.to_ascii_lowercase();
    if crate::project_environment::secret_looking_project_path(&name)
        || name.split('/').any(|part| {
            matches!(
                part,
                "auth.json" | ".git-credentials" | ".netrc" | "vault.json" | "vault" | "keys"
            )
        })
        || bytes
            .windows(15)
            .any(|window| window == b"PRIVATE KEY-----")
    {
        return Err(refused());
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        if crate::project_environment::contains_secret_configuration(text) {
            return Err(refused());
        }
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            validate_json(&value)?;
        }
        for word in text.split_whitespace() {
            if let Ok(url) = url::Url::parse(word.trim_matches(['\'', '"', ',', ';'])) {
                if !url.username().is_empty() || url.password().is_some() {
                    return Err(refused());
                }
            }
        }
    }
    Ok(())
}

fn validate_json(value: &Value) -> Result<(), DaemonError> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let lower = key.to_ascii_lowercase();
                if [
                    "password",
                    "secret",
                    "token",
                    "credential",
                    "api_key",
                    "apikey",
                    "authorization",
                    "private_key",
                ]
                .iter()
                .any(|part| lower.contains(part))
                    && !value.is_null()
                    && value != ""
                {
                    return Err(refused());
                }
                if key == "path" {
                    validate_bytes(value.as_str().ok_or_else(refused)?, b"")?;
                }
                if matches!(
                    key.as_str(),
                    "content_base64" | "contentBase64" | "source_base64"
                ) {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(value.as_str().ok_or_else(refused)?)
                        .map_err(|_| refused())?;
                    if decoded.len() as u64 > MAX_FILE {
                        return Err(refused());
                    }
                    validate_bytes("package-content", &decoded)?;
                } else {
                    validate_json(value)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_json(value)?;
            }
        }
        Value::String(text) => {
            if crate::project_environment::contains_secret_configuration(text) {
                return Err(refused());
            }
            if let Ok(url) = url::Url::parse(text) {
                if !url.username().is_empty() || url.password().is_some() {
                    return Err(refused());
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_kernel_payload(payload: &KernelContextPayload) -> Result<(), DaemonError> {
    if payload.vault.is_some()
        || payload
            .dependencies
            .iter()
            .any(|dependency| matches!(dependency, KernelExtensionDependency::Credential { .. }))
    {
        return Err(refused());
    }
    // MP-11: inspect decoded package files as well as metadata; never echo content.
    let value = serde_json::to_value((&payload.extensions, &payload.dependencies))
        .map_err(|_| refused())?;
    validate_json(&value)
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
    let objects = git(&repository, &["rev-list", "--objects", "--all"], MAX_FILE)?;
    let objects = std::str::from_utf8(&objects).map_err(|_| refused())?;
    if objects.lines().count() > 10_000 {
        return Err(refused());
    }
    for line in objects.lines() {
        let (oid, path) = line.split_once(' ').unwrap_or((line, ""));
        validate_bytes(path, b"")?;
        let kind = git(&repository, &["cat-file", "-t", oid], 128)?;
        if matches!(kind.as_slice(), b"blob\n" | b"commit\n" | b"tag\n") {
            let contents = git(&repository, &["cat-file", "-p", oid], MAX_FILE)?;
            *budget = budget.saturating_add(contents.len() as u64);
            if *budget > MAX_SCAN {
                return Err(refused());
            }
            validate_bytes(path, &contents)?;
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
                }
                if let Some(origin) = repository.origin_url {
                    validate_bytes("origin-url", origin.as_bytes())?;
                }
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
mod tests {
    use super::*;
    #[test]
    fn mp11_owner_context_refuses_credentials_and_environment_files() {
        for (path, bytes) in [
            (".env", b"canary-value".as_slice()),
            ("auth.json", b"canary-value"),
            ("file", b"API_KEY=synthetic-canary"),
            ("file", b"-----BEGIN PRIVATE KEY-----"),
            ("file", b"https://user:synthetic-canary@example.test"),
        ] {
            assert!(validate_bytes(path, bytes).is_err());
        }
        assert!(validate_bytes("README.md", b"ordinary project instructions").is_ok());
    }
    #[test]
    fn mp08_mp11_owner_package_refuses_credentials_in_git_history() {
        crate::test_support::isolated_env_test!();
        let _lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owner-history-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir_all(root.join("project")).unwrap();
        let _cleanup = ScanRoot(root.clone());
        let project = root.join("project");
        git(&project, &["init", "-b", "main"], MAX_FILE).unwrap();
        git(
            &project,
            &["config", "user.name", "Synthetic fixture"],
            MAX_FILE,
        )
        .unwrap();
        git(
            &project,
            &["config", "user.email", "fixture@example.test"],
            MAX_FILE,
        )
        .unwrap();
        fs::write(
            project.join("README.md"),
            "API_KEY=synthetic-history-canary\n",
        )
        .unwrap();
        git(&project, &["add", "README.md"], MAX_FILE).unwrap();
        git(&project, &["commit", "-m", "fixture"], MAX_FILE).unwrap();
        fs::write(
            project.join("README.md"),
            "Ordinary current project content\n",
        )
        .unwrap();
        git(&project, &["add", "README.md"], MAX_FILE).unwrap();
        git(
            &project,
            &["commit", "-m", "remove fixture value"],
            MAX_FILE,
        )
        .unwrap();
        let archive = super::super::development::export_development_context(
            super::super::development::DevelopmentContextExportRequest {
                project_id: "project".into(),
                repositories: vec![super::super::development::DevelopmentRepositorySelection {
                    workspace_id: project.display().to_string(),
                    worktree_id: None,
                    worktree_path: project,
                    role: super::super::development::DevelopmentRepositoryRole::Primary,
                }],
                archive_path: root.join("development.tar.gz"),
            },
        )
        .unwrap();
        assert!(validate_development_archive(&archive.archive_path).is_err());
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".owner-context-scan-")));
    }
}
