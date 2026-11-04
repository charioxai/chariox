use super::*;
use std::fs;

const HOUR: Duration = Duration::from_secs(3600);

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
    assert_eq!(first, root.join("18e4df7c/210eff29"));
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
        &BTreeSet::new(),
    );
    assert_eq!(trimmed.items, 2);
    assert!(trimmed.bytes >= 4096);
    assert!(
        !folder.exists(),
        "an agent folder left empty goes with its last item"
    );
    // Now: nothing is older than the window.
    let folder = agent_folder(&scratch);
    fs::write(folder.join("fresh.txt"), b"x").unwrap();
    let trimmed = trim(
        &scratch.0.join("t"),
        later(Duration::ZERO),
        HOUR,
        &BTreeSet::new(),
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
    let held = BTreeSet::from([folder.join("open.sock"), folder.join("busy/deep")]);
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
        &BTreeSet::new(),
    );
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
