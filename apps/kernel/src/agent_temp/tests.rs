use super::*;
use std::fs;

const HOUR: Duration = Duration::from_secs(3600);

#[cfg(target_os = "linux")]
#[test]
fn a_bound_unix_socket_survives_a_sweep_and_accepts_new_connections() {
    use std::os::unix::net::{UnixListener, UnixStream};
    let scratch = Scratch::new("socket");
    let folder = agent_folder(&scratch);
    let socket = folder.join("live.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::write(folder.join("stale.txt"), b"disposable").unwrap();
    let held = held_paths().expect("Linux can inspect held paths");
    trim(&scratch.0.join("t"), later(2 * HOUR), HOUR, &held);
    assert!(!folder.join("stale.txt").exists());
    let connection = UnixStream::connect(&socket);
    assert!(
        connection.is_ok(),
        "the sweep unlinked a live Unix socket: {connection:?}"
    );
    listener.accept().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_unix_socket_bound_by_relative_name_survives_its_holder_changing_cwd() {
    use std::io::Read;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::process::{Command, Stdio};
    for (nested, prefix) in [(false, "live"), (true, "live"), (false, "@live")] {
        let scratch = Scratch::new("rel");
        let folder = agent_folder(&scratch);
        let name = format!("{prefix}-{}.sock", std::process::id());
        let (bind_dir, bind_name, socket) = if nested {
            (
                folder.join("binder"),
                format!("../tree/{name}"),
                folder.join("tree").join(&name),
            )
        } else {
            (folder.clone(), name.clone(), folder.join(&name))
        };
        fs::create_dir_all(&bind_dir).unwrap();
        fs::create_dir_all(socket.parent().unwrap()).unwrap();
        fs::write(folder.join("stale.txt"), b"disposable").unwrap();
        let stale_socket = folder.join("closed.sock");
        drop(UnixListener::bind(&stale_socket).unwrap());
        // The socket table retains the bind name after the holder changes cwd.
        let mut server = Command::new("python3")
            .current_dir(&bind_dir)
            .env("TMPDIR", &folder)
            .args([
                "-I",
                "-c",
                "import os,socket,sys,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\nos.chdir('/')\nprint('ready',flush=True)\ntime.sleep(120)",
                &bind_name,
            ])
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 starts");
        let mut ready = [0u8; 5];
        server
            .stdout
            .take()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        let held = held_paths().expect("Linux can inspect held paths");
        trim(&scratch.0.join("t"), later(2 * HOUR), HOUR, &held);
        let connection = UnixStream::connect(&socket);
        let _ = server.kill();
        let _ = server.wait();
        assert!(
            connection.is_ok(),
            "the sweep unlinked a live Unix socket bound by {bind_name:?}: {connection:?}"
        );
        assert!(!folder.join("stale.txt").exists());
        assert!(
            !stale_socket.exists(),
            "an unrelated closed socket stays disposable"
        );
        drop(connection);
        let held = held_paths().expect("Linux can inspect held paths after the holder exits");
        trim(&scratch.0.join("t"), later(2 * HOUR), HOUR, &held);
        assert!(!socket.exists(), "the closed endpoint becomes disposable");
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("agent-temp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn agent_folder(scratch: &Scratch) -> PathBuf {
    let folder = scratch.0.join("t/room0001/agent001");
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn later(by: Duration) -> SystemTime {
    SystemTime::now() + by
}

#[test]
fn the_path_is_short_and_the_same_after_a_restart() {
    let scratch = Scratch::new("short");
    let root = scratch.0.join("t");
    let (room, agent) = ("18e4df7c1509db5b", "agent-210eff29d53051337c3df0cbaeb69178");
    let first = dir_in(&root, room, agent);
    assert_eq!(first, root.join("18e4df7c1509db5b/210eff29"));
    assert_eq!(first, dir_in(&root, room, agent));
    let vars = environment_in(&root, room, agent);
    assert_eq!(
        vars.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["TMPDIR", "TMP", "TEMP"]
    );
    assert!(first.is_dir());
}

#[test]
fn stale_items_go_and_fresh_ones_stay() {
    let scratch = Scratch::new("stale");
    let folder = agent_folder(&scratch);
    fs::write(folder.join("old.log"), vec![1u8; 4096]).unwrap();
    fs::create_dir_all(folder.join("build-cache")).unwrap();
    fs::write(folder.join("build-cache/obj.o"), b"x").unwrap();
    // Two hours on: both items have been idle for longer than the window.
    let trimmed = trim(
        &scratch.0.join("t"),
        later(2 * HOUR),
        HOUR,
        &HeldPaths::default(),
    );
    assert_eq!(trimmed.items, 2);
    assert!(trimmed.bytes >= 4096);
    assert!(
        folder.is_dir(),
        "an empty TMPDIR remains available until the Room ends"
    );
    // Now: nothing is older than the window.
    let folder = agent_folder(&scratch);
    fs::write(folder.join("fresh.txt"), b"x").unwrap();
    let trimmed = trim(
        &scratch.0.join("t"),
        later(Duration::ZERO),
        HOUR,
        &HeldPaths::default(),
    );
    assert_eq!(trimmed, Trimmed::default());
    assert!(folder.join("fresh.txt").exists());
}

#[test]
fn an_item_a_process_has_open_or_works_in_is_never_removed() {
    let scratch = Scratch::new("inuse");
    let folder = agent_folder(&scratch);
    fs::write(folder.join("open.sock"), b"x").unwrap();
    fs::create_dir_all(folder.join("busy/deep")).unwrap();
    fs::write(folder.join("busy/deep/out"), b"x").unwrap();
    fs::write(folder.join("stale.txt"), b"x").unwrap();
    let held = HeldPaths {
        paths: BTreeSet::from([folder.join("open.sock"), folder.join("busy/deep")]),
        ..HeldPaths::default()
    };
    let trimmed = trim(&scratch.0.join("t"), later(2 * HOUR), HOUR, &held);
    assert_eq!(trimmed.items, 1, "only the stale item goes");
    assert!(
        folder.join("open.sock").exists(),
        "an open file was removed"
    );
    assert!(
        folder.join("busy/deep/out").exists(),
        "a busy folder was removed"
    );
    assert!(!folder.join("stale.txt").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn an_open_file_survives_a_sweep_through_a_symlinked_home_or_ancestor() {
    let scratch = Scratch::new("aliased-home");
    let home = scratch.0.join("real/home");
    fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(&home, scratch.0.join("home-link")).unwrap();
    std::os::unix::fs::symlink(scratch.0.join("real"), scratch.0.join("parent-link")).unwrap();
    for alias in [
        scratch.0.join("home-link"),
        scratch.0.join("parent-link/home"),
    ] {
        let root = alias.join("t");
        let folder = dir_in(&root, "room0001", "agent-001");
        fs::create_dir_all(&folder).unwrap();
        let open = folder.join("open.txt");
        fs::write(&open, b"keep").unwrap();
        let file = fs::File::open(&open).unwrap();
        fs::write(folder.join("stale.txt"), b"disposable").unwrap();
        let held = held_paths().expect("Linux can inspect the real open file");
        let trimmed = trim(&root, later(2 * HOUR), HOUR, &held);
        assert!(
            open.exists(),
            "a sweep through {alias:?} removed an open file"
        );
        assert!(!folder.join("stale.txt").exists());
        assert_eq!(trimmed.items, 1, "only the idle sibling is disposable");
        drop(file);
        trim(&root, later(2 * HOUR), HOUR, &held_paths().unwrap());
        assert!(!open.exists(), "the closed file becomes disposable");
    }
}

#[cfg(unix)]
#[test]
fn an_aliased_held_path_preserves_its_real_scratch_item() {
    let scratch = Scratch::new("aliased-holder");
    let folder = agent_folder(&scratch);
    std::os::unix::fs::symlink(&folder, scratch.0.join("alias")).unwrap();
    fs::write(folder.join("held.txt"), b"keep").unwrap();
    fs::write(folder.join("stale.txt"), b"disposable").unwrap();
    // Absolute Unix socket names can retain the alias used at bind time.
    let held = HeldPaths {
        paths: BTreeSet::from([scratch.0.join("alias/held.txt")]),
        ..HeldPaths::default()
    };
    let trimmed = trim(&scratch.0.join("t"), later(2 * HOUR), HOUR, &held);
    assert!(folder.join("held.txt").exists());
    assert!(!folder.join("stale.txt").exists());
    assert_eq!(trimmed.items, 1);
}

#[test]
fn a_symlink_is_removed_but_never_followed() {
    let scratch = Scratch::new("link");
    let folder = agent_folder(&scratch);
    let outside = scratch.0.join("workspace");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.rs"), b"fn main() {}").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, folder.join("to-workspace")).unwrap();
    trim(
        &scratch.0.join("t"),
        later(2 * HOUR),
        HOUR,
        &HeldPaths {
            paths: BTreeSet::from([outside.join("keep.rs")]),
            ..HeldPaths::default()
        },
    );
    #[cfg(unix)]
    assert!(!folder.join("to-workspace").is_symlink());
    assert!(
        outside.join("keep.rs").exists(),
        "the trim followed a link out of the temp folder"
    );
}

#[test]
fn ending_a_room_clears_its_temp_at_once() {
    let scratch = Scratch::new("room");
    let folder = agent_folder(&scratch);
    fs::write(folder.join("fresh.txt"), b"x").unwrap();
    let room = folder.parent().unwrap().to_path_buf();
    let other = scratch.0.join("t/room0002/agent001");
    fs::create_dir_all(&other).unwrap();
    clear_room_in(&scratch.0.join("t"), "room0001");
    assert!(!room.exists());
    assert!(other.exists(), "another Room's temp folder was cleared");
}

#[test]
fn ending_a_room_preserves_scratch_for_another_with_the_same_prefix() {
    let scratch = Scratch::new("shared-prefix");
    let root = scratch.0.join("t");
    let (ended, live) = ("18e4df7c1509db5b", "18e4df7c2509db5b");
    let agent = "agent-210eff29d53051337c3df0cbaeb69178";
    environment_in(&root, ended, agent);
    environment_in(&root, live, agent);
    let file = dir_in(&root, live, agent).join("keep.txt");
    fs::write(&file, b"live scratch").unwrap();
    clear_room_in(&root, ended);
    assert!(!room_dir(&root, ended).exists());
    assert_eq!(fs::read(&file).unwrap(), b"live scratch");
    #[cfg(unix)]
    {
        let socket = dir_in(&root, live, agent).join("live.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        clear_room_in(&root, ended);
        std::os::unix::net::UnixStream::connect(&socket).unwrap();
        listener.accept().unwrap();
    }
}
