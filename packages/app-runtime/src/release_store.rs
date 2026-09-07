//! Immutable release staging for macOS/Linux. This does not activate or execute
//! an App. Paths are resolved through owned directory descriptors, never through
//! a check-then-open pathname. The original signed archive remains available for
//! policy/signature reverification after restart.

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "release_store/fs.rs"]
mod fs;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::fs::{check, entry_metadata, publish, remove_contents, same_entry, Dir};
    use chariox_app_package::VerifiedPackage;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    const ARCHIVE: &str = "envelope.cxapp";
    const PAYLOAD: &str = "payload";
    const MARKER: &str = ".chariox-stage";
    const MAGIC: &str = "chariox.app.release-stage.v1\n";
    const MAX_TREE_ENTRIES: usize = 131_072;
    static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

    #[derive(Debug, thiserror::Error)]
    pub enum ReleaseStoreError {
        #[error("app_release_invalid_root")]
        InvalidRoot,
        #[error("app_release_archive_mismatch")]
        ArchiveMismatch,
        #[error("app_release_reservation_exceeded")]
        ReservationExceeded,
        #[error("app_release_host_reserve")]
        HostReserve,
        #[error("app_release_invalid_existing_content")]
        InvalidExisting,
        #[error("app_release_unsafe_entry")]
        UnsafeEntry,
        #[error("app_release_entry_limit")]
        EntryLimit,
        #[error("app_release_io: {0}")]
        Io(#[from] std::io::Error),
    }

    type Result<T> = std::result::Result<T, ReleaseStoreError>;

    /// The caller must hold an exclusive reservation in the kernel's aggregate
    /// admission ledger for this call. These numbers do not create a filesystem
    /// quota or atomically reserve disk space against unrelated processes.
    #[derive(Debug, Clone, Copy)]
    pub struct StageBudget {
        pub max_stage_bytes: u64,
        pub reserved_bytes: u64,
        pub host_reserve_bytes: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum StageCheckpoint {
        Created,
        FileSynced(String),
        BeforePublish,
    }

    #[derive(Debug)]
    pub struct StagedRelease {
        pub package_digest: String,
        pub path: PathBuf,
        pub reused: bool,
        /// Launchers should use this verified, anchored directory handle rather
        /// than treating the display path as continuing verification evidence.
        pub directory: File,
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct CleanupReport {
        pub removed: usize,
        pub active: usize,
        pub unrecognized: usize,
    }

    pub struct ReleaseStore {
        root: Dir,
        path: PathBuf,
    }

    impl ReleaseStore {
        /// Open an existing kernel-owned mode-0700 directory. Every ancestor
        /// must be a real directory: relative paths, `..` and symlinks fail.
        pub fn open(path: &Path) -> Result<Self> {
            if !path.is_absolute() {
                return Err(ReleaseStoreError::InvalidRoot);
            }
            let mut root = Dir::absolute_root()?;
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => root = root.child(name)?,
                    _ => return Err(ReleaseStoreError::InvalidRoot),
                }
            }
            let metadata = root.0.metadata()?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
                return Err(ReleaseStoreError::InvalidRoot);
            }
            Ok(Self {
                root,
                path: path.to_owned(),
            })
        }

        pub fn stage(
            &self,
            package: &VerifiedPackage<'_>,
            archive: &[u8],
            budget: StageBudget,
        ) -> Result<StagedRelease> {
            self.stage_with_checkpoint(package, archive, budget, |_| Ok(()))
        }

        /// Conservative bytes the kernel must reserve before starting this
        /// stage. This includes the signed archive, extracted payload, rounded
        /// allocation units and per-entry metadata headroom.
        pub fn required_reservation(
            &self,
            package: &VerifiedPackage<'_>,
            archive: &[u8],
        ) -> Result<u64> {
            if package.package_digest() != format!("sha256:{:x}", Sha256::digest(archive)) {
                return Err(ReleaseStoreError::ArchiveMismatch);
            }
            let marker = format!("{MAGIC}{}\n", package.package_digest());
            let tree = ExpectedTree::new(package, archive, marker.as_bytes())?;
            let (allocation_unit, _) = self.disk_state()?;
            reservation_bytes(&tree, allocation_unit)
        }

        /// Checkpoints are for kernel progress reporting and deterministic
        /// interruption drills. They execute no App code.
        pub fn stage_with_checkpoint(
            &self,
            package: &VerifiedPackage<'_>,
            archive: &[u8],
            budget: StageBudget,
            mut checkpoint: impl FnMut(StageCheckpoint) -> Result<()>,
        ) -> Result<StagedRelease> {
            let digest = package.package_digest();
            if digest != format!("sha256:{:x}", Sha256::digest(archive)) {
                return Err(ReleaseStoreError::ArchiveMismatch);
            }
            let final_name = digest_name(digest)?;
            let marker = format!("{MAGIC}{digest}\n");
            let tree = ExpectedTree::new(package, archive, marker.as_bytes())?;
            if let Some(existing) = self.open_existing(final_name)? {
                verify_tree(&existing, &tree)?;
                return self.result(final_name, digest, existing, true);
            }
            self.check_budget(&tree, budget)?;
            let mut temporary = Temporary::create(&self.root)?;
            temporary
                .dir
                .write_new(OsStr::new(MARKER), marker.as_bytes())?;
            temporary.dir.0.sync_all()?;
            self.root.0.sync_all()?;
            checkpoint(StageCheckpoint::Created)?;
            for directory in &tree.directories {
                if directory.is_empty() {
                    continue;
                }
                let (parent, name) = parent(&temporary.dir, directory)?;
                parent.create_child(&name)?;
            }
            for (path, bytes) in &tree.files {
                if path == MARKER {
                    continue;
                }
                let (parent, name) = parent(&temporary.dir, path)?;
                parent.write_new(&name, bytes)?;
                checkpoint(StageCheckpoint::FileSynced(path.clone()))?;
            }
            // Sync and remove write permission from descendants before parents.
            for directory in tree.directories.iter().rev() {
                let dir = descend(&temporary.dir, directory)?;
                dir.readonly()?;
                dir.0.sync_all()?;
            }
            checkpoint(StageCheckpoint::BeforePublish)?;
            verify_tree(&temporary.dir, &tree)?;
            if !same_entry(&self.root, &temporary.name, &temporary.dir)? {
                return Err(ReleaseStoreError::UnsafeEntry);
            }
            match publish(&self.root, &temporary.name, OsStr::new(final_name)) {
                Ok(()) => {
                    temporary.published = true;
                    self.root.0.sync_all()?;
                    let directory = self.root.child(OsStr::new(final_name))?;
                    if !same_entry(&self.root, OsStr::new(final_name), &temporary.dir)? {
                        return Err(ReleaseStoreError::UnsafeEntry);
                    }
                    verify_tree(&directory, &tree)?;
                    self.result(final_name, digest, directory, false)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = self.root.child(OsStr::new(final_name))?;
                    verify_tree(&existing, &tree)?;
                    self.result(final_name, digest, existing, true)
                }
                Err(error) => Err(error.into()),
            }
        }

        /// Collect recognizable abandoned stages only. A live stage holds a
        /// nonblocking advisory lock; unknown entries and all releases survive.
        pub fn collect_abandoned(&self) -> Result<CleanupReport> {
            let mut report = CleanupReport::default();
            for name in self.root.entries(MAX_TREE_ENTRIES)? {
                if !stage_name(&name) {
                    continue;
                }
                let dir = match self.root.child(&name) {
                    Ok(dir) => dir,
                    Err(_) => {
                        report.unrecognized += 1;
                        continue;
                    }
                };
                // A just-created stage has not acquired its lock or written its
                // marker yet. Do not compete for that initial lock before the
                // marker proves initialization finished.
                let recognized = (|| -> Result<bool> {
                    let mut marker = dir.read_file(OsStr::new(MARKER), false)?;
                    if marker.metadata()?.len() > 256 {
                        return Ok(false);
                    }
                    let mut text = String::new();
                    (&mut marker).take(257).read_to_string(&mut text)?;
                    if text.len() > 256 {
                        return Ok(false);
                    }
                    let Some(digest) = text.strip_prefix(MAGIC).and_then(|s| s.strip_suffix('\n'))
                    else {
                        return Ok(false);
                    };
                    Ok(digest_name(digest).is_ok())
                })()
                .unwrap_or(false);
                if !recognized {
                    report.unrecognized += 1;
                    continue;
                }
                if !dir.try_lock()? {
                    report.active += 1;
                    continue;
                }
                if !same_entry(&self.root, &name, &dir)? {
                    continue;
                }
                let mut remaining = MAX_TREE_ENTRIES;
                remove_contents(&dir, &mut remaining, 0)?;
                if same_entry(&self.root, &name, &dir)? {
                    self.root.remove_directory(&name)?;
                    report.removed += 1;
                }
            }
            self.root.0.sync_all()?;
            Ok(report)
        }

        fn check_budget(&self, tree: &ExpectedTree<'_>, budget: StageBudget) -> Result<()> {
            let (allocation_unit, available) = self.disk_state()?;
            let required = reservation_bytes(tree, allocation_unit)?;
            if required > budget.max_stage_bytes || required > budget.reserved_bytes {
                return Err(ReleaseStoreError::ReservationExceeded);
            }
            let needed = budget
                .host_reserve_bytes
                .checked_add(required)
                .ok_or(ReleaseStoreError::HostReserve)?;
            if available < needed {
                return Err(ReleaseStoreError::HostReserve);
            }
            Ok(())
        }

        fn disk_state(&self) -> Result<(u64, u64)> {
            let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            check(unsafe { libc::fstatvfs(self.root.0.as_raw_fd(), stats.as_mut_ptr()) })?;
            let stats = unsafe { stats.assume_init() };
            // f_bsize is the preferred I/O size (1 MiB on APFS), not its
            // allocation unit. f_frsize scales the filesystem's block counts.
            let allocation_unit = stats.f_frsize as u64;
            if allocation_unit == 0 {
                return Err(ReleaseStoreError::HostReserve);
            }
            let available = (stats.f_bavail as u64)
                .checked_mul(allocation_unit)
                .ok_or(ReleaseStoreError::HostReserve)?;
            Ok((allocation_unit, available))
        }

        fn open_existing(&self, name: &str) -> Result<Option<Dir>> {
            match self.root.child(OsStr::new(name)) {
                Ok(directory) => Ok(Some(directory)),
                Err(ReleaseStoreError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        }

        fn result(
            &self,
            name: &str,
            digest: &str,
            dir: Dir,
            reused: bool,
        ) -> Result<StagedRelease> {
            Ok(StagedRelease {
                package_digest: digest.to_owned(),
                path: self.path.join(name),
                reused,
                directory: dir.0,
            })
        }
    }

    struct ExpectedTree<'a> {
        files: BTreeMap<String, &'a [u8]>,
        directories: BTreeSet<String>,
    }

    fn reservation_bytes(tree: &ExpectedTree<'_>, block: u64) -> Result<u64> {
        // The reservation includes metadata headroom; exact physical overhead
        // and aggregate enforcement still belong to the provisioned volume.
        let mut required = (tree.directories.len() as u64)
            .checked_mul(block)
            .ok_or(ReleaseStoreError::ReservationExceeded)?;
        for bytes in tree.files.values() {
            let blocks = (bytes.len() as u64)
                .div_ceil(block)
                .checked_add(1)
                .ok_or(ReleaseStoreError::ReservationExceeded)?;
            required = required
                .checked_add(
                    blocks
                        .checked_mul(block)
                        .ok_or(ReleaseStoreError::ReservationExceeded)?,
                )
                .ok_or(ReleaseStoreError::ReservationExceeded)?;
        }
        Ok(required)
    }

    impl<'a> ExpectedTree<'a> {
        fn new(
            package: &'a VerifiedPackage<'_>,
            archive: &'a [u8],
            marker: &'a [u8],
        ) -> Result<Self> {
            let mut files =
                BTreeMap::from([(ARCHIVE.to_owned(), archive), (MARKER.to_owned(), marker)]);
            let mut directories = BTreeSet::from([String::new(), PAYLOAD.to_owned()]);
            for (path, bytes) in package.files() {
                let full = format!("{PAYLOAD}/{path}");
                components(&full)?;
                let mut parts: Vec<_> = full.split('/').collect();
                parts.pop();
                while !parts.is_empty() {
                    directories.insert(parts.join("/"));
                    parts.pop();
                }
                files.insert(full, bytes);
            }
            if files.len() + directories.len() > MAX_TREE_ENTRIES {
                return Err(ReleaseStoreError::EntryLimit);
            }
            Ok(Self { files, directories })
        }
    }

    struct Temporary<'a> {
        root: &'a Dir,
        dir: Dir,
        name: OsString,
        published: bool,
    }
    impl<'a> Temporary<'a> {
        fn create(root: &'a Dir) -> Result<Self> {
            for _ in 0..32 {
                let name = OsString::from(format!(
                    ".stage-{}-{}-{:x}",
                    std::process::id(),
                    NEXT_STAGE.fetch_add(1, Ordering::Relaxed),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ));
                match root.create_child(&name) {
                    Ok(dir) => {
                        let temporary = Self {
                            root,
                            dir,
                            name,
                            published: false,
                        };
                        if !temporary.dir.try_lock()? {
                            return Err(ReleaseStoreError::UnsafeEntry);
                        }
                        return Ok(temporary);
                    }
                    Err(ReleaseStoreError::Io(error))
                        if error.kind() == std::io::ErrorKind::AlreadyExists =>
                    {
                        continue
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(ReleaseStoreError::UnsafeEntry)
        }
    }
    impl Drop for Temporary<'_> {
        fn drop(&mut self) {
            if self.published {
                return;
            }
            // Do not remove a replacement that no longer names this stage.
            if same_entry(self.root, &self.name, &self.dir).unwrap_or(false) {
                let mut remaining = MAX_TREE_ENTRIES;
                if remove_contents(&self.dir, &mut remaining, 0).is_ok()
                    && same_entry(self.root, &self.name, &self.dir).unwrap_or(false)
                {
                    let _ = self.root.remove_directory(&self.name);
                    let _ = self.root.0.sync_all();
                }
            }
        }
    }

    fn verify_tree(root: &Dir, tree: &ExpectedTree<'_>) -> Result<()> {
        let mut actual = BTreeSet::new();
        let mut remaining = MAX_TREE_ENTRIES;
        inspect_tree(root, "", &mut actual, &mut remaining)?;
        let expected: BTreeSet<_> = tree
            .files
            .keys()
            .chain(tree.directories.iter())
            .cloned()
            .collect();
        if actual != expected {
            return Err(ReleaseStoreError::InvalidExisting);
        }
        let mut buffer = [0u8; 64 * 1024];
        for (path, expected) in &tree.files {
            let (directory, name) = parent(root, path)?;
            let mut input = directory.read_file(&name, true)?;
            if input.metadata()?.len() != expected.len() as u64 {
                return Err(ReleaseStoreError::InvalidExisting);
            }
            for bytes in expected.chunks(buffer.len()) {
                input.read_exact(&mut buffer[..bytes.len()])?;
                if bytes != &buffer[..bytes.len()] {
                    return Err(ReleaseStoreError::InvalidExisting);
                }
            }
            if input.read(&mut buffer[..1])? != 0 {
                return Err(ReleaseStoreError::InvalidExisting);
            }
        }
        Ok(())
    }

    fn inspect_tree(
        dir: &Dir,
        prefix: &str,
        actual: &mut BTreeSet<String>,
        remaining: &mut usize,
    ) -> Result<()> {
        if prefix.split('/').count() > 64 {
            return Err(ReleaseStoreError::EntryLimit);
        }
        let metadata = dir.0.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o222 != 0 {
            return Err(ReleaseStoreError::UnsafeEntry);
        }
        actual.insert(prefix.to_owned());
        for name in dir.entries(*remaining)? {
            *remaining = remaining
                .checked_sub(1)
                .ok_or(ReleaseStoreError::EntryLimit)?;
            let text = name.to_str().ok_or(ReleaseStoreError::UnsafeEntry)?;
            let path = if prefix.is_empty() {
                text.to_owned()
            } else {
                format!("{prefix}/{text}")
            };
            let metadata = entry_metadata(dir, &name)?;
            match metadata.st_mode & libc::S_IFMT {
                libc::S_IFDIR => inspect_tree(&dir.child(&name)?, &path, actual, remaining)?,
                libc::S_IFREG => {
                    actual.insert(path);
                }
                _ => return Err(ReleaseStoreError::UnsafeEntry),
            }
        }
        Ok(())
    }

    fn parent(root: &Dir, path: &str) -> Result<(Dir, OsString)> {
        let mut parts = components(path)?;
        let name = parts.pop().ok_or(ReleaseStoreError::UnsafeEntry)?;
        Ok((descend(root, &parts.join("/"))?, OsString::from(name)))
    }
    fn descend(root: &Dir, path: &str) -> Result<Dir> {
        let mut current = root.child(OsStr::new("."))?;
        if !path.is_empty() {
            for part in components(path)? {
                current = current.child(OsStr::new(part))?;
            }
        }
        Ok(current)
    }
    fn components(path: &str) -> Result<Vec<&str>> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.is_empty()
            || parts.len() > 64
            || parts.iter().any(|part| {
                part.is_empty() || *part == "." || *part == ".." || part.contains(['\\', '\0'])
            })
        {
            return Err(ReleaseStoreError::UnsafeEntry);
        }
        Ok(parts)
    }
    fn digest_name(digest: &str) -> Result<&str> {
        let name = digest
            .strip_prefix("sha256:")
            .ok_or(ReleaseStoreError::ArchiveMismatch)?;
        if name.len() != 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ReleaseStoreError::ArchiveMismatch);
        }
        Ok(name)
    }
    fn stage_name(name: &OsStr) -> bool {
        name.to_str().is_some_and(|name| {
            name.starts_with(".stage-")
                && name.len() < 128
                && name[7..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        })
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;
