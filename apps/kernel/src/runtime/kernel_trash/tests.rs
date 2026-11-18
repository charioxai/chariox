//! Move to Trash and Undo through the kernel requests, on a real filesystem.
use std::os::unix::fs::MetadataExt as _;
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
fn naming_a_folder_and_something_in_it_moves_the_folder_once() {
    let m = machine("nested");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    let inner = folder.join("a.txt");
    std::fs::write(&inner, b"x").unwrap();

    let (trashed, failed) = move_to_trash(&[&folder, &inner, &folder]).unwrap();

    assert_eq!(trashed.len(), 1);
    assert!(failed.is_empty(), "{failed:?}");
    put_back(&trashed).unwrap();
    assert!(inner.exists());
}

struct Remove(PathBuf);

impl Drop for Remove {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_item_on_another_filesystem_uses_that_filesystems_trash_and_restores() {
    let shm = Path::new("/dev/shm");
    let m = machine("volume");
    let folder = shm.join(format!("chariox-trash-{}", std::process::id()));
    if !shm.is_dir() || std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    let uid = unsafe { libc::geteuid() };
    let per_volume = shm.join(format!(".Trash-{uid}"));
    let _cleanup = (Remove(folder.clone()), Remove(per_volume.clone()));
    if std::fs::metadata(&folder).unwrap().dev() == std::fs::metadata(&m.home).unwrap().dev() {
        return;
    }
    let file = folder.join("far.txt");
    std::fs::write(&file, b"far").unwrap();

    let (trashed, failed) = move_to_trash(&[&file]).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(
        trashed[0].trash_path,
        per_volume.join("files/far.txt").display().to_string()
    );
    let info = std::fs::read_to_string(per_volume.join("info/far.txt.trashinfo")).unwrap();
    assert!(
        info.contains(&format!(
            "Path={}",
            file.strip_prefix(shm).unwrap().display()
        )),
        "the path is relative to the mount point: {info}"
    );
    assert!(!m.home.join(".local/share/Trash/files/far.txt").exists());
    put_back(&trashed).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"far");
}

#[test]
fn an_item_where_no_trash_can_be_made_stays_and_the_reason_says_so() {
    let _m = machine("nodrive");
    // /proc is another filesystem on which no user can make a Trash folder.
    let (code, message) = say(move_to_trash(&[Path::new("/proc/version")]));
    assert_eq!(code, "file_trash_failed");
    assert!(
        message.contains("different drive") && message.contains("Nothing was moved"),
        "{message}"
    );
    assert!(Path::new("/proc/version").exists());
}

#[test]
fn the_macos_trash_knows_its_folders_and_puts_back_only_what_is_in_them() {
    let home = PathBuf::from("/Users/me");
    let trash = super::macos::Trash::new(home.clone(), 501);
    assert!(trash.holds(&home.join(".Trash/a.txt")));
    assert!(trash.holds(Path::new("/Volumes/Data/.Trashes/501/a.txt")));
    assert!(!trash.holds(Path::new("/Volumes/Data/.Trashes/502/a.txt")));
    assert!(!trash.holds(&home.join("Documents/a.txt")));
    let outside = trash.locate(&home.join("Documents/a.txt"), &home.join("a.txt"));
    assert_eq!(say(outside).0, "file_not_in_trash");
}
