//! Durable image/mount state. The helper owns this store and its exclusive root
//! lock until every client lease has been released or journaled for recovery.
mod code;
use super::{
    cgroup::{self, Bound},
    files, formatter,
    loop_device::Device,
    model::{
        self, Enrollment, Identity, Image, Journal, Owner, Request, Role, Snapshot, SNAPSHOT_IMAGE,
    },
    mount, Error, Result, DATA_BYTES, HOST_RESERVE_BYTES, MAX_INSTALLATIONS, MAX_RESERVED_BYTES,
    ROOT, TMP_BYTES,
};
use crate::private_fs::Dir;
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsStr,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

pub(super) struct Store {
    root: Dir,
    owners: BTreeMap<u32, Owner>,
    live: BTreeMap<String, Live>,
    recovering: VecDeque<String>,
}
struct Live {
    directory: Dir,
    path: PathBuf,
    journal: Journal,
    bound: Bound,
    code_sources: Option<super::code_sources::Sources>,
}
#[derive(Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    pub lease: String,
    pub data: String,
    pub temporary: String,
    pub data_mount_id: u64,
    pub temporary_mount_id: u64,
    pub data_root: model::Identity,
    pub temporary_root: model::Identity,
    pub data_bytes: u64,
    pub temporary_bytes: u64,
}
impl Store {
    pub fn open(enrollment: Enrollment) -> Result<Self> {
        enrollment.validate()?;
        let root = files::root_directory(Path::new(ROOT))?;
        if root.0.metadata()?.mode() & 0o7777 != 0o711 || !root.try_lock()? {
            return Err(Error::Busy);
        }
        root.sync()?;
        let mut store = Self {
            root,
            owners: enrollment
                .owners
                .into_iter()
                .map(|owner| (owner.uid, owner))
                .collect(),
            live: BTreeMap::new(),
            recovering: VecDeque::new(),
        };
        store.recover_all()?;
        Ok(store)
    }
    pub fn enrolled(&self, uid: u32) -> bool {
        self.owners.contains_key(&uid)
    }
    pub fn acquire(&mut self, uid: u32, request: Request) -> Result<Grant> {
        request.validate()?;
        let Request::Acquire {
            owner,
            installation,
            generation,
            cgroup_leaf,
            committed_generation,
        } = request
        else {
            return Err(Error::Invalid);
        };
        let principal = self.owners.get(&uid).ok_or(Error::Identity)?;
        let name = model::installation_name(&owner, &installation)?;
        if self.live.values().any(|live| {
            live.journal.uid == uid
                && live.journal.owner == owner
                && live.journal.installation == installation
        }) {
            return Err(Error::Busy);
        }
        let bound = Bound::acquire(principal, &cgroup_leaf)?;
        let user_directory = files::child(&self.root, &format!("u-{uid}"), 0o711)?;
        let existing = user_directory
            .entries(MAX_INSTALLATIONS)?
            .iter()
            .any(|entry| entry == OsStr::new(&name));
        self.capacity(!existing)?;
        let directory = files::child(&user_directory, &name, 0o711)?;
        let path = Path::new(ROOT).join(format!("u-{uid}")).join(&name);
        directory.sync()?;
        let mut journal = if let Some(mut journal) = files::read_journal(&directory)? {
            journal.validate(uid, &name)?;
            self.recover(&directory, &path, &mut journal)?;
            journal
        } else {
            // A crash before first journal publication can leave only empty
            // mount directories. Never adopt a pre-existing unjournaled image.
            for entry in directory.entries(4)? {
                if entry != OsStr::new("data") && entry != OsStr::new("tmp") {
                    return Err(Error::Identity);
                }
                if !directory.child(&entry)?.entries(1)?.is_empty() {
                    return Err(Error::Identity);
                }
            }
            let mut images = Vec::new();
            for role in [Role::Data, Role::Tmp] {
                let mount_root = files::child(&directory, role.directory(), 0o700)?;
                images.push(Image {
                    role,
                    inode: None,
                    uuid: random_uuid()?,
                    capacity: role.capacity(),
                    formatted: false,
                    retiring: false,
                    mount_id: None,
                    loop_minor: None,
                    mount_root: files::identity(&mount_root.0)?,
                });
            }
            Journal {
                schema: "chariox.app-storage-linux.v1".into(),
                uid,
                gid: principal.gid,
                owner: owner.clone(),
                installation: installation.clone(),
                generation,
                lease: random_hex()?,
                cgroup_leaf: cgroup_leaf.clone(),
                cgroup_identity: bound.identity.clone(),
                cgroup_root_identity: bound.root_identity.clone(),
                boot_id: cgroup::boot_id()?,
                pending_recovery: true,
                images: images.try_into().map_err(|_| Error::Identity)?,
                code: None,
                snapshot: None,
            }
        };
        if journal.gid != principal.gid {
            return Err(Error::Identity);
        }
        if let Some(committed) = committed_generation {
            self.snapshot(&directory, &mut journal, generation, committed)?;
        }
        journal.generation = generation;
        journal.lease = random_hex()?;
        journal.cgroup_leaf = cgroup_leaf;
        journal.cgroup_identity = bound.identity.clone();
        journal.cgroup_root_identity = bound.root_identity.clone();
        journal.boot_id = cgroup::boot_id()?;
        journal.pending_recovery = true;
        files::save_journal(&directory, &journal)?;
        let prepared = (|| {
            for index in 0..2 {
                self.prepare_image(&directory, &mut journal, index, principal)?;
            }
            bound.require_empty()?;
            Ok(Grant {
                lease: journal.lease.clone(),
                data: path.join("data").to_string_lossy().into_owned(),
                temporary: path.join("tmp").to_string_lossy().into_owned(),
                data_mount_id: journal.images[0].mount_id.ok_or(Error::Identity)?,
                temporary_mount_id: journal.images[1].mount_id.ok_or(Error::Identity)?,
                data_root: files::identity(&directory.child(OsStr::new("data"))?.0)?,
                temporary_root: files::identity(&directory.child(OsStr::new("tmp"))?.0)?,
                data_bytes: DATA_BYTES,
                temporary_bytes: TMP_BYTES,
            })
        })();
        // Failed preparation retains the exact cgroup, namespace parents and
        // capacity reservation in this helper until its recovery queue finishes.
        if prepared.is_err() {
            self.recovering.push_back(journal.lease.clone());
        }
        self.live.insert(
            journal.lease.clone(),
            Live {
                directory,
                path,
                journal,
                bound,
                code_sources: None,
            },
        );
        prepared
    }
    /// Runs on detached images before `generation` starts, on the committed
    /// generation's data snapshot (see `snapshot_step`).
    fn snapshot(
        &self,
        directory: &Dir,
        journal: &mut Journal,
        generation: u64,
        committed: u64,
    ) -> Result<()> {
        let kept = journal
            .snapshot
            .clone()
            .and_then(|kept| kept.inode.filter(|_| kept.generation == committed));
        match snapshot_step(journal.generation, generation, committed, kept.is_some()) {
            SnapshotStep::Keep => Ok(()),
            SnapshotStep::Take => self.take_snapshot(directory, journal, committed),
            SnapshotStep::Restore => restore_snapshot(directory, journal, kept),
            SnapshotStep::RestoreAndTake => {
                // The data is the committed generation's again before the
                // new copy, so a crash between the two takes it afresh.
                restore_snapshot(directory, journal, kept)?;
                journal.generation = committed;
                files::save_journal(directory, journal)?;
                self.take_snapshot(directory, journal, committed)
            }
            SnapshotStep::Discard => discard_snapshot(directory, journal),
        }
    }
    fn take_snapshot(&self, directory: &Dir, journal: &mut Journal, committed: u64) -> Result<()> {
        let Some(data) = journal.images[0]
            .inode
            .clone()
            .filter(|_| journal.images[0].formatted)
        else {
            return Ok(());
        };
        discard_snapshot(directory, journal)?;
        self.capacity_with(false, DATA_BYTES)?;
        journal.snapshot = Some(Snapshot {
            generation: committed,
            inode: None,
        });
        files::save_journal(directory, journal)?;
        let source = files::open_image(directory, Role::Data.image())?.ok_or(Error::Identity)?;
        files::require(directory, Role::Data.image(), &source, &data)?;
        let copy = files::create_image(directory, SNAPSHOT_IMAGE, DATA_BYTES)?;
        files::copy_image(&source, &copy, DATA_BYTES)?;
        directory.sync()?;
        journal.snapshot = Some(Snapshot {
            generation: committed,
            inode: Some(files::identity(&copy)?),
        });
        files::save_journal(directory, journal)
    }
    pub fn retry_pending(&mut self) {
        if let Some(lease) = self.recovering.pop_front() {
            if let Some(uid) = self.live.get(&lease).map(|live| live.journal.uid) {
                if self.release(uid, &lease, true).is_err() {
                    self.recovering.push_back(lease);
                }
            }
        }
    }
    fn prepare_image(
        &self,
        directory: &Dir,
        journal: &mut Journal,
        index: usize,
        owner: &Owner,
    ) -> Result<()> {
        let image = &journal.images[index];
        let file = if let Some(expected) = &image.inode {
            let file = files::open_image(directory, image.role.image())?.ok_or(Error::Identity)?;
            files::require(directory, image.role.image(), &file, expected)?;
            if file.metadata()?.len() != image.capacity {
                return Err(Error::Identity);
            }
            file
        } else {
            self.capacity(false)?;
            let file = files::create_image(directory, image.role.image(), image.capacity)?;
            journal.images[index].inode = Some(files::identity(&file)?);
            files::save_journal(directory, journal)?;
            file
        };
        if !journal.images[index].formatted {
            let image = &journal.images[index];
            formatter::format(
                &file,
                &self.root.0,
                image.capacity,
                &image.uuid,
                owner.uid,
                owner.gid,
            )?;
            mount::superblock(&file, image)?;
            journal.images[index].formatted = true;
            files::save_journal(directory, journal)?;
        }
        let image = &journal.images[index];
        mount::superblock(&file, image)?;
        // LOOP_CONFIGURE retains the supplied open file description. Release
        // its short formatter/inspection flock before that description becomes
        // the mounted loop's backing; the helper root lock owns this lifetime.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } != 0 {
            return Err(Error::Io);
        }
        let device = Device::attach(&file, image.capacity)?;
        journal.images[index].loop_minor = Some(device.minor);
        files::save_journal(directory, journal)?;
        let (mounted, mount_id) =
            mount::mount(directory, &journal.images[index], &file, &device, owner)?;
        drop(mounted);
        journal.images[index].mount_id = Some(mount_id);
        files::save_journal(directory, journal)?;
        // The mount owns the configured loop reference. Helper recovery can
        // reopen it by image inode; neither a borrowed loop number nor a path is authority.
        Ok(())
    }
    pub fn release(&mut self, uid: u32, lease: &str, disconnected: bool) -> Result<()> {
        model::hex(lease, 32)?;
        let live = self.live.get_mut(lease).ok_or(Error::Identity)?;
        if live.journal.uid != uid {
            return Err(Error::Identity);
        }
        if disconnected {
            live.bound.quiesce()?;
        } else {
            live.bound.require_empty()?;
        }
        let owner = self.owners.get(&uid).ok_or(Error::Identity)?;
        cleanup(&live.directory, &live.path, &mut live.journal, owner)?;
        self.live.remove(lease);
        Ok(())
    }
    pub fn shutdown(&mut self) {
        // Synchronous best effort, with durable recovery retained on any failure.
        // The daemon owns no detached cleanup thread or arbitrary PID signal.
        let leases = self
            .live
            .iter()
            .map(|(id, live)| (id.clone(), live.journal.uid))
            .collect::<Vec<_>>();
        for (lease, uid) in leases {
            let _ = self.release(uid, &lease, true);
        }
    }
    fn recover(&self, directory: &Dir, path: &Path, journal: &mut Journal) -> Result<()> {
        let owner = self.owners.get(&journal.uid).ok_or(Error::Identity)?;
        if journal.gid != owner.gid {
            return Err(Error::Identity);
        }
        if journal.pending_recovery {
            if let Some(bound) = Bound::recover(owner, journal)? {
                bound.quiesce()?;
            }
        }
        cleanup(directory, path, journal, owner)
    }
    fn recover_all(&mut self) -> Result<()> {
        for user in self.root.entries(16)? {
            let uid = user
                .to_str()
                .and_then(|v| v.strip_prefix("u-"))
                .and_then(|v| v.parse::<u32>().ok())
                .ok_or(Error::Identity)?;
            if user != OsStr::new(&format!("u-{uid}")) || !self.owners.contains_key(&uid) {
                return Err(Error::Identity);
            }
            let directory = self.root.child(&user)?;
            files::root_owned(&directory.0, true)?;
            for name in directory.entries(MAX_INSTALLATIONS)? {
                let child = directory.child(&name)?;
                files::root_owned(&child.0, true)?;
                child.sync()?; // complete any prior rename before observing it
                if let Some(mut journal) = files::read_journal(&child)? {
                    journal.validate(uid, name.to_str().ok_or(Error::Identity)?)?;
                    self.recover(
                        &child,
                        &Path::new(ROOT).join(&user).join(&name),
                        &mut journal,
                    )?;
                } else if !child
                    .entries(4)?
                    .iter()
                    .all(|entry| entry == "data" || entry == "tmp")
                {
                    return Err(Error::Identity);
                }
            }
        }
        self.capacity(false)
    }
    fn capacity(&self, new: bool) -> Result<()> {
        self.capacity_with(new, 0)
    }
    /// `extra` is a data snapshot about to be created.
    fn capacity_with(&self, new: bool, extra: u64) -> Result<()> {
        let mut count = usize::from(new);
        let mut snapshots = extra;
        let mut allocated = 0u64;
        for user in self.root.entries(16)? {
            let user = self.root.child(&user)?;
            for name in user.entries(MAX_INSTALLATIONS)? {
                count += 1;
                let directory = user.child(&name)?;
                files::root_owned(&directory.0, true)?;
                for (image, capacity) in [
                    (Role::Data.image(), DATA_BYTES),
                    (Role::Tmp.image(), TMP_BYTES),
                    (SNAPSHOT_IMAGE, DATA_BYTES),
                ] {
                    if let Some(file) = files::open_image(&directory, image)? {
                        let metadata = file.metadata()?;
                        if metadata.len() > capacity {
                            return Err(Error::Identity);
                        }
                        if image == SNAPSHOT_IMAGE {
                            snapshots = snapshots.checked_add(DATA_BYTES).ok_or(Error::Capacity)?;
                        }
                        allocated = allocated
                            .checked_add(
                                metadata
                                    .blocks()
                                    .saturating_mul(512)
                                    .min(metadata.len())
                                    .min(capacity),
                            )
                            .ok_or(Error::Capacity)?;
                    }
                }
            }
        }
        let promised = (count as u64)
            .checked_mul(DATA_BYTES + TMP_BYTES)
            .and_then(|images| images.checked_add(snapshots))
            .ok_or(Error::Capacity)?;
        if count > MAX_INSTALLATIONS || promised > MAX_RESERVED_BYTES {
            return Err(Error::Capacity);
        }
        let mut fs = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        if unsafe { libc::fstatvfs(self.root.0.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
            return Err(Error::Io);
        }
        let fs = unsafe { fs.assume_init() };
        reserve(fs.f_bavail.saturating_mul(fs.f_frsize), promised, allocated)
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn cleanup(directory: &Dir, path: &Path, journal: &mut Journal, owner: &Owner) -> Result<()> {
    code::cleanup(directory, path, journal)?;
    settle_snapshot(directory, journal)?;
    for index in 0..2 {
        let image = &journal.images[index];
        let file = files::open_image(directory, image.role.image())?;
        if let Some(file) = &file {
            let identity = files::identity(file)?;
            if let Some(expected) = &image.inode {
                files::require(directory, image.role.image(), file, expected)?;
            } else if image.formatted || file.metadata()?.len() > image.capacity {
                return Err(Error::Identity);
            }
            if let Some(device) = Device::find(&identity, image.capacity)? {
                if !image.formatted {
                    return Err(Error::Identity);
                }
                mount::superblock(file, image)?;
                mount::unmount(directory, path, image, &device, owner)?;
                device.detach()?;
            } else {
                let underlying = directory.child(OsStr::new(image.role.directory()))?;
                files::require(
                    directory,
                    image.role.directory(),
                    &underlying.0,
                    &image.mount_root,
                )?;
            }
        } else {
            if image.inode.is_some() && !image.retiring {
                return Err(Error::Identity);
            }
            let underlying = directory.child(OsStr::new(image.role.directory()))?;
            files::require(
                directory,
                image.role.directory(),
                &underlying.0,
                &image.mount_root,
            )?;
        }
        // Only temporary or incomplete images are discarded. Data survives every
        // clean release; deletions remain described by the journal until fsynced.
        if image.role == Role::Tmp || !image.formatted {
            let image_name = image.role.image();
            journal.images[index].retiring = true;
            files::save_journal(directory, journal)?;
            if let Some(file) = file {
                files::require(directory, image_name, &file, &files::identity(&file)?)?;
                directory.remove_file(OsStr::new(image_name))?;
                directory.sync()?;
            }
            journal.images[index].inode = None;
            journal.images[index].formatted = false;
            journal.images[index].retiring = false;
            journal.images[index].uuid = random_uuid()?;
        }
        journal.images[index].mount_id = None;
        journal.images[index].loop_minor = None;
        files::save_journal(directory, journal)?;
    }
    journal.pending_recovery = false;
    files::save_journal(directory, journal)
}
/// Finishes whatever an interrupted snapshot step left: an incomplete copy is
/// dropped, a restore whose rename landed is recorded, and a copy that the
/// journal no longer names is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotStep {
    Take,
    Keep,
    Restore,
    RestoreAndTake,
    Discard,
}

/// What happens to the committed generation's data snapshot before
/// `generation` starts, after `previous` ran; `kept` is a complete copy of it.
/// A staged (uncommitted) generation starts on committed data: it takes the
/// copy when the committed generation ran last, keeps it when retrying itself,
/// and restores it (then copies again) after another uncommitted generation,
/// so no failed update's writes survive. The committed generation starting
/// after an uncommitted one restores it; any other committed start drops it.
fn snapshot_step(previous: u64, generation: u64, committed: u64, kept: bool) -> SnapshotStep {
    if generation == committed {
        return if previous > committed && kept {
            SnapshotStep::Restore
        } else {
            SnapshotStep::Discard
        };
    }
    match (previous == committed, previous == generation, kept) {
        (true, _, true) | (false, true, _) => SnapshotStep::Keep,
        (true, _, false) => SnapshotStep::Take,
        // Staged before snapshots existed: there is nothing to restore.
        (false, false, false) => SnapshotStep::Keep,
        (false, false, true) => SnapshotStep::RestoreAndTake,
    }
}

/// Renames the copy over the data image. The journal still names the copy,
/// so a crash after the rename is settled by `settle_snapshot`.
fn restore_snapshot(directory: &Dir, journal: &mut Journal, kept: Option<Identity>) -> Result<()> {
    let kept = kept.ok_or(Error::Identity)?;
    let copy = files::open_image(directory, SNAPSHOT_IMAGE)?.ok_or(Error::Identity)?;
    files::require(directory, SNAPSHOT_IMAGE, &copy, &kept)?;
    drop(copy);
    files::replace(directory, SNAPSHOT_IMAGE, Role::Data.image())?;
    journal.images[0].inode = Some(kept);
    journal.snapshot = None;
    files::save_journal(directory, journal)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettleStep {
    /// A complete copy is still there: it must be the journaled one.
    Verify,
    /// The restore rename landed before the journal was saved.
    Adopt,
    /// An incomplete or unnamed copy: remove it and forget it.
    Remove,
}

/// Recovery of an interrupted snapshot step, from the journal and whether the
/// copy's file is present.
fn settle_step(snapshot: Option<&Snapshot>, copy_present: bool) -> SettleStep {
    match (snapshot.and_then(|kept| kept.inode.as_ref()), copy_present) {
        (Some(_), true) => SettleStep::Verify,
        (Some(_), false) => SettleStep::Adopt,
        (None, _) => SettleStep::Remove,
    }
}

fn settle_snapshot(directory: &Dir, journal: &mut Journal) -> Result<()> {
    let copy = files::open_image(directory, SNAPSHOT_IMAGE)?;
    let kept = journal
        .snapshot
        .as_ref()
        .and_then(|kept| kept.inode.clone());
    match (
        settle_step(journal.snapshot.as_ref(), copy.is_some()),
        kept,
        copy,
    ) {
        (SettleStep::Verify, Some(kept), Some(copy)) => {
            files::require(directory, SNAPSHOT_IMAGE, &copy, &kept)
        }
        (SettleStep::Adopt, Some(kept), None) => {
            let data = files::open_image(directory, Role::Data.image())?.ok_or(Error::Identity)?;
            files::require(directory, Role::Data.image(), &data, &kept)?;
            journal.images[0].inode = Some(kept);
            journal.snapshot = None;
            files::save_journal(directory, journal)
        }
        (_, _, copy) => {
            if let Some(copy) = copy {
                drop(copy);
                directory.remove_file(OsStr::new(SNAPSHOT_IMAGE))?;
                directory.sync()?;
            }
            if journal.snapshot.take().is_some() {
                files::save_journal(directory, journal)?;
            }
            Ok(())
        }
    }
}
/// The journal forgets the copy first; a leftover file is then unnamed and
/// removed by `settle_snapshot` on the next recovery.
fn discard_snapshot(directory: &Dir, journal: &mut Journal) -> Result<()> {
    if journal.snapshot.take().is_some() {
        files::save_journal(directory, journal)?;
    }
    if let Some(copy) = files::open_image(directory, SNAPSHOT_IMAGE)? {
        drop(copy);
        directory.remove_file(OsStr::new(SNAPSHOT_IMAGE))?;
        directory.sync()?;
    }
    Ok(())
}
fn random() -> Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    let mut done = 0;
    while done < bytes.len() {
        let count =
            unsafe { libc::getrandom(bytes[done..].as_mut_ptr().cast(), bytes.len() - done, 0) };
        if count < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if count <= 0 {
            return Err(Error::Io);
        }
        done += count as usize;
    }
    Ok(bytes)
}
fn random_hex() -> Result<String> {
    Ok(random()?.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn random_uuid() -> Result<String> {
    let mut bytes = random()?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn reserve(free: u64, promised: u64, allocated: u64) -> Result<()> {
    let missing = promised.checked_sub(allocated).ok_or(Error::Identity)?;
    let needed = HOST_RESERVE_BYTES
        .checked_add(missing)
        .ok_or(Error::Capacity)?;
    if free < needed {
        Err(Error::Capacity)
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_snapshot_step_keeps_every_failed_update_off_committed_data() {
        use SnapshotStep::*;
        // (previous, starting, committed, complete copy kept) → step
        for (previous, generation, committed, kept, step) in [
            // Staged on committed data: take a copy (or keep one already taken).
            (1, 2, 1, false, Take),
            (1, 2, 1, true, Keep),
            // The same staged generation retries on its own writes.
            (2, 2, 1, true, Keep),
            // Another staged generation after a failed one: committed data first.
            (2, 3, 1, true, RestoreAndTake),
            // Staged before snapshots existed: nothing to restore.
            (2, 3, 1, false, Keep),
            // The committed generation after a failed update rolls back.
            (2, 1, 1, true, Restore),
            (2, 1, 1, false, Discard),
            // A committed start after itself, or once an update committed.
            (1, 1, 1, true, Discard),
            (2, 2, 2, true, Discard),
        ] {
            assert_eq!(
                snapshot_step(previous, generation, committed, kept),
                step,
                "{previous} -> {generation} (committed {committed}, kept {kept})"
            );
        }
    }
    #[test]
    fn recovery_settles_each_interrupted_snapshot_state() {
        let identity = Identity {
            device: 1,
            inode: 2,
        };
        let complete = Snapshot {
            generation: 1,
            inode: Some(identity.clone()),
        };
        let incomplete = Snapshot {
            generation: 1,
            inode: None,
        };
        // A complete copy that is still present.
        assert_eq!(settle_step(Some(&complete), true), SettleStep::Verify);
        // A restore rename that landed before the journal was saved.
        assert_eq!(settle_step(Some(&complete), false), SettleStep::Adopt);
        // An incomplete copy, present or not yet created.
        assert_eq!(settle_step(Some(&incomplete), true), SettleStep::Remove);
        assert_eq!(settle_step(Some(&incomplete), false), SettleStep::Remove);
        // A copy the journal already forgot (an unnamed leftover).
        assert_eq!(settle_step(None, true), SettleStep::Remove);
    }
    #[test]
    fn existing_directories_do_not_fabricate_disk_reservations() {
        let promised = DATA_BYTES + TMP_BYTES;
        assert_eq!(
            reserve(HOST_RESERVE_BYTES, promised, DATA_BYTES),
            Err(Error::Capacity)
        );
        assert_eq!(
            reserve(HOST_RESERVE_BYTES + TMP_BYTES, promised, DATA_BYTES),
            Ok(())
        );
        assert_eq!(
            reserve(HOST_RESERVE_BYTES + TMP_BYTES, promised, 0),
            Err(Error::Capacity)
        );
        assert_eq!(reserve(HOST_RESERVE_BYTES + promised, promised, 0), Ok(()));
    }
}
