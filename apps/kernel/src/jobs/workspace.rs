use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// A file's bytes and executable permission. An upload carries only these two, so these are
/// what a job can change and what a returned file is compared by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileState {
    pub(crate) sha256: String,
    pub(crate) executable: bool,
}

pub(crate) type Manifest = BTreeMap<String, FileState>;

/// A file the remote machine returned, to be compared with and applied over the home copy.
pub(crate) struct ReturnedFile {
    pub(crate) bytes: Vec<u8>,
    pub(crate) executable: bool,
}

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

pub(crate) fn file_state(path: &Path) -> Result<Option<FileState>, DaemonError> {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(format!("Cannot read job file: {e}"))),
    };
    let executable = is_executable(&file.metadata().map_err(|e| error(e.to_string()))?);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| error(e.to_string()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(Some(FileState {
        sha256: format!("{:x}", hash.finalize()),
        executable,
    }))
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// Files are addressed relative to the upload root. A selected subdirectory still
/// respects the worktree's ignore rules and the same path checks as a full upload.
pub(crate) struct TransferWorkspace<'a> {
    pub(crate) root: &'a Path,
    worktree: &'a Path,
    prefix: &'a str,
    rules: Vec<String>,
    worktree_rules: Vec<String>,
}

impl<'a> TransferWorkspace<'a> {
    pub(crate) fn new(worktree: &'a Path, root: &'a Path) -> Result<Self, DaemonError> {
        let prefix = root
            .strip_prefix(worktree)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| error("The job's transfer directory must be inside its workspace"))?;
        if !prefix.is_empty() {
            checked_path(worktree, prefix)?;
            // Selecting an upload root must not hide a normally excluded ancestor.
            if prefix.split('/').any(hard_excluded) {
                return Err(error(format!(
                    "Job directory `{}` is excluded from workspace transfers; choose another directory inside the workspace",
                    root.display()
                )));
            }
        }
        let ignore = crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns;
        Ok(Self {
            root,
            worktree,
            prefix,
            rules: ignore(root),
            worktree_rules: if prefix.is_empty() {
                Vec::new()
            } else {
                ignore(worktree)
            },
        })
    }

    pub(crate) fn checked_path(&self, path: &str) -> Result<std::path::PathBuf, DaemonError> {
        checked_path(self.worktree, &self.worktree_path(path))
    }

    pub(crate) fn ignored(&self, path: &str) -> bool {
        use crate::workspace_live_sync_ignore::{
            user_rules_exclude_path, workspace_live_sync_ignored_path,
        };
        workspace_live_sync_ignored_path(path, &self.rules)
            || !self.worktree_rules.is_empty()
                && user_rules_exclude_path(&self.worktree_path(path), &self.worktree_rules)
    }

    fn worktree_path<'b>(&self, path: &'b str) -> std::borrow::Cow<'b, str> {
        if self.prefix.is_empty() {
            path.into()
        } else {
            format!("{}/{path}", self.prefix).into()
        }
    }

    pub(crate) fn snapshot(&self, outputs: &[String]) -> Result<Manifest, DaemonError> {
        let mut manifest = Manifest::new();
        let mut pending = vec![String::new()];
        let mut total = 0u64;
        while let Some(directory) = pending.pop() {
            for item in std::fs::read_dir(self.root.join(&directory))
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
                let ignored = self.ignored(&path);
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
                    if !ignored || outputs.iter().any(|pattern| {
                        crate::workspace_live_sync_ignore::workspace_live_sync_pattern_may_match_below(
                            pattern, &path,
                        )
                    }) {
                        pending.push(path);
                    }
                } else if metadata.is_file() && (!ignored || output) {
                    total = total
                        .saturating_add(item.metadata().map_err(|e| error(e.to_string()))?.len());
                    check_transfer_limit("upload", self.root, total, manifest.len() + 1)?;
                    if let Some(state) = file_state(&item.path())? {
                        manifest.insert(path, state);
                    }
                }
            }
        }
        Ok(manifest)
    }
}

pub(crate) fn check_transfer_limit(
    direction: &str,
    root: &Path,
    bytes: u64,
    files: usize,
) -> Result<(), DaemonError> {
    const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    const MAX_FILES: usize = 100_000;
    if bytes > MAX_BYTES || files > MAX_FILES {
        return Err(error(format!(
            "Job workspace {direction} for `{}` exceeds the limit of 2 GiB ({MAX_BYTES} bytes) and {MAX_FILES} files. Measured at least {bytes} bytes (file count {files}). Ignore generated files or choose a smaller directory.",
            root.display()
        )));
    }
    Ok(())
}

/// `Ok(true)` when the home copy now matches what the job returned, `Ok(false)` when a local
/// change since `base` kept it and the returned file went to the conflicts folder. Applying the
/// same return twice is harmless, so an interrupted return can simply be run again.
pub(crate) fn apply_return(
    root: &Path,
    job_id: &str,
    path: &str,
    base: Option<&FileState>,
    returned: Option<&ReturnedFile>,
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
    let current = file_state(&target)?;
    let returned_state = returned.map(|file| FileState {
        sha256: format!("{:x}", Sha256::digest(&file.bytes)),
        executable: file.executable,
    });
    if current == returned_state {
        return Ok(true);
    }
    if current.as_ref() != base {
        let conflict = format!(".chariox/jobs/{job_id}/conflicts/{path}");
        let conflict = checked_path(root, &conflict)?;
        match returned {
            Some(file) => publish(&conflict, &file.bytes, file.executable, &target)?,
            None => publish(
                &conflict,
                b"Remote job deleted this file; local edit retained.\n",
                false,
                &target,
            )?,
        }
        return Ok(false);
    }
    match returned {
        Some(file) => publish(&target, &file.bytes, file.executable, &target)?,
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

/// Writes `bytes` to `path`. The file keeps the read and write bits of `home_copy` (or of a
/// newly created file when there is none): a job changes the executable bit only, whatever
/// mode its machine happened to give the file.
fn publish(
    path: &Path,
    bytes: &[u8],
    executable: bool,
    home_copy: &Path,
) -> Result<(), DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Job file has no parent"))?;
    std::fs::create_dir_all(parent).map_err(|e| error(e.to_string()))?;
    let temporary = parent.join(format!(
        ".chariox-job-return-{:032x}",
        rand::random::<u128>()
    ));
    let result = std::fs::write(&temporary, bytes)
        .and_then(|()| set_executable(&temporary, home_copy, executable))
        .and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| error(format!("Cannot apply job file: {e}")))
}

#[cfg(unix)]
fn set_executable(file: &Path, home_copy: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let kept = std::fs::metadata(home_copy).or_else(|_| std::fs::metadata(file))?;
    let mode = kept.permissions().mode() & 0o777;
    let mode = match (executable, mode & 0o111 != 0) {
        (false, _) => mode & !0o111,
        (true, true) => mode,
        // Whoever may read the file may run it.
        (true, false) => mode | ((mode & 0o444) >> 2),
    };
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_executable(_: &Path, _: &Path, _: bool) -> std::io::Result<()> {
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    fn file(bytes: &[u8]) -> ReturnedFile {
        ReturnedFile {
            bytes: bytes.to_vec(),
            executable: false,
        }
    }
    #[test]
    fn offload_job_snapshot_ignores_runtime_state_and_includes_declared_artifact() {
        let tree = crate::test_support::TestWorktree::new("job-snapshot");
        let root = tree.path();
        std::fs::create_dir_all(root.join("target/release")).unwrap();
        std::fs::write(root.join("target/release/build-output"), b"binary").unwrap();
        std::fs::write(root.join("source.rs"), b"source").unwrap();
        std::fs::write(root.join(".env"), b"not copied").unwrap();
        let workspace = TransferWorkspace::new(root, root).unwrap();
        let normal = workspace.snapshot(&[]).unwrap();
        assert!(normal.contains_key("source.rs"));
        assert!(!normal.contains_key(".env"));
        assert!(!normal.contains_key("target/release/build-output"));
        let artifacts = workspace.snapshot(&["target/release/*".into()]).unwrap();
        assert!(artifacts.contains_key("target/release/build-output"));
        assert!(!artifacts.contains_key(".env"));
    }
    #[test]
    fn offload_job_snapshot_enters_nested_ignored_folders_below_a_recursive_output_pattern() {
        let tree = crate::test_support::TestWorktree::new("job-snapshot-recursive");
        let root = tree.path();
        std::fs::create_dir_all(root.join("target/release/deps")).unwrap();
        std::fs::write(root.join("target/release/app"), b"binary").unwrap();
        std::fs::write(root.join("target/release/deps/lib.rlib"), b"lib").unwrap();
        let artifacts = TransferWorkspace::new(root, root)
            .unwrap()
            .snapshot(&["target/**".into()])
            .unwrap();
        assert!(artifacts.contains_key("target/release/app"));
        assert!(artifacts.contains_key("target/release/deps/lib.rlib"));
    }
    #[test]
    fn offload_job_return_preserves_local_edit_and_records_remote_conflict() {
        let tree = crate::test_support::TestWorktree::new("job-conflict");
        let root = tree.path();
        std::fs::write(root.join("source.rs"), b"base").unwrap();
        let base = file_state(&root.join("source.rs")).unwrap().unwrap();
        std::fs::write(root.join("source.rs"), b"local edit").unwrap();
        assert!(!apply_return(
            root,
            "job-test",
            "source.rs",
            Some(&base),
            Some(&file(b"remote edit"))
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
        assert!(apply_return(root, "job-test", "new.rs", None, Some(&file(b"new file"))).unwrap());
        assert_eq!(std::fs::read(root.join("new.rs")).unwrap(), b"new file");
    }
    #[test]
    fn offload_job_return_rejects_escape_and_symlink() {
        let tree = crate::test_support::TestWorktree::new("job-return-paths");
        assert!(apply_return(
            tree.path(),
            "job-test",
            "../outside",
            None,
            Some(&file(b"bad"))
        )
        .is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", tree.path().join("link")).unwrap();
            assert!(apply_return(
                tree.path(),
                "job-test",
                "link/outside",
                None,
                Some(&file(b"bad"))
            )
            .is_err());
        }
    }
}
