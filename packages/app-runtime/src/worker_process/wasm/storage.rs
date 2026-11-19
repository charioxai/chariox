//! Supervisor-owned installation storage. The data lock is shared by native
//! preparation and deletion; a live worker's directory can never be erased.
use super::*;

pub fn root(database: &Path) -> Result<PathBuf, WorkerError> {
    let release =
        crate::release_store::root_for_database(database).map_err(|_| WorkerError::Preparation)?;
    let name = release
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(WorkerError::Preparation)?;
    Ok(release.with_file_name(name.replace("app-releases-", "app-wasm-state-")))
}
/// The directory name follows the installation alone: its id is unique in the
/// kernel, and enrollment moves an installation to another owner without
/// moving its saved files.
pub fn installation(installation: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(installation).expect("string serializes"))
    )
}

/// Called under the kernel's installation lifecycle gate after native reap.
/// Missing storage is already deleted, including after a partial prior erase.
pub fn delete(database: &Path, installation_id: &str) -> Result<(), WorkerError> {
    let path = root(database)?;
    let Some(root) = Dir::open_private_child_if_present(
        path.parent().ok_or(WorkerError::Preparation)?,
        path.file_name().ok_or(WorkerError::Preparation)?,
    )
    .map_err(|_| WorkerError::Preparation)?
    else {
        return Ok(());
    };
    let name = installation(installation_id);
    let dir = match root.child(OsStr::new(&name)) {
        Ok(dir) => dir,
        Err(crate::private_fs::FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(())
        }
        Err(_) => return Err(WorkerError::Preparation),
    };
    let data = match dir.child(OsStr::new("data")) {
        Ok(data) => Some(data),
        Err(crate::private_fs::FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(WorkerError::Preparation),
    };
    if let Some(data) = &data {
        if !data.try_lock().map_err(|_| WorkerError::Preparation)? {
            return Err(WorkerError::Storage("app_storage_busy"));
        }
    }
    snapshot::clear(&dir).map_err(|_| WorkerError::Storage("app_storage_io"))?;
    root.remove_directory(OsStr::new(&name))
        .map_err(|_| WorkerError::Preparation)?;
    root.sync().map_err(|_| WorkerError::Preparation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_directory_is_the_installations_not_its_owners() {
        // Enrollment hands an installation to another owner: its saved files
        // must stay where the next start opens them.
        assert_ne!(installation("app"), installation("other"));
    }
    #[test]
    fn delete_erases_data_tmp_and_metadata_only_after_the_data_lock_releases() {
        let scratch = std::env::temp_dir().join(format!(
            "wasm-delete-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&scratch).unwrap();
        let database = scratch.join("kernel.db");
        let path = root(&database).unwrap();
        let root = Dir::open_or_create_private_child(&scratch, path.file_name().unwrap()).unwrap();
        let name = installation("app");
        let dir = root.create_child(OsStr::new(&name)).unwrap();
        let data = dir.create_child(OsStr::new("data")).unwrap();
        let tmp = dir.create_child(OsStr::new("tmp")).unwrap();
        data.write_new(OsStr::new("kept"), b"private").unwrap();
        tmp.write_new(OsStr::new("scratch"), b"temporary").unwrap();
        assert!(data.try_lock().unwrap());
        assert!(matches!(
            delete(&database, "app"),
            Err(WorkerError::Storage("app_storage_busy"))
        ));
        assert!(path.join(&name).join("data/kept").exists());
        drop(data);
        delete(&database, "app").unwrap();
        assert!(!path.join(&name).exists());
        delete(&database, "app").unwrap();
        let new = root.create_child(OsStr::new(&name)).unwrap();
        assert!(new.entries(10).unwrap().is_empty());
        drop((new, tmp, dir, root));
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
