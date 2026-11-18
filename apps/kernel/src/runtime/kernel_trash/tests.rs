//! Move to Trash and Undo through the kernel requests, on a real filesystem.
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use super::{restore, trash};
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, RestoreKernelTrashRequest, TrashKernelFilesRequest, TrashedKernelFile,
};
use crate::machine_files::StateGuard;
use crate::test_temp_dir::TestTempDir;

struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl Env {
    fn set(vars: &[(&'static str, &Path)]) -> Self {
        Self(
            vars.iter()
                .map(|(name, value)| {
                    let previous = std::env::var_os(name);
                    std::env::set_var(name, value);
                    (*name, previous)
                })
                .collect(),
        )
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (name, previous) in self.0.drain(..) {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

struct Machine {
    home: PathBuf,
    _env: Env,
    _dir: TestTempDir,
    _lock: crate::env_lock::EnvGuard,
}

/// A user whose home trash is `<home>/.local/share/Trash`.
fn machine(label: &str) -> Machine {
    let lock = crate::env_lock::lock();
    let dir = TestTempDir::new(&format!("trash-{label}"));
    let home = dir.join("home");
    std::fs::create_dir_all(home.join("work")).unwrap();
    let share = home.join(".local/share");
    let env = Env::set(&[("HOME", &home), ("XDG_DATA_HOME", &share)]);
    Machine {
        home,
        _env: env,
        _dir: dir,
        _lock: lock,
    }
}

fn move_to_trash(paths: &[&Path]) -> Result<(Vec<TrashedKernelFile>, Vec<String>), DaemonError> {
    move_guarded(paths, &StateGuard::default())
}

fn move_guarded(
    paths: &[&Path],
    guard: &StateGuard,
) -> Result<(Vec<TrashedKernelFile>, Vec<String>), DaemonError> {
    let request = TrashKernelFilesRequest {
        paths: paths.iter().map(|p| p.display().to_string()).collect(),
    };
    match trash(&request, guard)? {
        LocalDaemonResponse::KernelFilesTrashed { trashed, failed } => {
            Ok((trashed, failed.into_iter().map(|f| f.message).collect()))
        }
        other => panic!("unexpected {other:?}"),
    }
}

fn put_back(items: &[TrashedKernelFile]) -> Result<(Vec<String>, Vec<String>), DaemonError> {
    let request = RestoreKernelTrashRequest {
        items: items.to_vec(),
    };
    match restore(&request, &StateGuard::default())? {
        LocalDaemonResponse::KernelTrashRestored { restored, failed } => {
            Ok((restored, failed.into_iter().map(|f| f.message).collect()))
        }
        other => panic!("unexpected {other:?}"),
    }
}

fn say<T>(result: Result<T, DaemonError>) -> (String, String) {
    match result.map(|_| ()).unwrap_err() {
        DaemonError::MachineRequest { code, message } => (code, message),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_file_and_a_folder_go_to_the_freedesktop_trash_and_nothing_is_deleted() {
    let m = machine("layout");
    let file = m.home.join("work/notes.txt");
    let folder = m.home.join("work/project");
    std::fs::write(&file, b"keep me").unwrap();
    std::fs::create_dir_all(folder.join("src")).unwrap();
    std::fs::write(folder.join("src/main.rs"), b"fn main(){}").unwrap();

    let (trashed, failed) = move_to_trash(&[&file, &folder]).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    assert!(!file.exists() && !folder.exists());
    let trash_root = m.home.join(".local/share/Trash");
    assert_eq!(
        std::fs::read(trash_root.join("files/notes.txt")).unwrap(),
        b"keep me"
    );
    assert!(trash_root.join("files/project/src/main.rs").exists());
    let info = std::fs::read_to_string(trash_root.join("info/notes.txt.trashinfo")).unwrap();
    let mut lines = info.lines();
    assert_eq!(lines.next(), Some("[Trash Info]"));
    assert_eq!(
        lines.next(),
        Some(format!("Path={}", file.display()).as_str())
    );
    let date = lines.next().unwrap().strip_prefix("DeletionDate=").unwrap();
    assert!(
        chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S").is_ok(),
        "{date}"
    );
    assert_eq!(trashed[0].path, file.display().to_string());
    assert_eq!(
        trashed[0].trash_path,
        trash_root.join("files/notes.txt").display().to_string()
    );
}

#[test]
fn a_path_with_odd_characters_is_recorded_percent_encoded() {
    let m = machine("encoding");
    let file = m.home.join("work/résumé 100%.txt");
    std::fs::write(&file, b"x").unwrap();
    move_to_trash(&[&file]).unwrap();
    let info = std::fs::read_to_string(
        m.home
            .join(".local/share/Trash/info/résumé 100%.txt.trashinfo"),
    )
    .unwrap();
    assert!(
        info.contains(&format!(
            "Path={}/work/r%C3%A9sum%C3%A9%20100%25.txt",
            m.home.display()
        )),
        "{info}"
    );
}

#[test]
fn a_name_that_is_in_the_trash_already_gets_a_free_name() {
    let m = machine("collide");
    let file = m.home.join("work/a.txt");
    for body in ["first", "second", "third"] {
        std::fs::write(&file, body).unwrap();
        move_to_trash(&[&file]).unwrap();
    }
    let root = m.home.join(".local/share/Trash");
    for (name, body) in [
        ("a.txt", "first"),
        ("a.txt.1", "second"),
        ("a.txt.2", "third"),
    ] {
        assert_eq!(
            std::fs::read_to_string(root.join("files").join(name)).unwrap(),
            body
        );
        assert!(root.join(format!("info/{name}.trashinfo")).exists());
    }
}

#[test]
fn restore_puts_an_item_back_and_clears_its_record() {
    let m = machine("restore");
    let file = m.home.join("work/notes.txt");
    let folder = m.home.join("work/project");
    std::fs::write(&file, b"keep me").unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a"), b"a").unwrap();
    let (trashed, _) = move_to_trash(&[&file, &folder]).unwrap();

    let (restored, failed) = put_back(&trashed).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), 2);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep me");
    assert_eq!(std::fs::read(folder.join("a")).unwrap(), b"a");
    let root = m.home.join(".local/share/Trash");
    assert_eq!(std::fs::read_dir(root.join("files")).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(root.join("info")).unwrap().count(), 0);
}

#[test]
fn restore_refuses_when_the_original_path_is_taken_and_never_overwrites() {
    let m = machine("conflict");
    let file = m.home.join("work/notes.txt");
    std::fs::write(&file, b"old").unwrap();
    let (trashed, _) = move_to_trash(&[&file]).unwrap();
    std::fs::write(&file, b"new work").unwrap();

    let (code, message) = say(put_back(&trashed));

    assert_eq!(code, "file_restore_conflict");
    assert!(message.contains("is taken"), "{message}");
    assert_eq!(std::fs::read(&file).unwrap(), b"new work");
    assert_eq!(
        std::fs::read(&trashed[0].trash_path).unwrap(),
        b"old",
        "the item stays in the Trash"
    );
    // A taken folder is refused the same way, even an empty one.
    let folder = m.home.join("work/dir");
    std::fs::create_dir_all(folder.join("inside")).unwrap();
    let (trashed, _) = move_to_trash(&[&folder]).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    assert_eq!(say(put_back(&trashed)).0, "file_restore_conflict");
    assert!(Path::new(&trashed[0].trash_path).join("inside").exists());
}

#[test]
fn restore_recreates_a_missing_parent_folder() {
    let m = machine("parent");
    let folder = m.home.join("work/deep");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("x.txt");
    std::fs::write(&file, b"x").unwrap();
    let (trashed, _) = move_to_trash(&[&file]).unwrap();
    std::fs::remove_dir(&folder).unwrap();

    put_back(&trashed).unwrap();

    assert_eq!(std::fs::read(&file).unwrap(), b"x");
}

#[test]
fn restore_only_takes_items_that_are_in_the_trash_as_recorded() {
    let m = machine("forged");
    let loose = m.home.join("work/loose.txt");
    std::fs::write(&loose, b"x").unwrap();
    let target = m.home.join("work/elsewhere.txt");
    let forged = |trash_path: &Path| TrashedKernelFile {
        path: target.display().to_string(),
        trash_path: trash_path.display().to_string(),
    };
    assert_eq!(say(put_back(&[forged(&loose)])).0, "file_not_in_trash");
    let file = m.home.join("work/real.txt");
    std::fs::write(&file, b"x").unwrap();
    let (trashed, _) = move_to_trash(&[&file]).unwrap();
    let (code, message) = say(put_back(&[forged(Path::new(&trashed[0].trash_path))]));
    assert_eq!(code, "file_not_in_trash");
    assert!(message.contains("keeps"), "{message}");
    assert!(!target.exists());
}

#[test]
fn one_failure_does_not_stop_the_others_and_is_reported() {
    let m = machine("partial");
    let file = m.home.join("work/here.txt");
    std::fs::write(&file, b"x").unwrap();
    let missing = m.home.join("work/gone.txt");

    let (trashed, failed) = move_to_trash(&[&file, &missing]).unwrap();

    assert_eq!(trashed.len(), 1);
    assert_eq!(failed.len(), 1);
    assert!(failed[0].contains("does not exist"), "{failed:?}");
}

#[test]
fn nothing_moved_is_an_error_with_the_reason() {
    let m = machine("none");
    let missing = m.home.join("work/gone.txt");
    let (code, message) = say(move_to_trash(&[&missing]));
    assert_eq!(code, "file_not_found");
    assert!(message.contains("gone.txt"), "{message}");
    assert_eq!(say(move_to_trash(&[])).0, "file_path_invalid");
}

#[test]
fn the_home_folder_the_trash_and_the_kernels_state_are_not_moved() {
    let m = machine("refused");
    let file = m.home.join("work/a.txt");
    std::fs::write(&file, b"x").unwrap();
    let (trashed, _) = move_to_trash(&[&file]).unwrap();
    assert_eq!(say(move_to_trash(&[&m.home])).0, "file_path_invalid");
    assert_eq!(say(move_to_trash(&[Path::new("/")])).0, "file_path_invalid");
    let already = say(move_to_trash(&[Path::new(&trashed[0].trash_path)]));
    assert!(already.1.contains("in the Trash already"), "{already:?}");

    let state = m.home.join("work/kernel-home");
    std::fs::create_dir_all(state.join("state")).unwrap();
    let _chariox = Env::set(&[("CHARIOX_HOME", &state)]);
    let guard = StateGuard::new(None);
    for path in [&state, &state.join("state"), &m.home.join("work")] {
        assert_eq!(
            say(move_guarded(&[path], &guard)).0,
            "file_state_protected",
            "{}",
            path.display()
        );
    }
}

#[test]
fn naming_a_folder_and_something_in_it_gives_each_its_own_result_and_undoes_both() {
    let m = machine("nested");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    let inner = folder.join("a.txt");
    std::fs::write(&inner, b"x").unwrap();

    let (trashed, failed) = move_to_trash(&[&inner, &folder, &folder]).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    let by_path = |path: &Path| {
        trashed
            .iter()
            .find(|item| item.path == path.display().to_string())
    };
    let (outer, nested) = (by_path(&folder).unwrap(), by_path(&inner).unwrap());
    assert_eq!(trashed.len(), 2, "the folder named twice is one result");
    assert_eq!(
        nested.trash_path,
        Path::new(&outer.trash_path)
            .join("a.txt")
            .display()
            .to_string(),
        "the inner item is reported where it now is, inside its folder"
    );
    let (restored, failed) = put_back(&trashed).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), 2);
    assert!(inner.exists());
}

#[test]
fn a_refused_folder_does_not_hide_what_was_named_inside_it() {
    let m = machine("refused-parent");
    let state = m.home.join("work/kernel-home");
    std::fs::create_dir_all(state.join("state")).unwrap();
    let kept = m.home.join("work/a.txt");
    std::fs::write(&kept, b"x").unwrap();
    let inside_state = state.join("state/keep.txt");
    std::fs::write(&inside_state, b"secret").unwrap();
    let _chariox = Env::set(&[("CHARIOX_HOME", &state)]);
    let guard = StateGuard::new(None);

    // `work` holds the kernel's state, so it is refused; `a.txt` inside it still moves, and the file
    // inside the state is refused with its own reason.
    let (trashed, failed) =
        move_guarded(&[&m.home.join("work"), &kept, &inside_state], &guard).unwrap();

    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].path, kept.display().to_string());
    assert_eq!(
        failed.len(),
        2,
        "every requested path ends with a result: {failed:?}"
    );
    assert!(
        failed
            .iter()
            .all(|reason| reason.contains("kernel's own state")),
        "{failed:?}"
    );
    assert!(inside_state.exists());
}

/// A temp topdir that a test treats as another filesystem than the rest of the temp root, so the
/// per-filesystem Trash is exercised without touching any real mount.
struct Volume {
    _dir: TestTempDir,
    top: PathBuf,
    home_trash: PathBuf,
}

fn volume(label: &str) -> Volume {
    let dir = TestTempDir::new(&format!("trash-vol-{label}"));
    let top = dir.join("vol");
    std::fs::create_dir_all(top.join("sub")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let home_trash = dir.join("home/share/Trash");
    Volume {
        _dir: dir,
        top,
        home_trash,
    }
}

fn on_volume(v: &Volume, uid: u32) -> super::platform::Trash {
    let top = v.top.clone();
    super::platform::Trash::on_devices(
        v.home_trash.clone(),
        uid,
        Box::new(move |path| {
            if path.starts_with(&top) {
                Ok(7_777)
            } else {
                std::fs::metadata(path).map(|metadata| metadata.dev())
            }
        }),
    )
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn trash_on(trash: &super::platform::Trash, path: &Path) -> Result<TrashedKernelFile, DaemonError> {
    let request = TrashKernelFilesRequest {
        paths: vec![path.display().to_string()],
    };
    match super::trash_with(trash, &request, &StateGuard::default())? {
        LocalDaemonResponse::KernelFilesTrashed { mut trashed, .. } => Ok(trashed.remove(0)),
        other => panic!("unexpected {other:?}"),
    }
}

fn restore_on(trash: &super::platform::Trash, item: &TrashedKernelFile) -> Result<(), DaemonError> {
    let request = RestoreKernelTrashRequest {
        items: vec![item.clone()],
    };
    super::restore_with(trash, &request, &StateGuard::default()).map(drop)
}

#[test]
fn an_item_on_another_filesystem_uses_that_filesystems_trash_and_restores() {
    let v = volume("own");
    let trash = on_volume(&v, euid());
    let file = v.top.join("sub/far.txt");
    std::fs::write(&file, b"far").unwrap();

    let item = trash_on(&trash, &file).unwrap();

    let own = v.top.join(format!(".Trash-{}", euid()));
    assert_eq!(
        item.trash_path,
        own.join("files/far.txt").display().to_string()
    );
    let info = std::fs::read_to_string(own.join("info/far.txt.trashinfo")).unwrap();
    assert!(
        info.contains("Path=sub/far.txt\n"),
        "relative to the mount point: {info}"
    );
    assert_eq!(std::fs::metadata(&own).unwrap().mode() & 0o777, 0o700);
    assert!(!v.home_trash.join("files/far.txt").exists());
    restore_on(&trash, &item).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"far");
}

#[test]
fn the_shared_trash_folder_is_used_only_when_it_is_a_real_sticky_folder() {
    let v = volume("shared");
    let trash = on_volume(&v, euid());
    let file = v.top.join("sub/a.txt");
    let shared = v.top.join(".Trash");
    let own = v.top.join(format!(".Trash-{}", euid()));

    // Not sticky: passed over for `.Trash-$uid`.
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::write(&file, b"1").unwrap();
    assert!(trash_on(&trash, &file)
        .unwrap()
        .trash_path
        .starts_with(own.to_str().unwrap()));
    assert_eq!(
        std::fs::read_dir(&shared).unwrap().count(),
        0,
        "nothing was put in an open folder"
    );

    // A real sticky folder: `.Trash/$uid`, private to the user.
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
    std::fs::write(&file, b"2").unwrap();
    let item = trash_on(&trash, &file).unwrap();
    let mine = shared.join(euid().to_string());
    assert!(
        item.trash_path.starts_with(mine.to_str().unwrap()),
        "{item:?}"
    );
    assert_eq!(std::fs::metadata(&mine).unwrap().mode() & 0o777, 0o700);
    restore_on(&trash, &item).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"2");

    // A link in place of `.Trash` is never followed.
    std::fs::remove_dir_all(&shared).unwrap();
    let elsewhere = v.top.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &shared).unwrap();
    assert!(trash_on(&trash, &file)
        .unwrap()
        .trash_path
        .starts_with(own.to_str().unwrap()));
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "the link's target was left alone"
    );
}

#[test]
fn a_trash_folder_of_another_owner_or_behind_a_link_is_refused_and_nothing_moves() {
    let v = volume("hostile");
    let me = euid();
    let file = v.top.join("sub/a.txt");
    std::fs::write(&file, b"keep").unwrap();

    // `.Trash-$uid` made by someone else (a different uid than the one this Trash acts as).
    let other = on_volume(&v, me + 1);
    let theirs = v.top.join(format!(".Trash-{}", me + 1));
    std::fs::create_dir(&theirs).unwrap();
    let (code, message) = say(trash_on(&other, &file));
    assert_eq!(code, "file_trash_failed");
    assert!(
        message.contains("belongs to someone else") && message.contains("Nothing was moved"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_dir(&theirs).unwrap().count(),
        0,
        "nothing was put in their folder"
    );

    // `.Trash-$uid` is a link to a folder someone else controls.
    let trash = on_volume(&v, me);
    let attacker = v.top.join("attacker");
    std::fs::create_dir(&attacker).unwrap();
    std::os::unix::fs::symlink(&attacker, v.top.join(format!(".Trash-{me}"))).unwrap();
    let (code, message) = say(trash_on(&trash, &file));
    assert_eq!(code, "file_trash_failed");
    assert!(
        message.contains("link") && message.contains("Nothing was moved"),
        "{message}"
    );
    assert_eq!(std::fs::read_dir(&attacker).unwrap().count(), 0);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
}

#[test]
fn a_home_trash_that_is_a_link_is_passed_over_and_never_followed() {
    let v = volume("homelink");
    let trash = on_volume(&v, euid());
    let attacker = v.top.join("attacker");
    std::fs::create_dir(&attacker).unwrap();
    std::fs::create_dir_all(v.home_trash.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&attacker, &v.home_trash).unwrap();
    let file = v.top.join("sub/a.txt");
    std::fs::write(&file, b"x").unwrap();

    // On the volume the item takes the volume's own Trash; the link is only a problem report.
    let item = trash_on(&trash, &file).unwrap();

    assert!(item.trash_path.starts_with(v.top.to_str().unwrap()));
    assert_eq!(std::fs::read_dir(&attacker).unwrap().count(), 0);
}

#[test]
fn a_restore_through_a_swapped_trash_folder_or_record_moves_nothing() {
    let m = machine("swap");
    let file = m.home.join("work/a.txt");
    std::fs::write(&file, b"keep").unwrap();
    let (trashed, _) = move_to_trash(&[&file]).unwrap();
    let root = m.home.join(".local/share/Trash");

    // The record is a link: it is not read, and the item stays.
    let record = root.join("info/a.txt.trashinfo");
    let real = m.home.join("work/record-copy");
    std::fs::rename(&record, &real).unwrap();
    std::os::unix::fs::symlink(&real, &record).unwrap();
    assert_eq!(say(put_back(&trashed)).0, "file_not_in_trash");
    std::fs::remove_file(&record).unwrap();
    std::fs::rename(&real, &record).unwrap();

    // `files` is a link to another folder: it is not entered.
    let held = m.home.join("work/held-files");
    std::fs::rename(root.join("files"), &held).unwrap();
    std::os::unix::fs::symlink(&held, root.join("files")).unwrap();
    assert_eq!(say(put_back(&trashed)).0, "file_not_in_trash");
    assert!(!file.exists() && held.join("a.txt").exists());

    // The Trash folder itself is a link: it is not used.
    std::fs::remove_file(root.join("files")).unwrap();
    std::fs::rename(&held, root.join("files")).unwrap();
    let moved = m.home.join("work/Trash-moved");
    std::fs::rename(&root, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &root).unwrap();
    assert_eq!(say(put_back(&trashed)).0, "file_not_in_trash");
    assert!(!file.exists());
}
