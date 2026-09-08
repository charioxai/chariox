//! Empty development selection owns a persistent plain directory, not a Git
//! checkout and not a bind mount of the caller's source workspace.
use super::*;
use crate::slice::SliceDevelopmentPublication;

fn publication(root: &Path) -> Result<SliceDevelopmentPublication, DaemonError> {
    let destination = root.join(SLICE_DEVELOPMENT_PUBLICATION_ID);
    let workspace = path_to_string(&destination.join("workspace"))?;
    Ok(SliceDevelopmentPublication {
        publication_id: SLICE_DEVELOPMENT_PUBLICATION_ID.into(),
        destination_root: path_to_string(&destination)?,
        primary_repository_path: workspace.clone(),
        repository_paths: vec![workspace],
    })
}

fn validate_binding(
    root: &Path,
    expected: Option<&SliceDevelopmentPublication>,
) -> Result<SliceDevelopmentPublication, DaemonError> {
    if !root.is_absolute()
        || root.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(slice_development_error(
            "empty slice storage root must be absolute and normalized",
        ));
    }
    let actual = publication(root)?;
    if expected.is_some_and(|expected| expected != &actual) {
        return Err(slice_development_error(
            "empty slice publication does not match durable state",
        ));
    }
    Ok(actual)
}

// Check canonical identity before chmod or following any child path. Never
// recreate a receipted directory: that would silently lose an existing workspace.
fn directory(path: &Path, may_create: bool) -> Result<(), DaemonError> {
    if may_create {
        let parent = path
            .parent()
            .ok_or_else(|| slice_development_error("missing storage parent"))?;
        if fs::canonicalize(parent).map_err(|e| slice_development_io_error("resolve", parent, e))?
            != parent
        {
            return Err(slice_development_error(
                "empty slice storage parent changed",
            ));
        }
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(slice_development_io_error("create", path, e)),
        }
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|e| slice_development_io_error("inspect", path, e))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || fs::canonicalize(path).map_err(|e| slice_development_io_error("resolve", path, e))?
            != path
    {
        return Err(slice_development_error(
            "empty slice publication must contain canonical real directories",
        ));
    }
    Ok(())
}

pub(super) fn materialize(
    root: &Path,
    expected: Option<&SliceDevelopmentPublication>,
) -> Result<SliceDevelopmentPublication, DaemonError> {
    let actual = validate_binding(root, expected)?;
    for path in [
        root,
        Path::new(&actual.destination_root),
        Path::new(&actual.primary_repository_path),
    ] {
        directory(path, expected.is_none())?;
    }
    if expected.is_none() {
        require_pristine(Path::new(&actual.primary_repository_path))?;
    }
    for path in [
        root,
        Path::new(&actual.destination_root),
        Path::new(&actual.primary_repository_path),
    ] {
        ensure_private_real_directory(path)?;
    }
    update_managed_publication_access("grant", root, &actual)?;
    // Make directory creation durable before the caller records publication in
    // the durable kernel state and permits the worker to start.
    #[cfg(unix)]
    for path in [
        Path::new(&actual.primary_repository_path),
        Path::new(&actual.destination_root),
        root,
        root.parent().expect("validated absolute storage root"),
    ] {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|e| slice_development_io_error("sync", path, e))?;
    }
    Ok(actual)
}

fn require_pristine(workspace: &Path) -> Result<(), DaemonError> {
    if fs::read_dir(workspace)
        .map_err(|e| slice_development_io_error("list", workspace, e))?
        .next()
        .transpose()
        .map_err(|e| slice_development_io_error("list", workspace, e))?
        .is_some()
    {
        return Err(slice_development_error(
            "unpublished empty slice workspace is not pristine",
        ));
    }
    Ok(())
}

pub(super) fn cleanup(
    root: &Path,
    expected: Option<&SliceDevelopmentPublication>,
) -> Result<(), DaemonError> {
    let actual = validate_binding(root, expected)?;
    for path in [
        root,
        Path::new(&actual.destination_root),
        Path::new(&actual.primary_repository_path),
    ] {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(slice_development_io_error("inspect", path, e)),
            Ok(_) => directory(path, false)?,
        }
    }
    let workspace = Path::new(&actual.primary_repository_path);
    if workspace.exists() {
        if expected.is_none() {
            require_pristine(workspace)?;
        }
        update_managed_publication_access("revoke", root, &actual)?;
        if expected.is_some() {
            fs::remove_dir_all(workspace)
                .map_err(|e| slice_development_io_error("remove", workspace, e))?;
        } else {
            // An interrupted first publication has not granted ownership of
            // pre-existing contents. Only remove empty directories in that case.
            fs::remove_dir(workspace)
                .map_err(|e| slice_development_io_error("remove", workspace, e))?;
        }
    }
    for path in [Path::new(&actual.destination_root), root] {
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(slice_development_io_error("remove", path, e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-empty-publication-{}",
                rand::random::<u64>()
            ));
            fs::create_dir(&root).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
        fn storage(&self) -> PathBuf {
            self.0.join("slice-1")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn restart_does_not_recreate_a_missing_durable_workspace() {
        let fixture = Fixture::new();
        let root = fixture.storage();
        let published = materialize(&root, None).unwrap();
        fs::remove_dir(&published.primary_repository_path).unwrap();
        assert!(materialize(&root, Some(&published)).is_err());
        assert!(!Path::new(&published.primary_repository_path).exists());
        cleanup(&root, Some(&published)).unwrap();
        cleanup(&root, Some(&published)).unwrap();
    }

    #[test]
    fn first_publication_does_not_adopt_or_delete_unowned_contents() {
        let fixture = Fixture::new();
        let root = fixture.storage();
        let workspace = root.join("development/workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("existing.txt"), "keep me").unwrap();
        assert!(materialize(&root, None).is_err());
        assert!(cleanup(&root, None).is_err());
        assert_eq!(
            fs::read_to_string(workspace.join("existing.txt")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn mismatched_receipt_is_rejected_before_any_mutation() {
        let fixture = Fixture::new();
        let root = fixture.storage();
        let mut published = publication(&root).unwrap();
        published.primary_repository_path = fixture.0.display().to_string();
        assert!(materialize(&root, Some(&published)).is_err());
        assert!(cleanup(&root, Some(&published)).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn interrupted_pristine_publication_can_be_retried_and_cleaned() {
        let fixture = Fixture::new();
        let root = fixture.storage();
        fs::create_dir(&root).unwrap();
        let published = materialize(&root, None).unwrap();
        assert_eq!(materialize(&root, None).unwrap(), published);
        cleanup(&root, None).unwrap();
        assert!(!root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_replacement_cannot_redirect_recovery_or_cleanup() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let root = fixture.storage();
        let published = materialize(&root, None).unwrap();
        let outside = fixture.0.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep me").unwrap();
        fs::remove_dir(&published.primary_repository_path).unwrap();
        symlink(&outside, &published.primary_repository_path).unwrap();
        assert!(materialize(&root, Some(&published)).is_err());
        assert!(cleanup(&root, Some(&published)).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "keep me"
        );
    }
}
