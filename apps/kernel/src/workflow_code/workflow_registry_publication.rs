//! MP-11 R1: own one staging directory and reserve one registry name per writer.
use super::*;
use fs2::FileExt;

pub(super) struct WorkflowRegistryPublication {
    // Keep the inode: unlinking a lock file would let another writer bypass it.
    _name_lock: fs::File,
    staging_dir: Option<PathBuf>,
    entry_dir: PathBuf,
}

impl WorkflowRegistryPublication {
    pub(super) fn begin(root: &Path, name: &str) -> Result<Self, crate::DaemonError> {
        fs::create_dir_all(root).map_err(io_error("workflow_registry.add"))?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(format!(".{name}.lock")))
            .map_err(io_error("workflow_registry.add"))?;
        lock.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                conflict(name)
            } else {
                io_error("workflow_registry.add")(error)
            }
        })?;
        let entry_dir = root.join(name);
        if entry_dir.exists() || root.join(format!("{name}.js")).exists() {
            return Err(conflict(name));
        }
        for _ in 0..8 {
            let staging_dir = root.join(format!(".{name}.tmp-{:032x}", rand::random::<u128>()));
            match fs::create_dir(&staging_dir) {
                Ok(()) => {
                    return Ok(Self {
                        _name_lock: lock,
                        staging_dir: Some(staging_dir),
                        entry_dir,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("workflow_registry.add")(error)),
            }
        }
        Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.add",
            message: "could not allocate a unique workflow registry staging directory".into(),
        })
    }

    pub(super) fn staging_dir(&self) -> &Path {
        self.staging_dir
            .as_deref()
            .expect("unpublished registry entry")
    }

    pub(super) fn publish(mut self) -> Result<(), crate::DaemonError> {
        fs::rename(self.staging_dir(), &self.entry_dir)
            .map_err(io_error("workflow_registry.add"))?;
        self.staging_dir = None;
        Ok(())
    }
}

impl Drop for WorkflowRegistryPublication {
    fn drop(&mut self) {
        if let Some(path) = self.staging_dir.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn conflict(name: &str) -> crate::DaemonError {
    crate::DaemonError::LocalTransport {
        operation: "workflow_registry.add",
        message: format!(
            "workflow registry entry `{name}` name conflict: already exists or is being added"
        ),
    }
}
