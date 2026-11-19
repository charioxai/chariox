//! Move to Trash on a filesystem of its own, through the peer request `TrashMachineFiles` /
//! `RestoreMachineTrash` (the same kernel request the Files client and the agent tool send).
//!
//! The filesystem is a tmpfs the test mounts on a folder inside its own temp directory, in a mount
//! namespace of a child process: the mount lives and dies with that child, so nothing outside the temp
//! root is read, made or removed, and the parent's cleanup finds an empty mount point. Mounting needs
//! CAP_SYS_ADMIN; without it the test says so and stops.
use std::os::unix::fs::{symlink, MetadataExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};

use super::{serve, Machine, StateGuard};
use crate::local::TrashedKernelFile;
use crate::test_temp_dir::TestTempDir;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const CHILD_ROOT: &str = "CHARIOX_TRASH_VOLUME_ROOT";
const CHILD_TEST: &str = "machine_files::trash_volume_tests::cases_in_the_mount_namespace";

#[test]
fn an_item_on_another_filesystem_never_goes_through_a_link_or_into_a_folder_of_someone_else() {
    let dir = TestTempDir::new("trash-volume");
    let exe = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(exe);
    child
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ROOT, dir.path())
        .env("HOME", dir.join("home"))
        .env("XDG_DATA_HOME", dir.join("home/.local/share"));
    // SAFETY: only async-signal-safe system calls run between fork and exec.
    unsafe {
        child.pre_exec(|| {
            if libc::unshare(libc::CLONE_NEWNS) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let none = std::ptr::null();
            if libc::mount(
                none,
                c"/".as_ptr(),
                none,
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null(),
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = match child.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("SKIPPED: this runner may not make a mount namespace (unshare: {error}), so the test cannot own a filesystem");
            return;
        }
        Err(error) => panic!("the child could not start: {error}"),
    };
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The cases themselves; they run only in the child of the test above, inside its mount namespace.
#[test]
fn cases_in_the_mount_namespace() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(root.join("home/.local/share")).unwrap();
    let uid = unsafe { libc::geteuid() };
    let own_name = format!(".Trash-{uid}");

    // 1. `$topdir/.Trash-$uid` is a link to a folder somebody else controls (on the same filesystem, so a
    // move through the link would succeed).
    let volume = mount(&root, "volume1");
    let attacker = folder(&volume, "attacker1");
    symlink(&attacker, volume.join(&own_name)).unwrap();
    let doc = document(&volume, "one.txt");
    let said = trash(&doc);
    assert!(
        refused(&said).contains("Nothing was moved"),
        "a link as the volume's trash must refuse: {said:?}"
    );
    assert!(doc.exists(), "the item stays where it was");
    assert!(is_empty(&attacker), "nothing may land behind the link");

    // 2. `$topdir/.Trash` is a real sticky folder whose `$uid` entry is a link: it is passed over and the
    // next allowed place (`.Trash-$uid`) takes the item.
    let volume = mount(&root, "volume2");
    let attacker = folder(&volume, "attacker2");
    let shared = volume.join(".Trash");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
    symlink(&attacker, shared.join(uid.to_string())).unwrap();
    let doc = document(&volume, "two.txt");
    let taken = trashed(&trash(&doc));
    assert!(
        Path::new(&taken.trash_path).starts_with(volume.join(&own_name).join("files")),
        "the next allowed place takes it: {taken:?}"
    );
    assert!(is_empty(&attacker), "nothing may land behind the link");
    assert!(!doc.exists());

    // 3. `$topdir/.Trash-$uid` belongs to someone else.
    let volume = mount(&root, "volume3");
    let foreign = volume.join(&own_name);
    for part in ["", "files", "info"] {
        let path = foreign.join(part);
        std::fs::create_dir_all(&path).unwrap();
        std::os::unix::fs::chown(&path, Some(uid + 4242), None).unwrap();
    }
    let doc = document(&volume, "three.txt");
    let said = trash(&doc);
    assert!(
        refused(&said).contains("Nothing was moved"),
        "a folder of someone else must refuse: {said:?}"
    );
    assert!(doc.exists());
    assert!(is_empty(&foreign.join("files")) && is_empty(&foreign.join("info")));

    // 4. A Trash of the user's own that is already there keeps what it holds, through a move and Undo.
    let volume = mount(&root, "volume4");
    let own = volume.join(&own_name);
    for part in ["files", "info"] {
        std::fs::create_dir_all(own.join(part)).unwrap();
        std::fs::set_permissions(own.join(part), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(own.join("files/earlier.txt"), "earlier").unwrap();
    std::fs::write(
        own.join("info/earlier.txt.trashinfo"),
        "[Trash Info]\nPath=work/earlier.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .unwrap();
    let doc = document(&volume, "four.txt");
    let taken = trashed(&trash(&doc));
    assert!(Path::new(&taken.trash_path).starts_with(own.join("files")));
    assert!(!doc.exists());
    let back = restore(&taken);
    assert!(
        matches!(back, RelayPeerResponse::MachineTrashRestored { ref failed, .. } if failed.is_empty()),
        "{back:?}"
    );
    assert!(doc.exists());
    assert_eq!(
        std::fs::read_to_string(own.join("files/earlier.txt")).unwrap(),
        "earlier"
    );
    assert!(own.join("info/earlier.txt.trashinfo").exists());
}

/// A new tmpfs on `<root>/<name>`, a filesystem other than the one `root` is on.
fn mount(root: &Path, name: &str) -> PathBuf {
    let target = root.join(name);
    std::fs::create_dir_all(&target).unwrap();
    let target_c = std::ffi::CString::new(target.to_str().unwrap()).unwrap();
    // SAFETY: valid NUL-terminated strings that outlive the call.
    let status = unsafe {
        libc::mount(
            c"tmpfs".as_ptr(),
            target_c.as_ptr(),
            c"tmpfs".as_ptr(),
            0,
            c"size=8m".as_ptr().cast(),
        )
    };
    assert_eq!(status, 0, "mount: {}", std::io::Error::last_os_error());
    let own = std::fs::metadata(&target).unwrap().dev();
    assert_ne!(
        own,
        std::fs::metadata(root).unwrap().dev(),
        "the case needs a filesystem of its own"
    );
    target
}

fn folder(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn document(volume: &Path, name: &str) -> PathBuf {
    let path = volume.join("work").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "keep me").unwrap();
    path
}

fn is_empty(folder: &Path) -> bool {
    std::fs::read_dir(folder).unwrap().next().is_none()
}

fn ask(request: RelayPeerRequest) -> RelayPeerResponse {
    serve(&Machine::new(StateGuard::default()), request)
}

fn trash(path: &Path) -> RelayPeerResponse {
    ask(RelayPeerRequest::TrashMachineFiles {
        paths: vec![path.display().to_string()],
    })
}

fn restore(item: &TrashedKernelFile) -> RelayPeerResponse {
    ask(RelayPeerRequest::RestoreMachineTrash {
        items: vec![item.clone()],
    })
}

fn trashed(answer: &RelayPeerResponse) -> TrashedKernelFile {
    match answer {
        RelayPeerResponse::MachineFilesTrashed { trashed, failed } if failed.is_empty() => {
            trashed[0].clone()
        }
        other => panic!("the item should have moved to the Trash: {other:?}"),
    }
}

fn refused(answer: &RelayPeerResponse) -> String {
    match answer {
        RelayPeerResponse::FileRefused { reason } => reason.clone(),
        other => panic!("the request should have been refused: {other:?}"),
    }
}
