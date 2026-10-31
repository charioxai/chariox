//! The ledger's decisions on observed commands, without processes: what a
//! pass holds, pauses and resumes given the room it is shown.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const GIB: u64 = 1024 * MIB;

/// A 100 GiB volume (floor 10 GiB) with `free` bytes left.
fn volume(free: &Arc<AtomicU64>) -> Box<Volume> {
    let free = free.clone();
    Box::new(move |_| {
        Some(Space {
            id: 1,
            free: free.load(Ordering::SeqCst),
            total: 100 * GIB,
        })
    })
}

fn room<'a>(memory: u64, tight: bool, volume: &'a Volume) -> Room<'a> {
    Room {
        memory,
        tight,
        light: 512 * MIB,
        growth_room: GIB,
        volume,
    }
}

fn command(pid: u32, resident: u64, growth: u64) -> pool::CommandUse {
    pool::CommandUse {
        pid,
        resident,
        growth,
        writes: 0,
        write_growth: 0,
        effective: pid,
        root: false,
    }
}

fn entry(arrived: u64, expected: Peak) -> Entry {
    Entry {
        arrived,
        expected: Some(expected),
        seen: Some(Seen {
            pid: arrived as u32,
            key: format!("key-{arrived}"),
            line: format!("job {arrived}"),
            cwd: Some(PathBuf::from("/scratch")),
        }),
        ..Entry::default()
    }
}

fn owner(
    unit: &str,
    holdable: bool,
    commands: Vec<pool::CommandUse>,
    entries: Vec<(u32, Entry)>,
) -> Arc<Mutex<Observation>> {
    Arc::new(Mutex::new(Observation {
        unit: unit.into(),
        holdable,
        commands,
        admission: entries.into_iter().collect(),
        ..Observation::default()
    }))
}

fn wait(member: &Arc<Mutex<Observation>>, pid: u32) -> Option<Wait> {
    member.lock().unwrap().admission[&pid].wait
}

fn scratch(writes: u64) -> Peak {
    Peak { memory: 0, writes }
}

/// Lets the next resume of any reason happen at once.
fn unsettle(ledger: &mut Ledger) {
    ledger.resumed_at.clear();
}

#[test]
fn two_writers_that_together_reach_the_floor_do_not_both_start() {
    // 8 GiB above the floor; each job writes 6 GiB of scratch and deletes it.
    let free = Arc::new(AtomicU64::new(18 * GIB));
    let volume = volume(&free);
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![
            (1, entry(1, scratch(6 * GIB))),
            (2, entry(2, scratch(6 * GIB))),
        ],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &volume),
    );
    assert_eq!(wait(&current, 1), None, "the first writer was held");
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Held(WaitReason::Disk)),
        "the second writer started although both together reach the floor"
    );
    // The first ends: the second gets the room.
    current.lock().unwrap().admission.remove(&1);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &volume),
    );
    assert_eq!(wait(&current, 2), None, "the second writer never started");
}

#[test]
fn a_resumed_writer_keeps_its_room_from_the_next_to_resume() {
    let free = Arc::new(AtomicU64::new(18 * GIB));
    let volume = volume(&free);
    let held = |arrived| Entry {
        wait: Some(Wait::Held(WaitReason::Disk)),
        ..entry(arrived, scratch(6 * GIB))
    };
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![(1, held(1)), (2, held(2))],
    );
    let mut ledger = Ledger::new(None);
    let members = std::slice::from_ref(&current);
    ledger.pass(members, &current, &room(GIB, false, &volume));
    assert_eq!(wait(&current, 1), None);
    assert!(wait(&current, 2).is_some());
    unsettle(&mut ledger);
    ledger.pass(members, &current, &room(GIB, false, &volume));
    assert!(
        wait(&current, 2).is_some(),
        "the second writer resumed onto room the first one needs"
    );
}

#[test]
fn writers_on_different_volumes_reserve_room_separately() {
    // Each volume has 8 GiB above its floor; a path under /other is the second.
    let volume: Box<Volume> = Box::new(|path| {
        Some(Space {
            id: if path.starts_with("/other") { 2 } else { 1 },
            free: 18 * GIB,
            total: 100 * GIB,
        })
    });
    let mut elsewhere = entry(3, scratch(6 * GIB));
    elsewhere.seen.as_mut().unwrap().cwd = Some(PathBuf::from("/other/build"));
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0), command(3, 0, 0)],
        vec![
            (1, entry(1, scratch(6 * GIB))),
            (2, entry(2, scratch(6 * GIB))),
            (3, elsewhere),
        ],
    );
    Ledger::new(None).pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &volume),
    );
    assert_eq!(wait(&current, 1), None);
    assert_eq!(wait(&current, 2), Some(Wait::Held(WaitReason::Disk)));
    assert_eq!(
        wait(&current, 3),
        None,
        "another volume's room was reserved"
    );
}
