//! Durable ownership for entries published into the user's ordinary registries.
//! Rollback detaches an entry into the private import root before verifying and
//! removing it, so a concurrent registry replacement is never recursively erased.
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    import_error, import_io_error, publish_directory_no_clobber, read_bounded_file, sync_directory,
    validate_portable_package_path, write_json_file, DaemonError, MaterializationBudget,
    MAX_MATERIALIZED_CONTEXT_BYTES, MAX_MATERIALIZED_CONTEXT_ENTRIES,
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
        if !present(&detached)? {
            if !present(&destination)? {
                continue;
            }
            require_identity(&destination, &entry.identity)?;
            publish_directory_no_clobber(&destination, &detached)?;
            sync_directory(destination.parent().expect("published entry parent"))?;
            sync_directory(&rollback)?;
        }
        if let Err(error) = require_identity(&detached, &entry.identity) {
            // A replacement racing the detach belongs to the user. Restore it
            // without clobbering a newer replacement, or retain it in this root.
            let _ = publish_directory_no_clobber(&detached, &destination);
            sync_directory(destination.parent().expect("published entry parent"))?;
            sync_directory(&rollback)?;
            return Err(error);
        }
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
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    fn hash_entry(
        path: &Path,
        hash: &mut Sha256,
        budget: &mut MaterializationBudget,
        depth: usize,
    ) -> Result<(), DaemonError> {
        if depth > 128 {
            return Err(import_error(
                "kernel context ownership tree exceeds its depth limit",
            ));
        }
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| import_io_error("inspect import ownership", error))?;
        hash.update(metadata.dev().to_be_bytes());
        hash.update(metadata.ino().to_be_bytes());
        hash.update(metadata.mode().to_be_bytes());
        if metadata.is_dir() {
            budget.directory()?;
            hash.update(b"directory\0");
            let mut children = fs::read_dir(path)
                .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
                .map_err(|error| import_io_error("read import ownership tree", error))?;
            children.sort_by_key(|entry| entry.file_name());
            hash.update((children.len() as u64).to_be_bytes());
            for child in children {
                use std::os::unix::ffi::OsStrExt;
                let name = child.file_name();
                hash.update((name.as_bytes().len() as u64).to_be_bytes());
                hash.update(name.as_bytes());
                hash_entry(&child.path(), hash, budget, depth + 1)?;
            }
        } else if metadata.is_file() {
            budget.file(metadata.len())?;
            hash.update(b"file\0");
            hash.update(metadata.len().to_be_bytes());
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(|error| import_io_error("open import ownership file", error))?;
            let opened = file
                .metadata()
                .map_err(|error| import_io_error("inspect import ownership file", error))?;
            if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
                return Err(import_error(
                    "kernel context entry changed during ownership check",
                ));
            }
            let mut read = 0u64;
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let length = file
                    .read(&mut buffer)
                    .map_err(|error| import_io_error("hash import ownership file", error))?;
                if length == 0 {
                    break;
                }
                read += length as u64;
                if read > metadata.len() {
                    return Err(import_error(
                        "kernel context entry changed during ownership check",
                    ));
                }
                hash.update(&buffer[..length]);
            }
            if read != metadata.len() {
                return Err(import_error(
                    "kernel context entry changed during ownership check",
                ));
            }
        } else {
            return Err(import_error(
                "kernel context ownership entry is not a regular file or directory",
            ));
        }
        let after = fs::symlink_metadata(path)
            .map_err(|error| import_io_error("recheck import ownership", error))?;
        if metadata.dev() != after.dev()
            || metadata.ino() != after.ino()
            || metadata.mode() != after.mode()
            || metadata.len() != after.len()
            || metadata.mtime() != after.mtime()
            || metadata.mtime_nsec() != after.mtime_nsec()
            || metadata.ctime() != after.ctime()
            || metadata.ctime_nsec() != after.ctime_nsec()
        {
            return Err(import_error(
                "kernel context entry changed during ownership check",
            ));
        }
        Ok(())
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| import_io_error("inspect import ownership", error))?;
    let mut hash = Sha256::new();
    hash.update(b"chariox.kernel-context-entry.v1\0");
    let mut budget = MaterializationBudget {
        bytes: 0,
        entries: 0,
    };
    hash_entry(path, &mut hash, &mut budget, 0)?;
    if budget.bytes > MAX_MATERIALIZED_CONTEXT_BYTES
        || budget.entries > MAX_MATERIALIZED_CONTEXT_ENTRIES
    {
        return Err(import_error(
            "kernel context ownership exceeds resource limits",
        ));
    }
    Ok(EntryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        tree_sha256: format!("{:x}", hash.finalize()),
    })
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
