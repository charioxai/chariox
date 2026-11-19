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
fn a_dangling_link_inside_a_moved_folder_is_restored_with_the_folder() {
    let m = machine("nested-dangling");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    let link = folder.join("missing-target");
    std::os::unix::fs::symlink("not-present", &link).unwrap();
    let (trashed, failed) = move_to_trash(&[&link, &folder]).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(trashed.len(), 2);

    let (restored, failed) = put_back(&trashed).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), 2);
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("not-present"));
}

/// What `move_to_trash` and `put_back` say about one path: the item that moved, if it did.
fn moved<'a>(trashed: &'a [TrashedKernelFile], path: &Path) -> &'a TrashedKernelFile {
    trashed
        .iter()
        .find(|item| item.path == path.display().to_string())
        .unwrap_or_else(|| panic!("{} was not reported as moved: {trashed:?}", path.display()))
}

#[test]
fn an_item_reported_inside_a_moved_folder_is_restored_on_its_own() {
    let m = machine("nested-alone");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    let (a, b) = (folder.join("a.txt"), folder.join("sub/b.txt"));
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    let (trashed, failed) = move_to_trash(&[&folder, &a, &b]).unwrap();
    assert!(failed.is_empty(), "{failed:?}");

    // The deep item alone: its folders are made again, the rest stays in the Trash.
    let (restored, failed) = put_back(&[moved(&trashed, &b).clone()]).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored, [b.display().to_string()]);
    assert_eq!(std::fs::read(&b).unwrap(), b"b");
    assert!(!a.exists());
    assert!(
        Path::new(&moved(&trashed, &folder).trash_path)
            .join("a.txt")
            .exists(),
        "the folder and its other item are still in the Trash"
    );

    // Its parent path is taken by something new: the item still comes back, nothing is replaced.
    let (restored, failed) = put_back(&[moved(&trashed, &a).clone()]).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored, [a.display().to_string()]);
    assert_eq!(std::fs::read(&a).unwrap(), b"a");

    // The item gone from the Trash, or recorded for another place, is refused.
    let (code, _) = say(put_back(&[moved(&trashed, &a).clone()]));
    assert_eq!(code, "file_not_in_trash");
    let mut elsewhere = moved(&trashed, &b).clone();
    elsewhere.path = m.home.join("work/other/b.txt").display().to_string();
    assert_eq!(say(put_back(&[elsewhere])).0, "file_not_in_trash");

    // The folder itself still restores, and is refused where its place is taken.
    let (code, message) = say(put_back(&[moved(&trashed, &folder).clone()]));
    assert_eq!(code, "file_restore_conflict");
    assert!(message.contains("is taken"), "{message}");
}

#[test]
fn an_item_inside_a_moved_folder_that_does_not_exist_is_a_failure_not_a_result() {
    let m = machine("nested-missing");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), b"x").unwrap();
    let missing = folder.join("missing.txt");

    let (trashed, failed) = move_to_trash(&[&folder, &folder.join("a.txt"), &missing]).unwrap();

    assert_eq!(trashed.len(), 2, "{trashed:?}");
    assert!(trashed
        .iter()
        .all(|item| item.path != missing.display().to_string()));
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].contains("does not exist"), "{failed:?}");
}

#[test]
fn a_path_with_dot_dot_is_refused_and_moves_nothing() {
    let m = machine("dotdot");
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    let notes = m.home.join("work/notes.txt");
    std::fs::write(&notes, b"x").unwrap();
    let up = format!("{}/../notes.txt", folder.display());

    let request = TrashKernelFilesRequest {
        paths: vec![folder.display().to_string(), up.clone()],
    };
    match trash(&request, &StateGuard::default()).unwrap() {
        LocalDaemonResponse::KernelFilesTrashed { trashed, failed } => {
            assert_eq!(trashed.len(), 1, "{trashed:?}");
            assert_eq!(failed.len(), 1, "{failed:?}");
            assert_eq!(failed[0].path, up);
            assert!(failed[0].message.contains("contains .."), "{failed:?}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(notes.exists(), "notes.txt is outside the folder that moved");

    // Alone, it is the request's own error, and so is Undo of such an item.
    assert_eq!(say(move_to_trash(&[Path::new(&up)])).0, "file_path_invalid");
    assert!(notes.exists());
    let item = TrashedKernelFile {
        path: up,
        trash_path: m
            .home
            .join(".local/share/Trash/files/project")
            .display()
            .to_string(),
    };
    assert_eq!(say(put_back(&[item])).0, "file_path_invalid");
}

#[test]
fn a_link_is_moved_as_the_link_and_nothing_behind_it_is_taken() {
    let m = machine("links");
    let outside = m.home.join("work/outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("x.txt"), b"x").unwrap();
    let link = m.home.join("work/link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let dangling = m.home.join("work/dangling");
    std::os::unix::fs::symlink("nowhere", &dangling).unwrap();

    // The link and a path through it: the link moves, the file behind it stays, and the path through
    // it is a failure of its own.
    let through = link.join("x.txt");
    let (trashed, failed) = move_to_trash(&[&link, &dangling, &through]).unwrap();
    assert_eq!(trashed.len(), 2, "{trashed:?}");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(outside.join("x.txt").exists());
    assert!(std::fs::symlink_metadata(&link).is_err());
    let (restored, failed) = put_back(&trashed).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), 2);
    assert_eq!(std::fs::read_link(&link).unwrap(), outside);
    assert_eq!(std::fs::read_link(&dangling).unwrap(), Path::new("nowhere"));

    // A link inside a folder that moved: a path through it is not reported as moved with the folder.
    let folder = m.home.join("work/project");
    std::fs::create_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&outside, folder.join("sub")).unwrap();
    let (trashed, failed) = move_to_trash(&[&folder, &folder.join("sub/x.txt")]).unwrap();
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(outside.join("x.txt").exists());
}

#[test]
fn a_name_up_to_255_bytes_moves_and_comes_back_with_its_exact_name() {
    let m = machine("long-names");
    let work = m.home.join("work");
    let names = [
        "a".repeat(255),
        "b".repeat(250),
        format!("{}-one", "c".repeat(250)),
        format!("{}-two", "c".repeat(250)),
    ];
    for name in &names {
        std::fs::write(work.join(name), name.as_bytes()).unwrap();
    }
    let paths: Vec<PathBuf> = names.iter().map(|name| work.join(name)).collect();
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

    let (trashed, failed) = move_to_trash(&refs).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(trashed.len(), 4);
    let in_trash: std::collections::HashSet<_> =
        trashed.iter().map(|item| item.trash_path.clone()).collect();
    assert_eq!(in_trash.len(), 4, "each item has its own name in the Trash");
    let (restored, failed) = put_back(&trashed).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), 4);
    for (name, path) in names.iter().zip(&paths) {
        assert_eq!(std::fs::read(path).unwrap(), name.as_bytes());
    }
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

fn write_at(path: &Path, text: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The path table of the home trash: the trash folder, `files/`, `info/`, a top-level entry, and items
/// inside an entry with any names. A place is read from its position under the trash folder, never by
/// looking for a folder named `files`.
#[test]
fn every_item_inside_a_moved_folder_restores_alone_whatever_its_folders_are_named() {
    let m = machine("path-table");
    let work = m.home.join("work");
    let root = m.home.join(".local/share/Trash");
    let own_trash = format!(".Trash-{}", euid());
    // (moved folder, item inside it): the item restores on its own while the folder stays in the Trash.
    let rows = [
        ("project", "files/a.txt".to_string()),
        ("project", "info/b.txt".to_string()),
        ("project", "files/files/c.txt".to_string()),
        ("project", ".local/share/Trash/files/d.txt".to_string()),
        ("project", format!("{own_trash}/files/e.txt")),
        ("files", "f.txt".to_string()),
        ("info", "g.txt".to_string()),
        ("ação", "日本語/ü.txt".to_string()),
    ];
    let mut named: Vec<PathBuf> = Vec::new();
    for (folder, inner) in &rows {
        let item = work.join(folder).join(inner);
        write_at(&item, inner.as_bytes());
        if !named.contains(&work.join(folder)) {
            named.push(work.join(folder));
        }
        named.push(item);
    }
    let refs: Vec<&Path> = named.iter().map(PathBuf::as_path).collect();
    let (trashed, failed) = move_to_trash(&refs).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(trashed.len(), named.len());

    for (folder, inner) in &rows {
        let item = work.join(folder).join(inner);
        let reported = moved(&trashed, &item);
        assert_eq!(
            reported.trash_path,
            root.join("files")
                .join(folder)
                .join(inner)
                .display()
                .to_string()
        );
        let (restored, failed) = put_back(std::slice::from_ref(reported)).unwrap();
        assert!(failed.is_empty(), "{inner}: {failed:?}");
        assert_eq!(restored, [item.display().to_string()]);
        assert_eq!(std::fs::read(&item).unwrap(), inner.as_bytes(), "{inner}");
        assert!(
            root.join("info")
                .join(format!("{folder}.trashinfo"))
                .exists(),
            "{inner}: the folder's record stays until the folder itself comes back"
        );
    }

    // A `.` in the reported path names the same item.
    let project = work.join("project");
    write_at(&project.join("files/a.txt"), b"again");
    let (again, _) = move_to_trash(&[&project.join("files/a.txt")]).unwrap();
    let dotted = TrashedKernelFile {
        path: again[0].path.clone(),
        trash_path: again[0].trash_path.replace("/files/", "/files/./"),
    };
    assert_ne!(dotted.trash_path, again[0].trash_path);
    put_back(&[dotted]).unwrap();
    assert_eq!(
        std::fs::read(project.join("files/a.txt")).unwrap(),
        b"again"
    );

    // The trash folder, `files/`, `info/`, a record and a path with `..` are not items.
    let entry = Path::new(&moved(&trashed, &project).trash_path).to_path_buf();
    for not_an_item in [
        root.clone(),
        root.join("files"),
        root.join("info"),
        root.join("info/project.trashinfo"),
        entry.join("files/../files/b.txt"),
    ] {
        let item = TrashedKernelFile {
            path: work.join("x.txt").display().to_string(),
            trash_path: not_an_item.display().to_string(),
        };
        let (code, _) = say(put_back(&[item]));
        assert_eq!(code, "file_not_in_trash", "{}", not_an_item.display());
    }
}

/// Names that are not text are kept as bytes: an item with such a name inside a moved folder restores
/// on its own through the platform trash.
#[test]
fn an_item_whose_name_is_not_text_restores_alone() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;
    let m = machine("bytes");
    let trash = super::platform::Trash::new(m.home.join(".local/share/Trash"), euid());
    let folder = m.home.join("work/raw");
    let inner = Path::new(OsStr::from_bytes(b"caf\xe9")).join("files/h.txt");
    write_at(&folder.join(&inner), b"h");

    let trash_path = trash.move_to(&folder).unwrap();
    let back = trash
        .restore(&trash_path.join(&inner), &folder.join(&inner))
        .unwrap();

    assert_eq!(back, folder.join(&inner));
    assert_eq!(std::fs::read(&back).unwrap(), b"h");
    assert!(trash_path.join(OsStr::from_bytes(b"caf\xe9")).is_dir());
}

/// The same table on another filesystem's `.Trash-$uid`: a folder named like a trash inside a moved
/// folder is part of the item, and only the trash folder at the top of the filesystem counts.
#[test]
fn inside_a_filesystems_own_trash_items_are_read_by_position_too() {
    let v = volume("table");
    let trash = on_volume(&v, euid());
    let own_trash = format!(".Trash-{}", euid());
    let folder = v.top.join("sub/project");
    let inners = [
        format!("{own_trash}/files/x.txt"),
        "files/y.txt".to_string(),
        ".Trash/0/files/z.txt".to_string(),
    ];
    for inner in &inners {
        write_at(&folder.join(inner), inner.as_bytes());
    }
    let item = trash_on(&trash, &folder).unwrap();
    for inner in &inners {
        let nested = TrashedKernelFile {
            path: folder.join(inner).display().to_string(),
            trash_path: Path::new(&item.trash_path)
                .join(inner)
                .display()
                .to_string(),
        };
        restore_on(&trash, &nested).unwrap();
        assert_eq!(std::fs::read(folder.join(inner)).unwrap(), inner.as_bytes());
    }

    // A folder of the user's own named `.Trash` or `.Trash-$uid` below the top is not a trash.
    let lookalike = v.top.join(format!("sub/{own_trash}/notes.txt"));
    write_at(&lookalike, b"n");
    let shared_lookalike = v.top.join("sub/.Trash/notes.txt");
    write_at(&shared_lookalike, b"s");
    for path in [&lookalike, &shared_lookalike] {
        let item = trash_on(&trash, path).unwrap();
        assert!(!path.exists());
        restore_on(&trash, &item).unwrap();
        assert!(path.exists());
    }
    let at_top = v.top.join(&own_trash);
    let (_, message) = say(trash_on(&trash, &at_top));
    assert!(message.contains("in the Trash already"), "{message}");
}

/// A long name cut for `files/` stays text: the reported path names the item exactly, Undo finds it,
/// and the record keeps the full name.
#[test]
fn a_long_name_with_characters_of_several_bytes_comes_back_exactly() {
    let m = machine("long-utf8");
    let work = m.home.join("work");
    let names = [
        "é".repeat(127),
        "日".repeat(85),
        format!("{}本", "日".repeat(84)),
        format!("a{}", "€".repeat(84)),
    ];
    for name in &names {
        std::fs::write(work.join(name), name.as_bytes()).unwrap();
    }
    let paths: Vec<PathBuf> = names.iter().map(|name| work.join(name)).collect();
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

    let (trashed, failed) = move_to_trash(&refs).unwrap();

    assert!(failed.is_empty(), "{failed:?}");
    for item in &trashed {
        let in_trash = Path::new(&item.trash_path);
        assert!(!item.trash_path.contains('\u{FFFD}'), "{}", item.trash_path);
        assert!(
            std::fs::symlink_metadata(in_trash).is_ok(),
            "{}",
            item.trash_path
        );
        let mut record = in_trash.file_name().unwrap().to_os_string();
        record.push(".trashinfo");
        let info =
            std::fs::read_to_string(m.home.join(".local/share/Trash/info").join(record)).unwrap();
        assert!(info.contains("Path=/"), "{info}");
    }
    let (restored, failed) = put_back(&trashed).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(restored.len(), names.len());
    for (name, path) in names.iter().zip(&paths) {
        assert_eq!(std::fs::read(path).unwrap(), name.as_bytes());
    }
}

/// A long name that is not text is cut by bytes, and still a name the filesystem takes and gives back.
#[test]
fn a_long_name_that_is_not_text_comes_back_exactly() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;
    let m = machine("long-bytes");
    let trash = super::platform::Trash::new(m.home.join(".local/share/Trash"), euid());
    let path = m.home.join("work").join(OsStr::from_bytes(&[0xff; 255]));
    std::fs::write(&path, b"raw").unwrap();

    let trash_path = trash.move_to(&path).unwrap();
    assert_eq!(trash_path.file_name().unwrap().len(), 245);
    trash.restore(&trash_path, &path).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"raw");
}
