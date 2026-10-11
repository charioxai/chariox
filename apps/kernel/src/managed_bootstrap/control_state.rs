//! MP-03: root-owned deployment state, separate from ordinary user runtime state.
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

use serde::de::DeserializeOwned;

use crate::config::write_private_file;
use crate::error::DaemonError;

const MAX_BYTES: u64 = 96 * 1024;
const PATHS: &[&str] = &[
    "/var/lib/chariox/managed/bootstrap-receipt.json",
    "/var/lib/chariox/managed/bootstrap-grant-binding.json",
    "/var/lib/chariox/managed/release-override.json",
    "/var/lib/chariox/disposable-worker/bootstrap-receipt.json",
    "/var/lib/chariox/disposable-worker/release-override.json",
];

fn protected(path: &Path) -> bool {
    PATHS.iter().any(|allowed| path == Path::new(allowed))
}

fn uses_helper(path: &Path) -> bool {
    if !protected(path)
        || std::env::var(super::MANAGED_PROVIDER_TOPOLOGY_ENV).as_deref() != Ok("path1")
    {
        return false;
    }
    // MP-03/MP-07: the v1 bridge must boot after H/I's unchanged updater,
    // which retains service-principal receipt ownership. A v2 service always
    // uses the helper; v1 also uses it when provisioning supplied private state.
    let version = std::env::var("CHARIOX_PATH1_SERVICE_POLICY_VERSION").ok();
    let parent_uid = path
        .parent()
        .and_then(|parent| fs::metadata(parent).ok())
        .map(|metadata| metadata.uid());
    requires_helper(version.as_deref(), parent_uid)
}

fn requires_helper(version: Option<&str>, parent_uid: Option<u32>) -> bool {
    // Unknown policy values cannot select legacy access.
    !matches!(version, None | Some("1")) || parent_uid == Some(0)
}

fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed control state",
        message: message.into(),
    }
}

fn helper(action: &str, path: &Path, bytes: Option<&[u8]>) -> Result<Vec<u8>, DaemonError> {
    let mut child = Command::new("/usr/bin/timeout")
        .args([
            "10s",
            "/usr/bin/sudo",
            "-n",
            "/usr/local/bin/chariox-managed-bootstrap",
            "--control-state",
            action,
        ])
        .arg(path)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(if bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("root control-state helper could not start"))?;
    let write_result = if let Some(bytes) = bytes {
        child.stdin.take().unwrap().write_all(bytes)
    } else {
        Ok(())
    };
    // Always reap, including a broken input pipe. Never include helper data in errors.
    let output = child
        .wait_with_output()
        .map_err(|_| error("root control-state helper did not settle"))?;
    if write_result.is_err() || !output.status.success() || output.stdout.len() as u64 > MAX_BYTES {
        return Err(error("root control-state helper failed"));
    }
    Ok(output.stdout)
}

pub(crate) fn read_optional<T: DeserializeOwned>(
    path: &Path,
    label: &str,
) -> Result<Option<T>, DaemonError> {
    if !uses_helper(path) {
        return match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(error("control state could not be inspected")),
            Ok(_) => super::state::read_bounded_json(path, label).map(Some),
        };
    }
    let bytes = helper("read", path, None)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| error("control state is invalid"))
}

pub(super) fn exists(path: &Path) -> Result<bool, DaemonError> {
    if uses_helper(path) {
        return helper("read", path, None).map(|bytes| !bytes.is_empty());
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(error("control state could not be inspected")),
    }
}

pub(super) fn persist(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("control state is too large"));
    }
    if uses_helper(path) {
        helper("write", path, Some(bytes)).map(|_| ())
    } else {
        write_private_file(path, bytes).map_err(|_| error("control state write failed"))
    }
}

fn validate_parent(path: &Path) -> Result<(), DaemonError> {
    for ancestor in path
        .parent()
        .ok_or_else(|| error("missing control parent"))?
        .ancestors()
    {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|_| error("control ancestor missing"))?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(error("control ancestor is unsafe"));
        }
    }
    Ok(())
}

fn read_root(path: &Path) -> Result<Vec<u8>, DaemonError> {
    validate_parent(path)?;
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(error("control file could not be opened")),
    };
    let metadata = file
        .metadata()
        .map_err(|_| error("control file could not be inspected"))?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > MAX_BYTES
    {
        return Err(error("control file is not root-private"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("control file read failed"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("control file is too large"));
    }
    Ok(bytes)
}

/// Called before logging initialization; payloads only cross private pipes.
pub fn run(action: &str, path: &Path) -> Result<(), DaemonError> {
    if unsafe { libc::geteuid() } != 0 || !protected(path) {
        return Err(error(
            "control helper requires root and an exact deployment path",
        ));
    }
    match action {
        "read" => std::io::stdout()
            .write_all(&read_root(path)?)
            .map_err(|_| error("control output failed")),
        "write" => {
            validate_parent(path)?;
            // Reject unsafe existing entries, including symlinks and hard links.
            read_root(path)?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| error("control input failed"))?;
            if bytes.len() as u64 > MAX_BYTES
                || serde_json::from_slice::<serde_json::Value>(&bytes).is_err()
            {
                return Err(error("control input is not bounded JSON"));
            }
            write_private_file(path, &bytes).map_err(|_| error("root control write failed"))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| error("control mode failed"))
        }
        _ => Err(error("unknown control-state action")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unix_control_receipt_denies_provider_access_and_keeps_sibling_workspace() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "chariox-control-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(root.join("managed")).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(root.join("workspace"), fs::Permissions::from_mode(0o777)).unwrap();
        let receipt = root.join("managed/bootstrap-receipt.json");
        write_private_file(&receipt, b"{}").unwrap();
        assert_eq!(read_root(&receipt).unwrap(), b"{}");
        let output = Command::new("/usr/bin/setpriv").current_dir(&root)
            .args(["--reuid=999", "--regid=999", "--clear-groups", "/bin/sh", "-c",
                "test ! -r managed/bootstrap-receipt.json && test ! -w managed/bootstrap-receipt.json && test -x . && touch workspace/public-canary"])
            .output().unwrap();
        assert!(
            output.status.success(),
            "provider Unix denial and sibling access"
        );
        fs::set_permissions(&receipt, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(read_root(&receipt).is_err());
        fs::set_permissions(&receipt, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&receipt, root.join("alias")).unwrap();
        assert!(read_root(&receipt).is_err());
        fs::remove_file(root.join("alias")).unwrap();
        fs::remove_file(&receipt).unwrap();
        symlink(root.join("workspace/public-canary"), &receipt).unwrap();
        assert!(read_root(&receipt).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_policy_bridge_keeps_legacy_access_until_private_activation() {
        assert!(!requires_helper(None, Some(1000)));
        assert!(!requires_helper(Some("1"), Some(1000)));
        assert!(requires_helper(Some("1"), Some(0)));
        assert!(requires_helper(Some("2"), Some(1000)));
        assert!(requires_helper(Some("2"), None));
        assert!(requires_helper(Some("3"), Some(1000)));
    }

    #[test]
    fn control_paths_are_exact_and_do_not_include_workspaces() {
        assert!(protected(Path::new(PATHS[0])));
        for path in [
            "/var/lib/chariox",
            "/var/lib/chariox/managed/repo",
            "/var/lib/chariox/managed/../managed/bootstrap-receipt.json",
            "/home/chariox/.chariox/config.json",
        ] {
            assert!(!protected(Path::new(path)));
        }
    }
}
