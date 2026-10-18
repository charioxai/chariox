//! MP-03: root-owned deployment state, separate from ordinary user runtime state.
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

use serde::de::DeserializeOwned;

use crate::config::write_private_file;
use crate::error::DaemonError;

pub(super) const MAX_BYTES: u64 = 96 * 1024;
const PATHS: &[&str] = &[
    "/var/lib/chariox/managed/bootstrap-receipt.json",
    "/var/lib/chariox/managed/bootstrap-grant-binding.json",
    "/var/lib/chariox/managed/release-override.json",
    "/var/lib/chariox/disposable-worker/bootstrap-receipt.json",
    "/var/lib/chariox/disposable-worker/release-override.json",
];

pub(super) fn protected(path: &Path) -> bool {
    PATHS.iter().any(|allowed| path == Path::new(allowed))
}

pub(super) fn uses_helper(path: &Path) -> bool {
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

pub(super) fn error(message: &str) -> DaemonError {
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
        let result = match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(error("control state could not be inspected")),
            Ok(_) => super::state::read_bounded_json(path, label).map(Some),
        };
        if result.is_ok() || !uses_helper(path) {
            return result;
        }
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
        Err(_) if uses_helper(path) => helper("read", path, None).map(|bytes| !bytes.is_empty()),
        Err(_) => Err(error("control state could not be inspected")),
    }
}

/// True when the control directory holds nothing but the receipt and its grant
/// binding. The root helper lists the root-private directory; callers get only
/// the verdict.
pub(super) fn directory_is_clean(receipt: &Path) -> Result<bool, DaemonError> {
    match helper("residue", receipt, None)?.as_slice() {
        b"clean" => Ok(true),
        b"residue" => Ok(false),
        _ => Err(error("control directory verdict is invalid")),
    }
}

fn root_directory_is_clean(receipt: &Path) -> Result<bool, DaemonError> {
    if !super::control_ownership::prepare(receipt)? {
        return Err(error("legacy control transition is still pending"));
    }
    validate_parent(receipt)?;
    only_receipt_and_grant_binding(receipt)
}

fn only_receipt_and_grant_binding(receipt: &Path) -> Result<bool, DaemonError> {
    let parent = receipt
        .parent()
        .ok_or_else(|| error("missing control parent"))?;
    for entry in
        fs::read_dir(parent).map_err(|_| error("control directory could not be inspected"))?
    {
        let name = entry
            .map_err(|_| error("control directory could not be inspected"))?
            .file_name();
        if Some(name.as_os_str()) != receipt.file_name() && name != "bootstrap-grant-binding.json" {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn persist(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("control state is too large"));
    }
    if uses_helper(path) {
        helper("write", path, Some(bytes)).map(|_| ())
    } else {
        match write_private_file(path, bytes) {
            Ok(()) => Ok(()),
            Err(_) if uses_helper(path) => helper("write", path, Some(bytes)).map(|_| ()),
            Err(_) => Err(error("control state write failed")),
        }
    }
}

/// MP-03/MP-07: continue serving the source updater's health checks, then
/// protect control state immediately after its lock/journal has been released.
pub(super) fn wait_for_kernel(
    child: &mut std::process::Child,
    path: &Path,
    topology: super::ManagedProviderTopology,
) -> Result<std::process::ExitStatus, DaemonError> {
    if topology == super::ManagedProviderTopology::Path1 && protected(path) {
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|_| error("kernel transition wait failed"))?
            {
                return Ok(status);
            }
            match helper("prepare", path, None)?.as_slice() {
                b"ready" => break,
                b"pending" => std::thread::sleep(std::time::Duration::from_secs(1)),
                _ => return Err(error("control transition status is invalid")),
            }
        }
    }
    child.wait().map_err(|_| error("kernel wait failed"))
}

pub(super) fn validate_parent(path: &Path) -> Result<(), DaemonError> {
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
    if !super::control_ownership::prepare(path)? {
        return Err(error("legacy control transition is still pending"));
    }
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
    if unsafe { libc::geteuid() } != 0 || unsafe { libc::getegid() } != 0 || !protected(path) {
        return Err(error(
            "control helper requires root and an exact deployment path",
        ));
    }
    let _lock = super::control_ownership::io_lock()?;
    match action {
        "prepare" => std::io::stdout()
            .write_all(if super::control_ownership::prepare(path)? {
                b"ready"
            } else {
                b"pending"
            })
            .map_err(|_| error("control transition output failed")),
        "residue" => std::io::stdout()
            .write_all(if root_directory_is_clean(path)? {
                b"clean"
            } else {
                b"residue"
            })
            .map_err(|_| error("control output failed")),
        "read" => std::io::stdout()
            .write_all(&read_root(path)?)
            .map_err(|_| error("control output failed")),
        "write" => {
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
    fn legacy_control_read_reestablishes_private_ownership_after_upgrade() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        use std::ffi::CString;
        let name = CString::new("chariox").unwrap();
        let mut account = unsafe { libc::getpwnam(name.as_ptr()) };
        if account.is_null() {
            let name = CString::new("nobody").unwrap();
            account = unsafe { libc::getpwnam(name.as_ptr()) };
        }
        assert!(!account.is_null());
        let (uid, gid) = unsafe { ((*account).pw_uid, (*account).pw_gid) };
        let root = std::env::temp_dir().join(format!(
            "chariox-legacy-control-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(root.join("managed")).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let parent = root.join("managed");
        let receipt = parent.join("bootstrap-receipt.json");
        write_private_file(&receipt, b"{}").unwrap();
        for path in [&parent, &receipt, &root.join("workspace")] {
            let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(name.as_ptr(), uid, gid) }, 0);
        }
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let before = Command::new("/usr/bin/setpriv")
            .current_dir(&root)
            .args([
                format!("--reuid={uid}"),
                format!("--regid={gid}"),
                "--clear-groups".into(),
                "/bin/sh".into(),
                "-c".into(),
                "test -r managed/bootstrap-receipt.json && test -w managed/bootstrap-receipt.json"
                    .into(),
            ])
            .status()
            .unwrap();
        assert!(before.success());
        let result = read_root(&receipt);
        let metadata = fs::metadata(&parent).unwrap();
        let check=Command::new("/usr/bin/setpriv").current_dir(&root).args([format!("--reuid={uid}"),format!("--regid={gid}"),"--clear-groups".into(),"/bin/sh".into(),"-c".into(),"test ! -r managed/bootstrap-receipt.json && test ! -w managed/bootstrap-receipt.json && touch workspace/public-canary".into()]).status().unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            result.unwrap(),
            b"{}",
            "legacy receipt must be promoted by the destination reader"
        );
        assert_eq!(metadata.uid(), 0);
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert!(check.success());
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
    fn root_directory_verdict_accepts_only_the_receipt_and_its_grant_binding() {
        let root = std::env::temp_dir().join(format!("chariox-residue-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let receipt = root.join("bootstrap-receipt.json");
        fs::write(&receipt, b"{}").unwrap();
        fs::write(root.join("bootstrap-grant-binding.json"), b"{}").unwrap();
        assert!(only_receipt_and_grant_binding(&receipt).unwrap());
        for residue in ["previous-kernel-state.json", "release-override.json"] {
            fs::write(root.join(residue), b"{}").unwrap();
            assert!(
                !only_receipt_and_grant_binding(&receipt).unwrap(),
                "{residue}"
            );
            fs::remove_file(root.join(residue)).unwrap();
        }
        fs::create_dir(root.join("disposable-worker")).unwrap();
        assert!(!only_receipt_and_grant_binding(&receipt).unwrap());
        fs::remove_dir_all(&root).unwrap();
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
