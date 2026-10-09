//! MP-08: Per-kernel durable manifests and export-time evidence invalidation.
use super::resolver::{environment_error, open_workspace_file};
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentEvidence {
    /// Content identities of lockfiles, env examples, config schemas and reference-bearing code.
    /// No source contents or values are stored in this index.
    pub files: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub private_inventory: BTreeMap<String, BTreeMap<String, u64>>,
}
impl ProjectEnvironmentEvidence {
    pub fn digest(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("evidence encodes"))
        )
    }
    pub fn changed_paths(&self, previous: &Self) -> BTreeMap<String, Vec<String>> {
        let mut result = BTreeMap::new();
        for workspace in self
            .files
            .keys()
            .chain(previous.files.keys())
            .chain(self.private_inventory.keys())
            .chain(previous.private_inventory.keys())
            .collect::<BTreeSet<_>>()
        {
            let current = self.files.get(workspace);
            let old = previous.files.get(workspace);
            let private = self.private_inventory.get(workspace);
            let old_private = previous.private_inventory.get(workspace);
            let paths = current
                .into_iter()
                .flat_map(|files| files.keys())
                .chain(old.into_iter().flat_map(|files| files.keys()))
                .chain(private.into_iter().flat_map(|files| files.keys()))
                .chain(old_private.into_iter().flat_map(|files| files.keys()))
                .collect::<BTreeSet<_>>();
            let changed: Vec<_> = paths
                .into_iter()
                .filter(|path| {
                    current.and_then(|files| files.get(*path))
                        != old.and_then(|files| files.get(*path))
                        || private.and_then(|files| files.get(*path))
                            != old_private.and_then(|files| files.get(*path))
                })
                .cloned()
                .collect();
            if !changed.is_empty() {
                result.insert(workspace.clone(), changed);
            }
        }
        result
    }
    pub fn capture(
        workspaces: &BTreeMap<String, PathBuf>,
        paths: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DaemonError> {
        if paths.len() > 32 {
            return Err(environment_error(
                "environment evidence exceeds workspace bounds",
            ));
        }
        let mut evidence = Self::default();
        let mut total = 0_u64;
        for (workspace, selected) in paths {
            let root = workspaces
                .get(workspace)
                .ok_or_else(|| environment_error("evidence workspace not selected"))?;
            if selected.len() > 20_000 {
                return Err(environment_error(
                    "environment evidence exceeds file bounds",
                ));
            }
            let mut files = BTreeMap::new();
            for path in selected {
                // The evidence inventory must never include value-bearing environment files.
                let filename = Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if filename == ".envrc"
                    || (filename.starts_with(".env")
                        && !filename.ends_with(".example")
                        && !filename.ends_with(".sample")
                        && !filename.ends_with(".template"))
                {
                    return Err(environment_error(
                        "value-bearing environment files are not discovery evidence",
                    ));
                }
                let mut file = open_workspace_file(root, path)?;
                let mut hash = Sha256::new();
                let mut buffer = [0; 8192];
                let mut size = 0;
                loop {
                    let count = file
                        .read(&mut buffer)
                        .map_err(|_| environment_error("environment evidence read failed"))?;
                    if count == 0 {
                        break;
                    }
                    size += count as u64;
                    total += count as u64;
                    if size > 16 * 1024 * 1024 || total > 256 * 1024 * 1024 {
                        return Err(environment_error(
                            "environment evidence exceeds byte bounds",
                        ));
                    }
                    hash.update(&buffer[..count]);
                }
                files.insert(path.clone(), format!("{:x}", hash.finalize()));
            }
            evidence.files.insert(workspace.clone(), files);
        }
        Ok(evidence)
    }
}

// MP-08 / MP-10 / MP-11: Receipt metadata for an explicit later fetch only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentSource {
    pub kernel_id: String,
    pub context: crate::transport::relay_peer::RemoteNativeInteractionContext,
    pub workspaces: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredProjectEnvironment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ProjectEnvironmentSource>,
    pub manifest: ProjectEnvironmentManifest,
    pub evidence: ProjectEnvironmentEvidence,
    /// Names already projected as missing; used to avoid repeated questions on later exports.
    pub reported_missing: BTreeSet<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_manifest: Option<ProjectEnvironmentManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_review: Option<ProjectEnvironmentReview>,
}

#[derive(Debug, Clone)]
pub struct ProjectEnvironmentStore {
    pub(super) root: PathBuf,
}
impl ProjectEnvironmentStore {
    pub fn new(private_state_root: &Path) -> Self {
        Self {
            root: private_state_root.join("project-environments"),
        }
    }
    pub(super) fn path(&self, project: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.json", Sha256::digest(project.as_bytes())))
    }
    pub fn load(&self, project: &str) -> Result<Option<StoredProjectEnvironment>, DaemonError> {
        let path = self.path(project);
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(environment_error(
                    "Project environment manifest unavailable",
                ))
            }
        };
        if fs::symlink_metadata(&path)
            .map_err(|_| environment_error("manifest unavailable"))?
            .file_type()
            .is_symlink()
            || !file
                .metadata()
                .map_err(|_| environment_error("manifest unavailable"))?
                .is_file()
        {
            return Err(environment_error(
                "Project environment manifest must be a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("manifest read failed"))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(environment_error("manifest exceeds bounds"));
        }
        let state: StoredProjectEnvironment = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("invalid stored environment manifest"))?;
        state.manifest.validate().map_err(environment_error)?;
        if state.manifest.project_id != project
            || state.evidence.digest() != state.manifest.evidence_digest
        {
            return Err(environment_error(
                "Project environment evidence binding mismatch",
            ));
        }
        super::restore_project_file_rules(&state.manifest);
        Ok(Some(state))
    }
    pub fn save(&self, state: &StoredProjectEnvironment) -> Result<(), DaemonError> {
        state.manifest.validate().map_err(environment_error)?;
        if state.evidence.digest() != state.manifest.evidence_digest {
            return Err(environment_error(
                "Project environment evidence binding mismatch",
            ));
        }
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("environment manifest directory unavailable"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|_| environment_error("secure environment directory failed"))?;
        }
        let bytes = serde_json::to_vec(state)
            .map_err(|_| environment_error("environment manifest encoding failed"))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(environment_error("manifest exceeds bounds"));
        }
        let temporary = self.root.join(format!(".tmp-{}", rand::random::<u64>()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .map_err(|_| environment_error("environment manifest staging failed"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| environment_error("persist environment manifest failed"))?;
            fs::rename(&temporary, self.path(&state.manifest.project_id))
                .map_err(|_| environment_error("publish environment manifest failed"))?;
            File::open(&self.root)
                .and_then(|file| file.sync_all())
                .map_err(|_| environment_error("sync environment manifest directory failed"))
        })();
        let _ = fs::remove_file(temporary);
        if result.is_ok() {
            super::restore_project_file_rules(&state.manifest);
        }
        result
    }
    pub fn remove(&self, project: &str) -> Result<(), DaemonError> {
        match fs::remove_file(self.path(project).with_extension("revisions.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(environment_error("remove revision history failed")),
        }
        for extension in ["detect.json", "detect-model.json"] {
            match fs::remove_file(self.path(project).with_extension(extension)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(environment_error("remove detection cache failed")),
            }
        }
        match fs::remove_file(self.path(project).with_extension("identity.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(environment_error("remove environment identity failed")),
        }
        match fs::remove_file(self.path(project)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(environment_error(
                "remove Project environment manifest failed",
            )),
        }
    }
}

/// MP-08: Serialize refresh and supplied-input updates on this kernel, including after restart.
/// The runtime acquires this in a blocking task before awaiting official-provider discovery.
pub struct ProjectEnvironmentLock {
    _file: File,
}
impl Drop for ProjectEnvironmentLock {
    fn drop(&mut self) {
        // A concurrent fork can retain this open file description until exec.
        // Closing only our descriptor must not prolong the kernel's ownership.
        let _ = fs2::FileExt::unlock(&self._file);
    }
}
impl std::fmt::Debug for ProjectEnvironmentLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProjectEnvironmentLock")
    }
}
impl ProjectEnvironmentStore {
    pub fn lock(&self, project: &str) -> Result<ProjectEnvironmentLock, DaemonError> {
        self.acquire_lock(project, false, "lock")
    }
    pub(crate) fn try_lock(&self, project: &str) -> Result<ProjectEnvironmentLock, DaemonError> {
        self.acquire_lock(project, true, "lock")
    }
    pub(super) fn identity_lock(
        &self,
        project: &str,
    ) -> Result<ProjectEnvironmentLock, DaemonError> {
        self.acquire_lock(project, false, "identity.lock")
    }
    /// Saves and mutation snapshots queue behind each other for a bounded time,
    /// while a longer Detect, export or adjustment holder reports busy.
    pub(crate) fn lock_briefly(
        &self,
        project: &str,
    ) -> Result<ProjectEnvironmentLock, DaemonError> {
        let file = self.lock_file(project, "lock")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(ProjectEnvironmentLock { _file: file }),
                Err(error) if error.kind() != std::io::ErrorKind::WouldBlock => {
                    return Err(environment_error("environment refresh lock failed"))
                }
                Err(_) if std::time::Instant::now() >= deadline => return Err(environment_error(
                    "Project environment is busy with Detect, export or adjustment; retry shortly",
                )),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
    }
    pub(crate) async fn lock_briefly_async(
        &self,
        project: &str,
    ) -> Result<ProjectEnvironmentLock, DaemonError> {
        let (store, project) = (self.clone(), project.to_owned());
        tokio::task::spawn_blocking(move || store.lock_briefly(&project))
            .await
            .map_err(|_| environment_error("environment lock task failed"))?
    }
    fn acquire_lock(
        &self,
        project: &str,
        nonblocking: bool,
        extension: &str,
    ) -> Result<ProjectEnvironmentLock, DaemonError> {
        let file = self.lock_file(project, extension)?;
        if nonblocking {
            fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
                environment_error("Project environment already has an active export or adjustment")
            })?;
        } else {
            fs2::FileExt::lock_exclusive(&file)
                .map_err(|_| environment_error("environment refresh lock failed"))?;
        }
        Ok(ProjectEnvironmentLock { _file: file })
    }
    fn lock_file(&self, project: &str, extension: &str) -> Result<File, DaemonError> {
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("environment manifest directory unavailable"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|_| environment_error("secure environment directory failed"))?;
        }
        let path = self.path(project).with_extension(extension);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options
            .open(path)
            .map_err(|_| environment_error("environment refresh lock unavailable"))?;
        if !file
            .metadata()
            .map_err(|_| environment_error("environment refresh lock unavailable"))?
            .is_file()
        {
            return Err(environment_error(
                "environment refresh lock must be a regular file",
            ));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn dropping_adjustment_releases_lock_with_an_inherited_descriptor() {
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "chariox-inherited-project-lock-{:032x}",
            rand::random::<u128>()
        )));
        let store = ProjectEnvironmentStore::new(&scratch.0);
        let first = store.try_lock("project").unwrap();
        // dup, like fork before exec, retains the same open file description.
        // O_CLOEXEC cannot release that reference until the child execs.
        let inherited = first._file.try_clone().unwrap();
        assert!(store.try_lock("project").is_err());
        drop(first);
        let next = store.try_lock("project").unwrap();
        assert!(store.try_lock("project").is_err());
        drop(inherited);
        // Closing the old inherited descriptor cannot release the new owner.
        assert!(store.try_lock("project").is_err());
        drop(next);
        assert!(store.try_lock("project").is_ok());
    }
}
