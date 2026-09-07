use std::{
    fs as disk,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
};

use rand::{rngs::OsRng, RngCore};

use super::{fs, keys, read_bundle_open};
use crate::{ErrorCode, Limits};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = disk::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-publish-race-{}-{:x}",
                std::process::id(),
                OsRng.next_u64()
            ));
        disk::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        disk::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn renamed_bundle_and_output_paths_keep_the_selected_directories() {
    let scratch = Scratch::new();
    let bundle = scratch.0.join("bundle");
    let output = scratch.0.join("output");
    disk::create_dir(&bundle).unwrap();
    disk::create_dir(&output).unwrap();
    disk::write(bundle.join("selected.js"), "selected bytes").unwrap();
    let opened = fs::Directory::open(&bundle).unwrap();
    let destination = fs::Destination::open(&output.join("result"), false).unwrap();
    fs::outside_bundle(&opened, &destination.directory).unwrap();
    disk::rename(&bundle, scratch.0.join("selected-bundle")).unwrap();
    disk::rename(&output, scratch.0.join("selected-output")).unwrap();
    disk::create_dir(&bundle).unwrap();
    disk::create_dir(&output).unwrap();
    disk::write(bundle.join("replacement.js"), "must not be packaged").unwrap();
    let files = read_bundle_open(&opened, &Limits::default(), None).unwrap();
    assert_eq!(files.keys().collect::<Vec<_>>(), vec!["selected.js"]);
    destination
        .prepare(b"selected result")
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(
        disk::read(scratch.0.join("selected-output/result")).unwrap(),
        b"selected result"
    );
    assert!(!output.join("result").exists());
}

#[test]
fn signing_inode_moved_into_the_open_bundle_is_rejected_before_reading() {
    let scratch = Scratch::new();
    let keys_directory = scratch.0.join("keys");
    let bundle = scratch.0.join("bundle");
    disk::DirBuilder::new()
        .mode(0o700)
        .create(&keys_directory)
        .unwrap();
    disk::create_dir(&bundle).unwrap();
    let private = keys_directory.join("private");
    keys::keygen(
        "com.example",
        "Developer",
        &private,
        &scratch.0.join("public.json"),
    )
    .unwrap();
    let signing = keys::read_signing_key(&private).unwrap();
    let opened = fs::Directory::open(&bundle).unwrap();
    fs::outside_bundle(&opened, &signing.directory).unwrap();
    disk::rename(&private, bundle.join("secret.bin")).unwrap();
    assert_eq!(
        read_bundle_open(
            &opened,
            &Limits::default(),
            Some(fs::identity(&signing.file).unwrap())
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidPath
    );
}

#[test]
fn replaced_temporary_name_is_neither_published_nor_deleted() {
    let scratch = Scratch::new();
    let output = scratch.0.join("result");
    let pending = fs::PreparedFile::new(&output, b"expected", false).unwrap();
    let temporary = disk::read_dir(&scratch.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".cxapp-write-")
        })
        .unwrap();
    disk::remove_file(&temporary).unwrap();
    disk::write(&temporary, "replacement must survive").unwrap();
    assert!(pending.publish().is_err());
    assert_eq!(disk::read(&temporary).unwrap(), b"replacement must survive");
    assert!(!output.exists());
}

#[test]
fn public_output_rejects_a_directory_writable_by_other_users() {
    let scratch = Scratch::new();
    disk::set_permissions(&scratch.0, disk::Permissions::from_mode(0o777)).unwrap();
    assert!(
        matches!(fs::PreparedFile::new(&scratch.0.join("result"), b"data", false), Err(error) if error.code == ErrorCode::InvalidPath)
    );
    assert_eq!(disk::read_dir(&scratch.0).unwrap().count(), 0);
}
