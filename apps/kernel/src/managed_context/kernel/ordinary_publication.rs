//! Durable ownership for entries published into the user's ordinary registries.
//! Rollback detaches an entry into the private import root before verifying and
//! removing it, so a concurrent registry replacement is never recursively erased.
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    import_error, import_io_error, publish_directory_no_clobber, read_bounded_file, sync_directory,
    validate_portable_package_path, write_json_file, DaemonError, MaterializationBudget,
};

const PUBLISHED_ENTRIES_NAME: &str = "published-entries.json";
const ROLLBACK_DIRECTORY: &str = ".ordinary-rollback";
const ORDINARY_REGISTRY_DIRECTORIES: &[&str] = &[
    "mcps",
    "skills",
    "scripts",
    "credentials",
    "envs",
    "envs/.portable",
    "connectors",
    "connectors/definitions",
    "connectors/adapters",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedEntry {
    relative: String,
    identity: EntryIdentity,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryIdentity {
    device: u64,
    inode: u64,
    tree_sha256: String,
}

pub(super) fn record_ordinary_entries(
    staging: &Path,
    home: &Path,
    budget: &mut MaterializationBudget,
) -> Result<(), DaemonError> {
    let mut entries = Vec::new();
    let staged = staging.join("user");
    if staged.exists() {
        collect_entries(&staged, home, Path::new(""), &mut entries)?;
    }
    // Persist ownership before any rename; publication preserves device/inode.
    write_json_file(
        &staging.join(PUBLISHED_ENTRIES_NAME),
        &entries,
        false,
        budget,
    )?;
    sync_directory(staging)
}

fn collect_entries(
    staged: &Path,
    home: &Path,
    relative: &Path,
    entries: &mut Vec<PublishedEntry>,
) -> Result<(), DaemonError> {
    let mut children = fs::read_dir(staged.join(relative))
        .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
        .map_err(|error| import_io_error("read staged kernel context", error))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = relative.join(child.file_name());
        let relative = path
            .to_str()
            .ok_or_else(|| import_error("kernel context entry is not UTF-8"))?
            .to_string();
        if ORDINARY_REGISTRY_DIRECTORIES.contains(&relative.as_str()) {
            collect_entries(staged, home, &path, entries)?;
            continue;
        }
        match fs::symlink_metadata(home.join(&path)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => entries.push(PublishedEntry {
                relative,
                identity: entry_identity(&staged.join(&path))?,
            }),
            Ok(_) => {
                return Err(import_error(format!(
                    "kernel context entry `{relative}` already exists in the ordinary registry"
                )))
            }
            Err(error) => return Err(import_io_error("inspect ordinary registry", error)),
        }
    }
    Ok(())
}

fn read_entries(root: &Path) -> Result<Vec<PublishedEntry>, DaemonError> {
    let bytes = read_bounded_file(&root.join(PUBLISHED_ENTRIES_NAME), 4 * 1024 * 1024)?;
    // Legacy path-only records cannot prove ownership. Preserve them and fail
    // closed instead of guessing which user entry an old import might own.
    let entries: Vec<PublishedEntry> = serde_json::from_slice(&bytes).map_err(|_| {
        import_error("published kernel context ownership records are invalid or unavailable")
    })?;
    let mut paths = std::collections::BTreeSet::new();
    for entry in &entries {
        validate_portable_package_path(&entry.relative)?;
        if !paths.insert(&entry.relative)
            || entry.identity.tree_sha256.len() != 64
            || !entry
                .identity
                .tree_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(import_error(
                "published kernel context ownership records are invalid",
            ));
        }
    }
    Ok(entries)
}

pub(super) fn publish_ordinary_entries(root: &Path, home: &Path) -> Result<(), DaemonError> {
    for entry in read_entries(root)? {
        let source = root.join("user").join(&entry.relative);
        let destination = home.join(&entry.relative);
        if !present(&source)? {
            require_identity(&destination, &entry.identity)?;
            continue;
        }
        require_identity(&source, &entry.identity)?;
        let parent = destination
            .parent()
            .ok_or_else(|| import_error("published kernel context entry has no parent"))?;
        fs::create_dir_all(parent)
            .map_err(|error| import_io_error("create ordinary registry directory", error))?;
        publish_directory_no_clobber(&source, &destination)?;
        sync_directory(parent)?;
        sync_directory(source.parent().expect("staged entry parent"))?;
    }
    Ok(())
}

pub(super) fn remove_ordinary_entries(root: &Path, home: &Path) -> Result<(), DaemonError> {
    remove_ordinary_entries_with_phase_writer(root, home, |path, identity| {
        write_json_file(path, identity, false, &mut MaterializationBudget::new())
    })
}

fn remove_ordinary_entries_with_phase_writer(
    root: &Path,
    home: &Path,
    mut write_phase: impl FnMut(&Path, &EntryIdentity) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    let entries = read_entries(root)?;
    let rollback = root.join(ROLLBACK_DIRECTORY);
    super::ensure_private_directory(&rollback)?;
    sync_directory(root)?;
    for (index, entry) in entries.iter().enumerate() {
        if present(&root.join("user").join(&entry.relative))? {
            continue;
        }
        let destination = home.join(&entry.relative);
        let detached = rollback.join(index.to_string());
        let deleting = rollback.join(format!("deleting-{index}.json"));
        let deletion_started = present(&deleting)?;
        // A completed deletion must never detach a later user replacement.
        if deletion_started && !present(&detached)? {
            continue;
        }
        if !present(&detached)? {
            if !present(&destination)? {
                continue;
            }
            require_identity(&destination, &entry.identity)?;
            publish_directory_no_clobber(&destination, &detached)?;
            sync_directory(destination.parent().expect("published entry parent"))?;
            sync_directory(&rollback)?;
        }
        if deletion_started {
            let phase: EntryIdentity = serde_json::from_slice(&read_bounded_file(&deleting, 4096)?)
                .map_err(|_| import_error("invalid ordinary rollback deletion phase"))?;
            if phase != entry.identity {
                return Err(import_error(
                    "ordinary rollback deletion phase does not match publication",
                ));
            }
            require_inode(&detached, &entry.identity)?;
        } else if let Err(error) = require_identity(&detached, &entry.identity) {
            // A replacement racing the detach belongs to the user. Restore it
            // without clobbering a newer replacement, or retain it in this root.
            let _ = publish_directory_no_clobber(&detached, &destination);
            sync_directory(destination.parent().expect("published entry parent"))?;
            sync_directory(&rollback)?;
            return Err(error);
        }
        if !deletion_started {
            // Persist verified ownership before the first destructive unlink.
            // After a crash the tree digest may differ, but the private detached
            // inode and this phase still authorize finishing this deletion only.
            // Incomplete writes remain pending and carry no deletion authority.
            // Retry has just revalidated the intact tree, so it can discard an
            // interrupted pending file and create a fresh, fsynced record.
            let pending = deleting.with_extension("pending");
            if present(&pending)? {
                fs::remove_file(&pending)
                    .map_err(|error| import_io_error("remove pending rollback phase", error))?;
            }
            // The production writer fsyncs the new file before returning.
            write_phase(&pending, &entry.identity)?;
            publish_directory_no_clobber(&pending, &deleting)?;
        }
        // Also sync on resume: publication may have succeeded just before a
        // directory-sync failure. No child unlink may precede this durable phase.
        sync_directory(&rollback)?;
        let metadata = fs::symlink_metadata(&detached)
            .map_err(|error| import_io_error("inspect detached import entry", error))?;
        let removed = if metadata.is_dir() {
            fs::remove_dir_all(&detached)
        } else {
            fs::remove_file(&detached)
        };
        removed.map_err(|error| import_io_error("remove owned kernel context entry", error))?;
        sync_directory(&rollback)?;
    }
    Ok(())
}

fn present(path: &Path) -> Result<bool, DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(import_io_error("inspect kernel context entry", error)),
    }
}

#[cfg(unix)]
fn require_inode(path: &Path, expected: &EntryIdentity) -> Result<(), DaemonError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| import_io_error("inspect deleting import entry", error))?;
    if metadata.dev() != expected.device
        || metadata.ino() != expected.inode
        || !(metadata.is_dir() || metadata.is_file())
    {
        return Err(import_error(
            "deleting kernel context entry changed; preserving user data",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_inode(_path: &Path, _expected: &EntryIdentity) -> Result<(), DaemonError> {
    Err(import_error(
        "ordinary registry ownership requires Unix filesystem identity",
    ))
}

fn require_identity(path: &Path, expected: &EntryIdentity) -> Result<(), DaemonError> {
    if entry_identity(path)? != *expected {
        return Err(import_error(
            "published kernel context entry changed; preserving user data",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn entry_identity(path: &Path) -> Result<EntryIdentity, DaemonError> {
    let ((device, inode), tree_sha256) = super::import_ownership::entry_fingerprint(path)?;
    Ok(EntryIdentity { device, inode, tree_sha256 })
}

#[cfg(not(unix))]
fn entry_identity(_path: &Path) -> Result<EntryIdentity, DaemonError> {
    Err(import_error(
        "ordinary registry ownership requires Unix filesystem identity",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "mp11-ordinary-publication-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let context = root.join("context");
        let home = root.join("home");
        fs::create_dir_all(context.join("user/skills/imported")).unwrap();
        fs::create_dir_all(home.join("skills")).unwrap();
        fs::write(context.join("user/skills/imported/original"), b"imported").unwrap();
        fs::write(home.join("skills/unrelated"), b"user-owned").unwrap();
        record_ordinary_entries(&context, &home, &mut MaterializationBudget::new()).unwrap();
        publish_ordinary_entries(&context, &home).unwrap();
        (root, context, home)
    }

    fn detach(context: &Path, home: &Path) -> PathBuf {
        let rollback = context.join(ROLLBACK_DIRECTORY);
        super::super::ensure_private_directory(&rollback).unwrap();
        let detached = rollback.join("0");
        fs::rename(home.join("skills/imported"), &detached).unwrap();
        detached
    }

    #[test]
    fn mp11_f4_interrupted_rollback_resumes_the_same_owned_entry() {
        let (root, context, home) = fixture("interrupted");
        publish_ordinary_entries(&context, &home).unwrap();
        let detached = detach(&context, &home);
        remove_ordinary_entries(&context, &home).unwrap();
        assert!(!detached.exists());
        assert!(!home.join("skills/imported").exists());
        assert_eq!(
            fs::read(home.join("skills/unrelated")).unwrap(),
            b"user-owned"
        );
        remove_ordinary_entries(&context, &home).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    struct FixtureCleanup(PathBuf);

    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove owned MP-11 synthetic fixture");
        }
    }

    #[test]
    fn mp11_f4_failed_phase_write_preserves_intact_tree_and_allows_retry() {
        for partial in [b"".as_slice(), b"{\"device\":".as_slice()] {
            let (root, context, home) = fixture("failed-phase-write");
            let _cleanup = FixtureCleanup(root);
            let entry = read_entries(&context).unwrap().remove(0);
            let rollback = context.join(ROLLBACK_DIRECTORY);
            let deleting = rollback.join("deleting-0.json");
            let failed = remove_ordinary_entries_with_phase_writer(&context, &home, |path, _| {
                // Disk exhaustion or a failed sync after file creation must
                // never turn an incomplete record into deletion authority.
                fs::write(path, partial).unwrap();
                Err(import_io_error(
                    "injected phase write failure",
                    io::Error::from_raw_os_error(libc::ENOSPC),
                ))
            });
            assert!(failed.is_err());
            let detached = rollback.join("0");
            require_identity(&detached, &entry.identity).unwrap();
            assert!(
                !deleting.exists(),
                "failed phase write published final journal"
            );
            fs::create_dir(home.join("skills/imported")).unwrap();
            fs::write(
                home.join("skills/imported/replacement"),
                b"user replacement",
            )
            .unwrap();
            remove_ordinary_entries(&context, &home).unwrap();
            assert!(!detached.exists());
            assert!(!deleting.with_extension("pending").exists());
            assert_eq!(
                fs::read(home.join("skills/imported/replacement")).unwrap(),
                b"user replacement"
            );
            assert_eq!(
                fs::read(home.join("skills/unrelated")).unwrap(),
                b"user-owned"
            );
            remove_ordinary_entries(&context, &home).unwrap();
        }
    }

    #[test]
    fn mp11_f4_interrupted_pending_phase_write_is_revalidated_before_retry() {
        for modified in [false, true] {
            let (root, context, home) = fixture("interrupted-phase-write");
            let _cleanup = FixtureCleanup(root);
            let detached = detach(&context, &home);
            let deleting = context.join(ROLLBACK_DIRECTORY).join("deleting-0.json");
            let pending = deleting.with_extension("pending");
            // Simulate process interruption while the private pending file
            // is incomplete; it cannot authorize any unlink on the next run.
            fs::write(&pending, b"{\"device\":").unwrap();
            if modified {
                fs::write(detached.join("user-addition"), b"user data").unwrap();
                assert!(remove_ordinary_entries(&context, &home).is_err());
                assert!(!deleting.exists());
                assert_eq!(
                    fs::read(home.join("skills/imported/user-addition")).unwrap(),
                    b"user data"
                );
            } else {
                remove_ordinary_entries(&context, &home).unwrap();
                assert!(!detached.exists());
                assert!(
                    !pending.exists(),
                    "interrupted pending phase was not replaced"
                );
                let phase: EntryIdentity =
                    serde_json::from_slice(&fs::read(&deleting).unwrap()).unwrap();
                assert!(phase == read_entries(&context).unwrap().remove(0).identity);
            }
            assert_eq!(
                fs::read(home.join("skills/unrelated")).unwrap(),
                b"user-owned"
            );
        }
    }

    #[test]
    fn mp11_f4_partial_deletion_resumes_without_republishing() {
        let (root, context, home) = fixture("partial-deletion");
        let entry = read_entries(&context).unwrap().remove(0);
        let detached = detach(&context, &home);
        // Simulate the durable verified-deletion phase, followed by a crash
        // after recursive removal has already unlinked one original child.
        write_json_file(
            &context.join(ROLLBACK_DIRECTORY).join("deleting-0.json"),
            &entry.identity,
            false,
            &mut MaterializationBudget::new(),
        )
        .unwrap();
        fs::remove_file(detached.join("original")).unwrap();
        fs::create_dir(home.join("skills/imported")).unwrap();
        fs::write(
            home.join("skills/imported/replacement"),
            b"user replacement",
        )
        .unwrap();
        remove_ordinary_entries(&context, &home).unwrap();
        assert!(!detached.exists());
        assert_eq!(
            fs::read(home.join("skills/imported/replacement")).unwrap(),
            b"user replacement"
        );
        remove_ordinary_entries(&context, &home).unwrap();
        assert_eq!(
            fs::read(home.join("skills/unrelated")).unwrap(),
            b"user-owned"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp11_f4_deletion_phase_preserves_replaced_detached_inode() {
        let (root, context, home) = fixture("deleting-replacement");
        let entry = read_entries(&context).unwrap().remove(0);
        let detached = detach(&context, &home);
        write_json_file(
            &context.join(ROLLBACK_DIRECTORY).join("deleting-0.json"),
            &entry.identity,
            false,
            &mut MaterializationBudget::new(),
        )
        .unwrap();
        fs::rename(&detached, root.join("reserved-owned-inode")).unwrap();
        fs::create_dir(&detached).unwrap();
        fs::write(detached.join("foreign"), b"foreign data").unwrap();
        assert!(remove_ordinary_entries(&context, &home).is_err());
        assert_eq!(fs::read(detached.join("foreign")).unwrap(), b"foreign data");
        assert!(!home.join("skills/imported").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp11_f4_detached_replacement_is_restored_or_retained_without_clobber() {
        for collision in [false, true] {
            let (root, context, home) = fixture("detach-race");
            let destination = home.join("skills/imported");
            fs::rename(&destination, root.join("old-owned-inode")).unwrap();
            fs::create_dir(&destination).unwrap();
            fs::write(destination.join("replacement"), b"first user replacement").unwrap();
            // Simulate replacement in the precheck/detach window. Only the
            // post-detach identity check may authorize recursive deletion.
            let detached = detach(&context, &home);
            if collision {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("newer"), b"second user replacement").unwrap();
            }
            assert!(remove_ordinary_entries(&context, &home).is_err());
            let preserved = if collision { &detached } else { &destination };
            assert_eq!(
                fs::read(preserved.join("replacement")).unwrap(),
                b"first user replacement"
            );
            if collision {
                assert_eq!(
                    fs::read(destination.join("newer")).unwrap(),
                    b"second user replacement"
                );
            }
            assert!(context.is_dir());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn mp11_f4_legacy_path_only_journal_preserves_user_entries() {
        let (root, context, home) = fixture("legacy");
        fs::write(
            context.join(PUBLISHED_ENTRIES_NAME),
            br#"["skills/imported"]"#,
        )
        .unwrap();
        assert!(remove_ordinary_entries(&context, &home).is_err());
        assert_eq!(
            fs::read(home.join("skills/imported/original")).unwrap(),
            b"imported"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp11_f4_publication_replay_rejects_changed_destination_and_preserves_staging() {
        let (root, context, home) = fixture("replay");
        fs::write(home.join("skills/imported/new"), b"user addition").unwrap();
        assert!(publish_ordinary_entries(&context, &home).is_err());
        assert!(remove_ordinary_entries(&context, &home).is_err());
        assert_eq!(
            fs::read(home.join("skills/imported/new")).unwrap(),
            b"user addition"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
