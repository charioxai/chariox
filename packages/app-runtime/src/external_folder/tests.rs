use super::*;
use std::{fs, os::unix::fs::symlink, path::PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("chariox-folder-{:032x}", rand::random::<u128>()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn folder(&self) -> ExternalFolder {
        ExternalFolder::select(&self.0).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn selected_folder_reopens_from_serialized_identity_and_round_trips_a_file() {
    let scratch = Scratch::new();
    fs::create_dir(scratch.0.join("notes")).unwrap();
    let identity = scratch.folder().identity();
    let serialized = serde_json::to_vec(&identity).unwrap();
    let folder =
        ExternalFolder::reopen(&scratch.0, serde_json::from_slice(&serialized).unwrap()).unwrap();
    folder
        .prepare_replace("notes/a.md", b"# First")
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(folder.read("notes/a.md").unwrap(), b"# First");
    folder
        .prepare_replace("notes/a.md", b"# Second")
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(folder.read("notes/a.md").unwrap(), b"# Second");
    assert_eq!(
        folder.list("notes").unwrap(),
        [FolderEntry {
            name: "a.md".into(),
            kind: FolderEntryKind::File,
            bytes: 8
        }]
    );
    assert_eq!(
        scratch.folder().identity(),
        identity,
        "writing children must not invalidate the folder's identity"
    );
}

#[test]
fn malformed_paths_links_devices_and_aliases_cannot_read_or_write_outside_the_folder() {
    let scratch = Scratch::new();
    let outside = Scratch::new();
    fs::write(outside.0.join("outside"), b"untouched").unwrap();
    symlink(&outside.0, scratch.0.join("link")).unwrap();
    symlink(outside.0.join("outside"), scratch.0.join("file-link")).unwrap();
    fs::hard_link(outside.0.join("outside"), scratch.0.join("alias")).unwrap();
    symlink("/dev/zero", scratch.0.join("device")).unwrap();
    let folder = scratch.folder();
    for path in [
        "",
        "/etc/passwd",
        "../outside",
        "./outside",
        "a/../outside",
        "a//b",
        "a/",
        "C:\\x",
        "a\0b",
        "link/outside",
        "file-link",
        "alias",
        "device",
    ] {
        assert!(folder.read(path).is_err(), "read {path:?}");
        assert!(
            folder.prepare_replace(path, b"wrong").is_err(),
            "write {path:?}"
        );
    }
    assert!(folder.list("").unwrap().is_empty());
    assert_eq!(fs::read(outside.0.join("outside")).unwrap(), b"untouched");
    assert!(ExternalFolder::select(&scratch.0.join("link")).is_err());
}

#[test]
fn a_renamed_or_replaced_root_cannot_inherit_a_persisted_grant() {
    let scratch = Scratch::new();
    let selected = scratch.0.join("selected");
    fs::create_dir(&selected).unwrap();
    let identity = ExternalFolder::select(&selected).unwrap().identity();
    fs::rename(&selected, scratch.0.join("moved")).unwrap();
    assert!(matches!(
        ExternalFolder::reopen(&selected, identity),
        Err(FolderError::Identity)
    ));
    fs::create_dir(&selected).unwrap();
    assert!(matches!(
        ExternalFolder::reopen(&selected, identity),
        Err(FolderError::Identity)
    ));
    fs::remove_dir(&selected).unwrap();
    symlink(scratch.0.join("moved"), &selected).unwrap();
    assert!(matches!(
        ExternalFolder::reopen(&selected, identity),
        Err(FolderError::Identity)
    ));
}

#[test]
fn an_ancestor_swap_after_admission_does_not_redirect_publication() {
    let scratch = Scratch::new();
    let outside = Scratch::new();
    fs::create_dir(scratch.0.join("notes")).unwrap();
    fs::write(outside.0.join("a.md"), b"outside").unwrap();
    let staged = scratch
        .folder()
        .prepare_replace("notes/a.md", b"admitted")
        .unwrap();
    fs::rename(scratch.0.join("notes"), scratch.0.join("moved")).unwrap();
    symlink(&outside.0, scratch.0.join("notes")).unwrap();
    staged.publish().unwrap();
    assert_eq!(fs::read(scratch.0.join("moved/a.md")).unwrap(), b"admitted");
    assert_eq!(fs::read(outside.0.join("a.md")).unwrap(), b"outside");
    assert!(scratch.folder().read("notes/a.md").is_err());
}

#[test]
fn a_cancelled_staging_is_cleaned_and_a_post_rename_failure_is_uncertain() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    fs::write(scratch.0.join("a.md"), b"old").unwrap();
    drop(folder.prepare_replace("a.md", b"cancelled").unwrap());
    assert_eq!(folder.read("a.md").unwrap(), b"old");
    assert_eq!(folder.list("").unwrap().len(), 1);
    let result = folder
        .prepare_replace("a.md", b"published")
        .unwrap()
        .publish_checked(|| Err(std::io::Error::other("sync lost")));
    assert_eq!(result, Err(FolderError::OutcomeUncertain));
    assert_eq!(folder.read("a.md").unwrap(), b"published");
    assert_eq!(folder.list("").unwrap().len(), 1);
}

#[test]
fn replacement_of_a_staged_inode_cannot_publish_an_attackers_file() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    let staged = folder.prepare_replace("a.md", b"staged").unwrap();
    let temporary = scratch.0.join(&staged.temporary);
    fs::remove_file(&temporary).unwrap();
    fs::write(&temporary, b"replacement").unwrap();
    assert_eq!(staged.publish(), Err(FolderError::Unsafe));
    assert!(!scratch.0.join("a.md").exists());
    assert_eq!(
        fs::read(temporary).unwrap(),
        b"replacement",
        "cleanup must not unlink a replacement inode"
    );
}

#[test]
fn file_and_directory_limits_are_enforced_before_returning_contents() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    assert!(matches!(
        folder.prepare_replace("large", &vec![0; MAX_FILE_BYTES + 1]),
        Err(FolderError::Limit)
    ));
    fs::write(scratch.0.join("large"), vec![0; MAX_FILE_BYTES + 1]).unwrap();
    assert_eq!(folder.read("large"), Err(FolderError::Limit));
    for index in 0..MAX_DIRECTORY_ENTRIES {
        fs::write(scratch.0.join(format!("entry-{index}")), b"").unwrap();
    }
    assert_eq!(folder.list(""), Err(FolderError::Limit));
}

#[test]
fn unpublished_staging_is_neither_listed_nor_readable_through_another_folder_handle() {
    let scratch = Scratch::new();
    let writer = scratch.folder();
    let reader = scratch.folder();
    let staged = writer
        .prepare_replace("notes.md", b"not published yet")
        .unwrap();
    assert!(
        reader.list("").unwrap().is_empty(),
        "another App must not discover uncommitted staging names"
    );
    assert!(
        reader.read(staged.temporary.to_str().unwrap()).is_err(),
        "guessing a staging name must not expose its bytes"
    );
    staged.publish().unwrap();
    assert_eq!(reader.read("notes.md").unwrap(), b"not published yet");
}
