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
fn created_scaffold_targets_wasm_without_signing_secrets_or_machine_paths() {
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
    assert_eq!(created["manifest"]["schema"], "chariox.app.v2");
    let manifest: crate::v2::ManifestV2 =
        serde_json::from_slice(&disk::read(project.join("app.json")).unwrap()).unwrap();
    manifest.validate(&Limits::default()).unwrap();
    assert_eq!(manifest.runtime.engine, crate::v2::RuntimeEngineV2::Wasm);
    assert_eq!(manifest.runtime.component, "runtime/app.wasm");
    assert!(disk::read_to_string(project.join("src/app.ts"))
        .unwrap()
        .contains("@chariox/app-sdk-wasm"));
    assert!(!project.join("bundle/runtime/main.mjs").exists());
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
    // A scaffold supplies source, not a pretend component. Packing requires
    // the SDK build to produce the admitted WASM component first.
    assert!(super::v2::pack_directory(
        &project.join("bundle"),
        &project.join("app.json"),
        &key,
        &scratch.0.join("greeting.cxapp"),
        &Limits::default(),
    )
    .is_err());
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

#[test]
fn numbered_migration_files_become_the_packed_chain_and_gaps_are_refused() {
    let scratch = Scratch::new();
    let keys_dir = scratch.0.join("keys");
    disk::DirBuilder::new()
        .mode(0o700)
        .create(&keys_dir)
        .unwrap();
    let key = keys_dir.join("signing.key");
    let public = scratch.0.join("public.json");
    super::keygen("com.example", "Developer", &key, &public).unwrap();
    let project = scratch.0.join("app");
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    };
    super::run_cli(&args(&[
        "create",
        project.to_str().unwrap(),
        "--app-id",
        "com.example.migrating",
        "--publisher",
        public.to_str().unwrap(),
        "--kernel-protocol",
        "500",
    ]))
    .unwrap();
    let publisher = super::read_publisher(&public, &Limits::default()).unwrap();
    let mut options = super::ManifestOptions::new(
        "com.example.migrating".into(),
        "0.1.0".into(),
        publisher.publisher,
        500,
    );
    options.tools = Some("schemas/tools.json".into());
    let manifest = super::generate_manifest(options, &Limits::default()).unwrap();
    disk::create_dir(project.join("bundle/runtime")).unwrap();
    disk::write(
        project.join("bundle/runtime/main.js"),
        "// package-format fixture",
    )
    .unwrap();
    assert!(manifest.migrations.is_none());
    let migrations = project.join("bundle/migrations");
    disk::create_dir(&migrations).unwrap();
    disk::write(migrations.join("001.js"), "export default async () => {}").unwrap();
    disk::write(migrations.join("002.mjs"), "export default async () => {}").unwrap();
    let pack = |name: &str| {
        super::pack_directory(
            &project.join("bundle"),
            &manifest,
            &key,
            &scratch.0.join(name),
            500,
            &Limits::default(),
        )
    };
    let report = pack("two.cxapp").unwrap();
    let chain = report.manifest.migrations.unwrap();
    assert_eq!(chain.target_version, 2);
    assert_eq!(
        chain
            .steps
            .iter()
            .map(|step| (step.from, step.to, step.entry.as_str()))
            .collect::<Vec<_>>(),
        [(0, 1, "migrations/001.js"), (1, 2, "migrations/002.mjs")]
    );
    disk::write(migrations.join("004.js"), "export default async () => {}").unwrap();
    let refused = pack("gap.cxapp").unwrap_err();
    assert_eq!(refused.code, ErrorCode::UnexpectedEntry);
    disk::remove_file(migrations.join("004.js")).unwrap();
    // Only three plain digits count: "+03" parses as 3 but is refused, as is
    // a file under migrations/ that is not a numbered step.
    for stray in ["+03.js", "setup.js"] {
        disk::write(migrations.join(stray), "export default async () => {}").unwrap();
        assert_eq!(
            pack("stray.cxapp").unwrap_err().code,
            ErrorCode::UnexpectedEntry,
            "{stray}"
        );
        disk::remove_file(migrations.join(stray)).unwrap();
    }
    // A manifest that declares its own chain is packed as written.
    let mut declared = manifest.clone();
    declared.migrations = Some(chain.clone());
    let report = super::pack_directory(
        &project.join("bundle"),
        &declared,
        &key,
        &scratch.0.join("declared.cxapp"),
        500,
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(report.manifest.migrations, Some(chain));
}

#[test]
fn manifest_cli_emits_contract2_wasm_instead_of_a_native_backend() {
    let scratch = Scratch::new();
    let public = scratch.0.join("publisher.json");
    super::keygen(
        "com.example",
        "Developer",
        &scratch.0.join("signing.key"),
        &public,
    )
    .unwrap();
    let output = scratch.0.join("app.json");
    let args = [
        "manifest",
        "--app-id",
        "com.example.test",
        "--version",
        "1.0.0",
        "--publisher",
        public.to_str().unwrap(),
        "--kernel-protocol",
        "962",
        "--output",
        output.to_str().unwrap(),
    ]
    .map(str::to_owned);
    let response = super::run_cli(&args).unwrap();
    assert_eq!(response["manifest"]["schema"], "chariox.app.v2");
    assert_eq!(
        response["manifest"]["runtime"],
        serde_json::json!({"engine":"wasm","component":"runtime/app.wasm"})
    );
    let written: crate::v2::ManifestV2 =
        serde_json::from_slice(&disk::read(output).unwrap()).unwrap();
    written.validate(&Limits::default()).unwrap();
    assert_eq!(
        written.required_features,
        vec![crate::v2::RequiredFeature::WasmComponent]
    );
}
