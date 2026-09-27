use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{read_admitted_data_volume, AdmittedDataVolumeIdentity};

const BOOT_ID: &str = "01234567-89ab-cdef-0123-456789abcdef";
const SERIAL: &str = "12345";
const SIZE_GB: u32 = 50;
// Synthetic mountinfo for parser coverage; it is not evidence of a live mount.
const MOUNTINFO_FIXTURE: &str =
    "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[test]
#[ignore = "Linux root-only; run explicitly as UID 0 with --ignored --show-output"]
fn read_admitted_data_volume_checks_boot_identity_serial_size_and_bounded_json() {
    if !require_root_owned_fixtures() {
        return;
    }
    let fixture = Fixture::new();
    assert_eq!(
        read(&fixture, BOOT_ID, SERIAL, SIZE_GB).expect("valid filesystem fixture is admitted"),
        AdmittedDataVolumeIdentity {
            serial: SERIAL.to_string(),
            size_gb: SIZE_GB,
        },
    );

    assert!(read(&fixture, "fedcba98-7654-3210-fedc-ba9876543210", SERIAL, SIZE_GB).is_err());
    assert!(read(&fixture, BOOT_ID, "54321", SIZE_GB).is_err());
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB + 1).is_err());

    let mut unknown_key = observation_json();
    unknown_key
        .as_object_mut()
        .expect("fixture observation is an object")
        .insert("unexpected".to_string(), serde_json::Value::Bool(true));
    fixture.write_observation(&serde_json::to_vec(&unknown_key).expect("serialize fixture"));
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let mut oversized = serde_json::to_vec(&observation_json()).expect("serialize fixture");
    oversized.resize(4097, b' ');
    fixture.write_observation(&oversized);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());
}

#[test]
#[ignore = "Linux root-only; run explicitly as UID 0 with --ignored --show-output"]
fn read_admitted_data_volume_matches_the_current_mountinfo_fixture() {
    if !require_root_owned_fixtures() {
        return;
    }
    let fixture = Fixture::new();
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_ok());

    for invalid_mountinfo in [
        "",
        "36 25 8:17 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n",
        "36 25 8:16 / /var/lib/chariox-docker/other rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n",
        "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - ext4 /dev/sdb rw,prjquota\n",
        "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2\n",
        concat!(
            "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n",
            "37 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n",
        ),
    ] {
        fixture.write_mountinfo(invalid_mountinfo);
        assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());
    }
}

#[test]
#[ignore = "Linux root-only; run explicitly as UID 0 with --ignored --show-output"]
fn read_admitted_data_volume_rejects_insecure_or_symlinked_files_and_directories() {
    if !require_root_owned_fixtures() {
        return;
    }
    let fixture = Fixture::new();
    set_mode(&fixture.observation_path, 0o600);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    let symlink_target = fixture.root.join("observation-target.fixture");
    fs::copy(&fixture.observation_path, &symlink_target).expect("copy fixture observation");
    fs::remove_file(&fixture.observation_path).expect("remove fixture observation");
    symlink(&symlink_target, &fixture.observation_path).expect("create fixture symlink");
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    set_mode(&fixture.observation_dir, 0o700);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    let real_data_dir = fixture.root.join("data-real");
    fs::rename(&fixture.observation_dir, &real_data_dir).expect("move fixture data directory");
    symlink(&real_data_dir, &fixture.observation_dir).expect("symlink fixture data directory");
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    set_mode(&fixture.runtime_dir, 0o775);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    let real_runtime_dir = fixture.root.join("runtime-real");
    fs::rename(&fixture.runtime_dir, &real_runtime_dir).expect("move fixture runtime directory");
    symlink(&real_runtime_dir, &fixture.runtime_dir).expect("symlink fixture runtime directory");
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    set_owner(&fixture.observation_path, 1, 1);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    set_owner(&fixture.observation_dir, 1, 1);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());

    let fixture = Fixture::new();
    set_owner(&fixture.runtime_dir, 1, 1);
    assert!(read(&fixture, BOOT_ID, SERIAL, SIZE_GB).is_err());
}

fn read(
    fixture: &Fixture,
    boot_id: &str,
    serial: &str,
    size_gb: u32,
) -> Result<AdmittedDataVolumeIdentity, crate::error::DaemonError> {
    read_admitted_data_volume(
        &fixture.observation_path,
        &fixture.mountinfo_path,
        boot_id,
        serial,
        size_gb,
    )
}

fn require_root_owned_fixtures() -> bool {
    if unsafe { libc::geteuid() } == 0 {
        return true;
    }
    eprintln!(
        "UNAVAILABLE: admitted data-volume security cases require Linux UID 0 to create and validate root-owned fixtures; no security coverage was exercised"
    );
    false
}

fn observation_json() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "linuxBootId": BOOT_ID,
        "dataVolumeSerial": SERIAL,
        "dataVolumeSizeGb": SIZE_GB,
        "filesystemUuid": "12345678-1234-1234-1234-123456789abc",
        "devicePath": "/dev/sdb",
        "majorMinor": "8:16",
        "mountTarget": "/var/lib/chariox-docker/data",
    })
}

struct Fixture {
    root: PathBuf,
    runtime_dir: PathBuf,
    observation_dir: PathBuf,
    observation_path: PathBuf,
    mountinfo_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "chariox-volume-observation-{}-{sequence}",
            std::process::id(),
        ));
        let runtime_dir = root.join("runtime");
        let observation_dir = runtime_dir.join("data");
        fs::create_dir_all(&observation_dir).expect("create observation fixture directories");
        let observation_path = observation_dir.join("admission.fixture.json");
        let mountinfo_path = root.join("mountinfo.fixture");
        let fixture = Self {
            root,
            runtime_dir,
            observation_dir,
            observation_path,
            mountinfo_path,
        };
        set_owner(&fixture.runtime_dir, 0, 0);
        set_owner(&fixture.observation_dir, 0, 0);
        set_mode(&fixture.runtime_dir, 0o755);
        set_mode(&fixture.observation_dir, 0o755);
        fixture.write_observation(
            &serde_json::to_vec(&observation_json()).expect("serialize fixture"),
        );
        fixture.write_mountinfo(MOUNTINFO_FIXTURE);
        fixture
    }

    fn write_observation(&self, bytes: &[u8]) {
        fs::write(&self.observation_path, bytes).expect("write observation fixture");
        set_owner(&self.observation_path, 0, 0);
        set_mode(&self.observation_path, 0o644);
    }

    fn write_mountinfo(&self, contents: &str) {
        fs::write(&self.mountinfo_path, contents).expect("write mountinfo fixture");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set fixture mode");
}

fn set_owner(path: &Path, uid: u32, gid: u32) {
    let path = CString::new(path.as_os_str().as_bytes()).expect("fixture path has no NUL byte");
    let result = unsafe { libc::chown(path.as_ptr(), uid, gid) };
    assert_eq!(result, 0, "set fixture owner: {}", io::Error::last_os_error());
}
