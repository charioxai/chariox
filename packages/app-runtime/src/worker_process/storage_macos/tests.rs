use super::*;
use serde_json::json;
use std::{
    ffi::CString,
    fs,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
};

mod hosted;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let name = std::env::temp_dir().join("chariox-app-storage-XXXXXX");
        let mut bytes = CString::new(name.as_os_str().as_bytes())
            .unwrap()
            .into_bytes_with_nul();
        assert!(!unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) }.is_null());
        let path = PathBuf::from(
            unsafe { std::ffi::CStr::from_ptr(bytes.as_ptr().cast()) }
                .to_str()
                .unwrap(),
        );
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Pure fixtures contain no mounts. The hosted fixture explicitly detaches
        // and verifies before allowing this recursive test-only cleanup.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(dir: &Dir) -> Journal {
    let data = private_mount(dir, "data").unwrap();
    let tmp = private_mount(dir, "tmp").unwrap();
    Journal {
        schema: "chariox.app-storage.v1".into(),
        owner: "owner".into(),
        installation: "app".into(),
        generation: 1,
        pending_recovery: true,
        images: [
            journal::image("data", 64 * 1024 * 1024, FileIdentity::of(&data.0).unwrap()),
            journal::image("tmp", 64 * 1024 * 1024, FileIdentity::of(&tmp.0).unwrap()),
        ],
    }
}

#[test]
fn command_plan_uses_fixed_capacity_private_owner_and_no_overwrite_or_sparse_options() {
    let args = volume::create_arguments(
        Path::new("/private/owned/data.dmg"),
        "cx-data-nonce",
        64 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        &args[..11],
        [
            "create",
            "-size",
            "67108864b",
            "-type",
            "UDIF",
            "-layout",
            "NONE",
            "-fs",
            "APFS",
            "-volname",
            "cx-data-nonce"
        ]
    );
    assert!(args.windows(2).any(|v| v == ["-mode", "0700"]));
    assert!(!args
        .iter()
        .any(|v| matches!(v.as_str(), "-ov" | "SPARSE" | "SPARSEBUNDLE" | "-force")));
    assert_eq!(args.last().unwrap(), "/private/owned/data.dmg");
}

#[test]
fn only_exact_image_entities_and_volume_identity_can_authorize_device_actions() {
    // Representative Apple plist-to-JSON shape; actual hosted observations are
    // kept separately as evidence, and are not inferred from this fixture.
    let image = Path::new("/private/owned/data.dmg");
    let info = json!({"images":[{"image-path":"/private/other/data.dmg","system-entities":[{"dev-entry":"/dev/disk7"}]},
        {"image-path":image,"system-entities":[{"dev-entry":"/dev/disk8"},{"dev-entry":"/dev/disk9"},{"dev-entry":"/dev/disk9s1"}]}]});
    assert_eq!(
        identity::image_entities(&info, image).unwrap().unwrap(),
        ["/dev/disk8", "/dev/disk9", "/dev/disk9s1"]
    );
    assert!(
        identity::image_entities(&info, Path::new("/private/owned/absent.dmg"))
            .unwrap()
            .is_none()
    );
    let mut duplicated = info.clone();
    duplicated["images"]
        .as_array_mut()
        .unwrap()
        .push(info["images"][1].clone());
    assert_eq!(
        identity::image_entities(&duplicated, image),
        Err(Error::Identity)
    );
    for node in [
        "/dev/disk0",
        "/dev/disk01",
        "/dev/disk8/../disk1",
        "/dev/rdisk8",
        "/dev/disk8s0",
        "/dev/disk8s1s2s3",
    ] {
        assert!(identity::device(node).is_err(), "{node}");
    }
    let volume = json!({"FilesystemType":"apfs","VolumeName":"expected","DeviceNode":"/dev/disk9s1",
        "VolumeUUID":"12345678-1234-1234-1234-123456789abc"});
    assert_eq!(
        identity::volume_info(&volume, "expected").unwrap(),
        Some((
            "/dev/disk9s1".into(),
            "12345678-1234-1234-1234-123456789ABC".into()
        ))
    );
    assert_eq!(
        identity::volume_info(&volume, "other"),
        Err(Error::Identity)
    );
    let whole = json!({"WholeDisk":true,"VirtualOrPhysical":"Virtual","DeviceNode":"/dev/disk8"});
    assert_eq!(identity::whole_info(&whole, "/dev/disk8"), Ok(()));
    assert_eq!(
        identity::whole_info(&whole, "/dev/disk9"),
        Err(Error::Identity)
    );
}

#[test]
fn journal_reopens_with_same_identity_and_rejects_unsafe_capacity_and_paths() {
    let scratch = Scratch::new();
    let dir = Dir::open_private(&scratch.0).unwrap();
    let journal = record(&dir);
    journal.save(&dir).unwrap();
    let saved = journal::load(&dir).unwrap().unwrap();
    assert_eq!(saved.images[0].image, journal.images[0].image);
    assert_eq!(
        saved.images[0].mount_identity,
        journal.images[0].mount_identity
    );
    assert!(saved.pending_recovery);
    let mut bad = saved.clone();
    bad.images[0].image = "../../other.dmg".into();
    assert!(bad.save(&dir).is_err());
    let mut bad = saved.clone();
    bad.images[1].capacity = CAPACITIES[0];
    assert!(bad.save(&dir).is_err());
    let mut bad = saved;
    bad.generation = 0;
    assert!(bad.save(&dir).is_err());
    assert!(journal::load(&dir).unwrap().unwrap().pending_recovery);
}

#[test]
fn held_file_identity_and_private_root_reject_replacement_and_aliases() {
    let scratch = Scratch::new();
    let dir = Dir::open_private(&scratch.0).unwrap();
    let file = dir.create_private_file(OsStr::new("image.dmg")).unwrap();
    let identity = FileIdentity::of(&file).unwrap();
    fs::rename(scratch.0.join("image.dmg"), scratch.0.join("moved.dmg")).unwrap();
    let _replacement = dir.create_private_file(OsStr::new("image.dmg")).unwrap();
    assert_eq!(
        identity.require(&dir, OsStr::new("image.dmg"), &file),
        Err(Error::Identity)
    );
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(StorageRoot::open(&scratch.0).is_err());
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn accounting_reserves_full_unallocated_image_capacity_and_host_headroom() {
    let reserved = 132 * 1024 * 1024;
    assert_eq!(
        require_capacity(MAX_RESERVED_BYTES, reserved, HOST_RESERVE + reserved),
        Ok(())
    );
    assert_eq!(
        require_capacity(MAX_RESERVED_BYTES + 1, reserved, HOST_RESERVE + reserved),
        Err(Error::Capacity)
    );
    assert_eq!(
        require_capacity(reserved, reserved, HOST_RESERVE + reserved - 1),
        Err(Error::Capacity)
    );
    assert_eq!(
        require_capacity(reserved, reserved, 0),
        Err(Error::Capacity)
    );
}

#[test]
fn interrupted_atomic_metadata_is_removed_only_after_its_owner_releases_the_lock() {
    let scratch = Scratch::new();
    let dir = Dir::open_private(&scratch.0).unwrap();
    let name = OsStr::new(".replace-123-0-abcd");
    let file = dir.create_private_file(name).unwrap();
    assert!(crate::private_fs::try_lock_file(&file).unwrap());
    assert_eq!(journal::recover_temporaries(&dir), Err(Error::Busy));
    drop(file);
    journal::recover_temporaries(&dir).unwrap();
    assert!(dir.entries(1).unwrap().is_empty());
}

#[test]
fn drop_preserves_recovery_after_an_explicit_cleanup_attempt() {
    let scratch = Scratch::new();
    let dir = Dir::open_private(&scratch.0).unwrap();
    let journal = record(&dir);
    journal.save(&dir).unwrap();
    // No image exists: this remains an ordinary metadata test even if Drop is
    // accidentally changed to retry. Such a retry would clear the pending bit.
    drop(MountedStorage {
        root: dir,
        path: scratch.0.clone(),
        journal,
        images: [None, None],
        mounted: [None, None],
        released: false,
        cleanup_attempted: true,
        deadline: std::time::Instant::now(),
    });
    let dir = Dir::open_private(&scratch.0).unwrap();
    assert!(journal::load(&dir).unwrap().unwrap().pending_recovery);
}
