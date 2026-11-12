use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};

pub(crate) type Manifest = BTreeMap<String, String>;

pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: "job_invalid".into(),
        message: message.into(),
    }
}

pub(crate) fn relative(path: &str) -> Result<&str, DaemonError> {
    if path.is_empty()
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(error("Job paths must stay relative to the workspace"));
    }
    Ok(path)
}

pub(crate) fn digest(path: &Path) -> Result<Option<String>, DaemonError> {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(format!("Cannot read job file: {e}"))),
    };
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| error(e.to_string()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(Some(format!("{:x}", hash.finalize())))
}

pub(crate) fn snapshot(root: &Path, outputs: &[String]) -> Result<Manifest, DaemonError> {
    let rules = crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(root);
    let mut manifest = Manifest::new();
    let mut pending = vec![String::new()];
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        for item in std::fs::read_dir(root.join(&directory))
            .map_err(|e| error(format!("Cannot scan job workspace: {e}")))?
        {
            let item = item.map_err(|e| error(format!("Cannot scan job file: {e}")))?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| error("Job workspace filenames must be UTF-8"))?;
            let path = if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            };
            let metadata = item.file_type().map_err(|e| error(e.to_string()))?;
            if metadata.is_symlink() {
                continue;
            }
            let ignored =
                crate::workspace_live_sync_ignore::workspace_live_sync_ignored_path(&path, &rules);
            let output = outputs.iter().any(|pattern| {
                crate::workspace_live_sync_ignore::workspace_live_sync_ignore_pattern_matches(
                    pattern, &path,
                )
            });
            if hard_excluded(&path) {
                continue;
            }
            if metadata.is_dir() {
                // Traverse an ignored build directory only when a declared output lies beneath it.
                if !ignored
                    || outputs
                        .iter()
                        .any(|pattern| pattern.starts_with(&format!("{path}/")))
                {
                    pending.push(path);
                }
            } else if metadata.is_file() && (!ignored || output) {
                total =
                    total.saturating_add(item.metadata().map_err(|e| error(e.to_string()))?.len());
                if total > 2 * 1024 * 1024 * 1024 || manifest.len() >= 100_000 {
                    return Err(error("Job workspace exceeds the 2 GiB or 100000 file transfer limit; ignore generated files"));
                }
                if let Some(hash) = digest(&item.path())? {
                    manifest.insert(path, hash);
                }
            }
        }
    }
    Ok(manifest)
}

pub(crate) fn apply_return(
    root: &Path,
    job_id: &str,
    path: &str,
    base: Option<&str>,
    returned: Option<&[u8]>,
) -> Result<bool, DaemonError> {
    relative(path)?;
    if !job_id.starts_with("job-")
        || !job_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(error("Invalid job identity"));
    }
    let target = checked_path(root, path)?;
    let current = digest(&target)?;
    if current.as_deref() != base {
        let conflict = format!(".chariox/jobs/{job_id}/conflicts/{path}");
        let conflict = checked_path(root, &conflict)?;
        publish(
            &conflict,
            returned.unwrap_or(b"Remote job deleted this file; local edit retained.\n"),
        )?;
        return Ok(false);
    }
    match returned {
        Some(bytes) => publish(&target, bytes)?,
        None if current.is_some() => std::fs::remove_file(&target)
            .map_err(|e| error(format!("Cannot return job deletion: {e}")))?,
        None => {}
    }
    Ok(true)
}

pub(crate) fn checked_path(root: &Path, path: &str) -> Result<std::path::PathBuf, DaemonError> {
    relative(path)?;
    let mut target = root.to_path_buf();
    for component in Path::new(path).components() {
        target.push(component);
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(error("A job file path crosses a symbolic link; keep the local file and inspect the result")),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(error(e.to_string())),
        }
    }
    Ok(target)
}

fn publish(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Job file has no parent"))?;
    std::fs::create_dir_all(parent).map_err(|e| error(e.to_string()))?;
    let temporary = parent.join(format!(
        ".chariox-job-return-{:032x}",
        rand::random::<u128>()
    ));
    let result = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| error(format!("Cannot apply job file: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offload_job_snapshot_ignores_runtime_state_and_includes_declared_artifact() {
        let tree = crate::test_support::TestWorktree::new("job-snapshot");
        let root = tree.path();
        std::fs::create_dir_all(root.join("target/release")).unwrap();
        std::fs::write(root.join("target/release/build-output"), b"binary").unwrap();
        std::fs::write(root.join("source.rs"), b"source").unwrap();
        std::fs::write(root.join(".env"), b"not copied").unwrap();
        let normal = snapshot(root, &[]).unwrap();
        assert!(normal.contains_key("source.rs"));
        assert!(!normal.contains_key(".env"));
        assert!(!normal.contains_key("target/release/build-output"));
        let artifacts = snapshot(root, &["target/release/*".into()]).unwrap();
        assert!(artifacts.contains_key("target/release/build-output"));
        assert!(!artifacts.contains_key(".env"));
    }
    #[test]
    fn offload_job_return_preserves_local_edit_and_records_remote_conflict() {
        let tree = crate::test_support::TestWorktree::new("job-conflict");
        let root = tree.path();
        std::fs::write(root.join("source.rs"), b"base").unwrap();
        let base = digest(&root.join("source.rs")).unwrap().unwrap();
        std::fs::write(root.join("source.rs"), b"local edit").unwrap();
        assert!(!apply_return(
            root,
            "job-test",
            "source.rs",
            Some(&base),
            Some(b"remote edit")
        )
        .unwrap());
        assert_eq!(
            std::fs::read(root.join("source.rs")).unwrap(),
            b"local edit"
        );
        assert_eq!(
            std::fs::read(root.join(".chariox/jobs/job-test/conflicts/source.rs")).unwrap(),
            b"remote edit"
        );
        assert!(apply_return(root, "job-test", "new.rs", None, Some(b"new file")).unwrap());
        assert_eq!(std::fs::read(root.join("new.rs")).unwrap(), b"new file");
    }
    #[test]
    fn offload_job_return_rejects_escape_and_symlink() {
        let tree = crate::test_support::TestWorktree::new("job-return-paths");
        assert!(apply_return(tree.path(), "job-test", "../outside", None, Some(b"bad")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", tree.path().join("link")).unwrap();
            assert!(
                apply_return(tree.path(), "job-test", "link/outside", None, Some(b"bad")).is_err()
            );
        }
    }
}

pub(crate) fn hard_excluded(path: &str) -> bool {
    path.split('/').any(|part| {
        matches!(
            part,
            ".git" | ".chariox" | ".codex" | ".claude" | ".opencode" | ".cursor" | ".ssh"
        )
    }) || (crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(path)
        && !crate::workspace_live_sync_ignore::workspace_runtime_excluded_path(path))
}
