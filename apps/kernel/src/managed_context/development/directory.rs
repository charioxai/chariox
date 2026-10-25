//! Directory workspaces use the same checked file objects as Git overlays.
//! They never initialize Git or synthesize commits at either endpoint.
use super::*;

pub(super) fn export_directory(
    selection: &DevelopmentRepositorySelection,
    worktree: &Path,
    staging_root: &Path,
    repository_ids: &mut BTreeSet<String>,
    target_directories: &mut BTreeSet<String>,
    budget: &mut ManifestMemoryBudget,
    vault: Option<&vault_artifacts::ExportVault<'_>>,
) -> Result<
    (
        DevelopmentRepositoryManifest,
        RepositoryMaterializationEstimate,
    ),
    DaemonError,
> {
    let logical_name = export::repository_logical_name(worktree, None);
    let source_basename = export::source_repository_basename(worktree)?;
    let repository_id =
        export::unique_repository_id(None, "directory", &logical_name, repository_ids);
    let target_directory =
        export::target_directory_for_export(&source_basename, target_directories)?;
    budget.consume(logical_name.len() + target_directory.len() + 2048)?;
    let root = open_root(worktree)?;
    let (entries, directories, size, pending) =
        snapshot(worktree, &root, &repository_id, staging_root, budget, vault)?;
    overlay::stage_objects(staging_root, &entries, pending, vault)?;
    let verified = {
        let (entries, directories, size, _) = snapshot(
            worktree,
            &root,
            &repository_id,
            staging_root,
            &mut ManifestMemoryBudget::new(),
            vault,
        )?;
        (entries, directories, size)
    };
    // The verification pass compares digests only; it stages nothing.
    if (&entries, &directories, size) != (&verified.0, &verified.1, verified.2)
        || fs::symlink_metadata(worktree.join(".git")).is_ok()
    {
        return Err(context_error(
            "directory workspace changed while exporting; retry",
        ));
    }
    let estimate = charge_overlay_materialization(
        RepositoryMaterializationEstimate {
            checkout_bytes: directories.len() as u64 * 4096,
            materialized_entries: directories.len() as u64,
        },
        &entries,
        UNBOUNDED_BYTES,
        u64::MAX,
    )?;
    Ok((
        DevelopmentRepositoryManifest {
            workspace_kind: DevelopmentWorkspaceKind::Directory,
            directories,
            repository_id,
            source_binding_sha256: source_repository_binding_sha256(
                &DevelopmentSourceRepositoryBinding {
                    role: selection.role,
                    workspace_id: selection.workspace_id.clone(),
                    worktree_id: selection.worktree_id.clone(),
                },
            ),
            logical_name,
            role: selection.role,
            target_directory,
            // Legacy Git fields remain empty for schema-3 directory workspaces.
            head_sha: String::new(),
            branch: None,
            upstream: None,
            origin_url: None,
            bundle_path: String::new(),
            bundle_sha256: String::new(),
            bundle_size_bytes: 0,
            overlay: entries,
            overlay_size_bytes: size,
        },
        estimate,
    ))
}

pub(super) type SnapshotOverlay = (
    Vec<DevelopmentOverlayEntry>,
    Vec<String>,
    u64,
    BTreeMap<String, zeroize::Zeroizing<Vec<u8>>>,
);

pub(super) fn snapshot(
    worktree: &Path,
    root: &File,
    repository_id: &str,
    staging: &Path,
    budget: &mut ManifestMemoryBudget,
    vault: Option<&vault_artifacts::ExportVault<'_>>,
) -> Result<SnapshotOverlay, DaemonError> {
    let patterns = overlay::read_context_ignore_patterns(worktree)?;
    let mut pending = vec![PathBuf::new()];
    let mut directories = BTreeSet::new();
    let mut files = BTreeSet::new();
    let started = std::time::Instant::now();
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(worktree.join(&relative))
            .map_err(|error| context_io_error("enumerate workspace directory", error))?
        {
            let entry =
                entry.map_err(|error| context_io_error("read workspace directory entry", error))?;
            if started.elapsed() > std::time::Duration::from_secs(120) {
                return Err(context_error(
                    "directory scan exceeded the 120 second time limit",
                ));
            }
            let path = relative.join(entry.file_name());
            let path_text = path
                .to_str()
                .ok_or_else(|| context_error("directory workspace entry is not UTF-8"))?
                .to_string();
            validate_relative_path(&path_text)?;
            if overlay::context_force_excluded_path(&path_text)
                || (vault.is_none()
                    && crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(
                        &path_text,
                    ))
            {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| context_io_error("inspect directory workspace entry", error))?;
            let rule_path = if file_type.is_dir() {
                format!("{path_text}/")
            } else {
                path_text.clone()
            };
            let excluded =
                crate::workspace_live_sync_ignore::user_rules_exclude_path(&rule_path, &patterns);
            if excluded
                && (!file_type.is_dir() || !overlay::may_include_descendant(&patterns, &path_text))
            {
                continue;
            }
            // Resolve every component under the retained root, without following
            // symlinks, even if a directory was replaced during enumeration.
            let file = open_relative(root, &path)
                .map_err(|error| context_io_error("open directory workspace entry", error))?;
            let metadata = file
                .metadata()
                .map_err(|error| context_io_error("inspect directory workspace entry", error))?;
            budget.consume(path_text.len() + 256)?;
            if metadata.is_dir() {
                if !excluded {
                    directories.insert(path_text);
                }
                pending.push(path);
            } else if metadata.is_file() {
                files.insert(path_text);
            } else {
                return Err(context_error("directory workspace contains a special file"));
            }
        }
    }
    // Selected children of ignored directories still need declared parents.
    // Do not publish unrelated ignored directories that were only traversed.
    let mut parents = BTreeSet::new();
    for path in directories.iter().chain(files.iter()) {
        let mut current = path.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            if !directories.contains(parent) && parents.insert(parent.to_string()) {
                budget.consume(parent.len() + 256)?;
            }
            current = parent;
        }
    }
    directories.extend(parents);
    let mut stored = BTreeSet::new();
    let mut pending_objects = BTreeMap::new();
    let mut size = 0;
    let mut entries = Vec::new();
    for path in files {
        let file = open_relative(root, Path::new(&path))
            .map_err(|error| context_io_error("open workspace file", error))?;
        let (bytes, metadata) = read_bounded(file, MAX_OVERLAY_FILE_BYTES)?;
        overlay::validate_overlay_file_bytes(&path, &bytes)?;
        let state = overlay::store_overlay_object(
            repository_id,
            bytes,
            file_is_executable(&metadata),
            staging,
            &mut stored,
            &mut size,
            &mut pending_objects,
        )?;
        budget.consume(overlay::file_state_manifest_bytes(&state))?;
        entries.push(DevelopmentOverlayEntry {
            path,
            index: DevelopmentFileState::Absent,
            worktree: state,
        });
    }
    Ok((
        entries,
        directories.into_iter().collect(),
        size,
        pending_objects,
    ))
}

fn read_bounded(mut file: File, limit: u64) -> Result<(Vec<u8>, fs::Metadata), DaemonError> {
    let metadata = file
        .metadata()
        .map_err(|error| context_io_error("inspect workspace file", error))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(context_error(
            "workspace file must be a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| context_io_error("read workspace file", error))?;
    if bytes.len() as u64 > limit {
        return Err(context_error("workspace file grew beyond its size budget"));
    }
    Ok((bytes, metadata))
}

#[cfg(unix)]
pub(crate) fn open_root(path: &Path) -> Result<File, DaemonError> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| context_io_error("open workspace root", error))
}

#[cfg(unix)]
pub(crate) fn open_relative(root: &File, path: &Path) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut parent = root.try_clone()?;
    let mut parts = path.components().peekable();
    while let Some(component) = parts.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if parts.peek().is_some() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: parent owns a live descriptor, name is NUL-terminated, and a
        // successful openat transfers one new owned descriptor to File.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Ok(parent)
}

#[cfg(not(unix))]
pub(crate) fn open_root(_: &Path) -> Result<File, DaemonError> {
    Err(context_error(
        "directory workspace export requires a Unix host",
    ))
}
#[cfg(not(unix))]
pub(crate) fn open_relative(_: &File, _: &Path) -> io::Result<File> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(all(test, unix))]
#[test]
fn mp11_pinned_root_refuses_leaf_and_ancestor_replacements() {
    use std::os::unix::fs::symlink;
    let fixture = std::env::temp_dir().join(format!("mp11-pinned-{:016x}", rand::random::<u64>()));
    let workspace = fixture.join("workspace");
    let outside = fixture.join("outside");
    fs::create_dir_all(workspace.join("nested")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(workspace.join("nested/input"), b"inside").unwrap();
    fs::write(outside.join("input"), b"outside").unwrap();
    let root = open_root(&workspace).unwrap();
    fs::rename(workspace.join("nested/input"), workspace.join("original")).unwrap();
    symlink(outside.join("input"), workspace.join("nested/input")).unwrap();
    assert!(open_relative(&root, Path::new("nested/input")).is_err());
    fs::rename(workspace.join("nested"), workspace.join("previous")).unwrap();
    symlink(&outside, workspace.join("nested")).unwrap();
    assert!(open_relative(&root, Path::new("nested/input")).is_err());
    fs::remove_dir_all(fixture).unwrap();
}
