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

#[test]
fn created_scaffold_packs_and_validates_with_no_signing_secret_or_machine_path() {
    let scratch = Scratch::new();
    let keys_dir = scratch.0.join("keys");
    disk::DirBuilder::new()
        .mode(0o700)
        .create(&keys_dir)
        .unwrap();
    let key = keys_dir.join("signing.key");
    let public = scratch.0.join("public.json");
    super::keygen("com.example", "Developer", &key, &public).unwrap();
    let project = scratch.0.join("my-app");
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    };
    let created = super::run_cli(&args(&[
        "create",
        project.to_str().unwrap(),
        "--app-id",
        "com.example.greeting",
        "--publisher",
        public.to_str().unwrap(),
        "--kernel-protocol",
        "500",
    ]))
    .unwrap();
    assert_eq!(created["status"], "created-locally");
    assert_eq!(created["manifest"]["version"], "0.1.0");
    assert_eq!(created["manifest"]["minKernelProtocol"], 500);
    let manifest = super::read_manifest(&project.join("app.json"), &Limits::default()).unwrap();
    assert_eq!(manifest.runtime.entry, "runtime/main.mjs");
    let seed = disk::read(&key).unwrap();
    fn verify_source(path: &std::path::Path, seed: &[u8], external_path: &str) -> usize {
        let mut files = 0;
        for entry in disk::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                files += verify_source(&entry.path(), seed, external_path);
            } else {
                let bytes = disk::read(entry.path()).unwrap();
                assert!(!bytes.windows(seed.len()).any(|window| window == seed));
                assert!(!String::from_utf8_lossy(&bytes).contains(external_path));
                files += 1;
            }
        }
        files
    }
    assert_eq!(
        verify_source(&project, &seed, scratch.0.to_str().unwrap()),
        7
    );
    let archive = scratch.0.join("greeting.cxapp");
    let report = super::pack_directory(
        &project.join("bundle"),
        &manifest,
        &key,
        &archive,
        500,
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(report.file_count, Some(3));
    let publisher =
        super::read_publisher(&project.join("publisher.json"), &Limits::default()).unwrap();
    let verified = super::validate_archive(&archive, &publisher, 500, &Limits::default()).unwrap();
    assert_eq!(verified.package_digest, report.package_digest);
}

#[test]
fn scaffold_never_overwrites_existing_empty_populated_or_linked_destinations() {
    let scratch = Scratch::new();
    let key = scratch.0.join("key");
    let public = scratch.0.join("publisher.json");
    super::keygen("com.example", "Developer", &key, &public).unwrap();
    let publisher = super::read_publisher(&public, &Limits::default()).unwrap();
    let empty = scratch.0.join("empty");
    let populated = scratch.0.join("populated");
    disk::create_dir(&empty).unwrap();
    disk::create_dir(&populated).unwrap();
    disk::write(populated.join("keep.txt"), "user source").unwrap();
    let linked = scratch.0.join("linked");
    std::os::unix::fs::symlink(&empty, &linked).unwrap();
    for destination in [&empty, &populated, &linked] {
        assert!(super::create_scaffold(
            destination,
            "com.example.test",
            "0.1.0",
            &publisher,
            500,
            &Limits::default()
        )
        .is_err());
    }
    assert_eq!(disk::read_dir(&empty).unwrap().count(), 0);
    assert_eq!(
        disk::read(populated.join("keep.txt")).unwrap(),
        b"user source"
    );
    assert_eq!(disk::read_dir(&populated).unwrap().count(), 1);
    assert!(disk::symlink_metadata(&linked)
        .unwrap()
        .file_type()
        .is_symlink());
    let invalid = scratch.0.join("invalid");
    assert!(super::create_scaffold(
        &invalid,
        "invalid app ID",
        "0.1.0",
        &publisher,
        500,
        &Limits::default()
    )
    .is_err());
    assert!(!invalid.exists());
}
