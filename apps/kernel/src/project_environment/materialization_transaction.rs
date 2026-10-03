//! MP-08 / MP-10 / MP-11: Import rollback respects code-layer and target ownership.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy)]
pub(super) enum MaterializationTarget {
    /// M28 owns the entire disposable checkout and its publication rollback.
    StagedDevelopment,
    /// A lease may privatize identical configuration; differing contents stay untouched.
    MountedSource,
}

pub(super) struct ProjectEnvironmentMaterialization {
    files: Vec<MaterializedFile>,
}
impl ProjectEnvironmentMaterialization {
    pub(super) fn prepare(
        manifest: &ProjectEnvironmentManifest,
        resolved: &ResolvedProjectEnvironment,
        roots: &BTreeMap<String, PathBuf>,
        target: MaterializationTarget,
    ) -> Result<Self, DaemonError> {
        let mut transaction = Self { files: Vec::new() };
        for ((workspace, path), value) in
            super::materialize::project_environment_files(manifest, resolved)?
        {
            let root = roots
                .get(&workspace)
                .ok_or_else(|| environment_error("materialization workspace not selected"))?;
            use super::materialize::WorkspaceFilePublication;
            let publication = match target {
                MaterializationTarget::StagedDevelopment => WorkspaceFilePublication::Replace,
                MaterializationTarget::MountedSource
                    if manifest.entries.iter().any(|entry| {
                        entry.workspace_id == workspace
                            && entry.name == path
                            && entry.kind == ProjectEnvironmentEntryKind::ConfigFile
                    }) =>
                {
                    WorkspaceFilePublication::ReuseMatching
                }
                MaterializationTarget::MountedSource => WorkspaceFilePublication::Create,
            };
            transaction
                .files
                .push(super::materialize::write_private_workspace_file_owned(
                    root,
                    &path,
                    value.as_bytes(),
                    publication,
                )?);
        }
        Ok(transaction)
    }
    pub(super) fn commit(self) {
        for file in self.files {
            file.commit();
        }
    }
}

pub(super) struct MaterializedFile {
    #[cfg(unix)]
    parent: std::fs::File,
    #[cfg(unix)]
    name: std::ffi::CString,
    #[cfg(unix)]
    identity: std::fs::Metadata,
    #[cfg(unix)]
    original_permissions: Option<(std::fs::File, std::fs::Permissions)>,
    #[cfg(target_os = "linux")]
    replacement_backup: Option<Box<MaterializedFile>>,
    committed: bool,
}
impl MaterializedFile {
    #[cfg(unix)]
    pub(super) fn new(
        parent: std::fs::File,
        name: std::ffi::CString,
        file: &std::fs::File,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            parent,
            name,
            identity: file
                .metadata()
                .map_err(|_| environment_error("materialized file identity unavailable"))?,
            committed: false,
            original_permissions: None,
            #[cfg(target_os = "linux")]
            replacement_backup: None,
        })
    }
    #[cfg(target_os = "linux")]
    pub(super) fn refresh_identity(&mut self, file: &std::fs::File) -> Result<(), DaemonError> {
        self.identity = file
            .metadata()
            .map_err(|_| environment_error("materialized file identity unavailable"))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    pub(super) fn exchange_matching(
        &mut self,
        name: &std::ffi::CString,
        original: &std::fs::Metadata,
    ) -> Result<(), DaemonError> {
        use std::os::fd::AsRawFd;
        let backup_parent = self
            .parent
            .try_clone()
            .map_err(|_| environment_error("target config directory unavailable"))?;
        if unsafe {
            libc::renameat2(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                self.parent.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        } != 0
        {
            return Err(environment_error(
                "publish private mounted configuration failed",
            ));
        }
        let backup_name = std::mem::replace(&mut self.name, name.clone());
        self.replacement_backup = Some(Box::new(Self {
            parent: backup_parent,
            name: backup_name,
            identity: original.clone(),
            original_permissions: None,
            replacement_backup: None,
            committed: false,
        }));
        // The path could change while its selected contents were being read.
        // Recheck the exchanged inode; failure Drop restores that actual file.
        let backup = self.replacement_backup.as_mut().unwrap();
        let current = metadata_at(&backup.parent, &backup.name).ok_or_else(|| {
            environment_error("mounted configuration changed during reconciliation")
        })?;
        let matching = same_file(&current, original);
        backup.identity = current;
        if !matching {
            return Err(environment_error(
                "mounted configuration changed during reconciliation",
            ));
        }
        Ok(())
    }
    #[cfg(unix)]
    pub(super) fn preserve_permissions(&mut self, file: std::fs::File) {
        self.original_permissions = Some((file, self.identity.permissions()));
    }
    #[cfg(unix)]
    pub(super) fn sync(&self) -> Result<(), DaemonError> {
        self.parent
            .sync_all()
            .map_err(|_| environment_error("sync target config directory failed"))
    }
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for MaterializedFile {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::fs::MetadataExt,
            };
            let descriptor = unsafe {
                libc::openat(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                return;
            }
            let current = unsafe { std::fs::File::from_raw_fd(descriptor) };
            let Ok(identity) = current.metadata() else {
                return;
            };
            // Do not erase a replacement file or a file edited after publication.
            if identity.dev() == self.identity.dev()
                && identity.ino() == self.identity.ino()
                && identity.len() == self.identity.len()
                && identity.mtime() == self.identity.mtime()
                && identity.mtime_nsec() == self.identity.mtime_nsec()
            {
                #[cfg(target_os = "linux")]
                if let Some(backup) = &mut self.replacement_backup {
                    if metadata_at(&backup.parent, &backup.name)
                        .is_some_and(|current| same_file(&current, &backup.identity))
                        && unsafe {
                            libc::renameat2(
                                self.parent.as_raw_fd(),
                                self.name.as_ptr(),
                                backup.parent.as_raw_fd(),
                                backup.name.as_ptr(),
                                libc::RENAME_EXCHANGE,
                            )
                        } == 0
                    {
                        backup.identity = self.identity.clone();
                    } else {
                        // A failed restore must retain the original for recovery.
                        backup.committed = true;
                    }
                    let _ = self.parent.sync_all();
                    return;
                }
                if let Some((file, permissions)) = &self.original_permissions {
                    use std::os::unix::fs::PermissionsExt;
                    // A subsequent target permission change belongs to the target.
                    if identity.permissions().mode() & 0o7777 == 0o600 {
                        let _ = file.set_permissions(permissions.clone());
                        let _ = file.sync_all();
                    }
                } else {
                    unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) };
                }
                let _ = self.parent.sync_all();
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}
#[cfg(target_os = "linux")]
fn metadata_at(parent: &std::fs::File, name: &std::ffi::CString) -> Option<std::fs::Metadata> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    (fd >= 0)
        .then(|| unsafe { std::fs::File::from_raw_fd(fd) }.metadata().ok())
        .flatten()
}
