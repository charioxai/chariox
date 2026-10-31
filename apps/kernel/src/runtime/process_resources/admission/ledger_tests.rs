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
        holdable: true,
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
#[test]
fn a_disk_wait_does_not_resume_onto_memory_another_build_took() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    // A runs with 800 MiB still to grow; 1000 MiB are left, so 200 are free.
    let mut build = entry(
        1,
        Peak {
            memory: 900 * MIB,
            writes: 0,
        },
    );
    build.admitted = true;
    build.resident = 100 * MIB;
    let waiting = Entry {
        wait: Some(Wait::Held(WaitReason::Disk)),
        ..entry(
            2,
            Peak {
                memory: 600 * MIB,
                writes: GIB,
            },
        )
    };
    let current = owner(
        "agent",
        true,
        vec![command(1, 100 * MIB, 0), command(2, 0, 0)],
        vec![(1, build), (2, waiting)],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(1000 * MIB, false, &volume),
    );
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Held(WaitReason::Disk)),
        "a command held for disk resumed with no memory budget left"
    );
}

#[test]
fn a_memory_wait_does_not_resume_onto_a_disk_that_filled() {
    // 2 GiB above the floor, while the command writes 6 GiB.
    let free = Arc::new(AtomicU64::new(12 * GIB));
    let volume = volume(&free);
    let waiting = Entry {
        wait: Some(Wait::Held(WaitReason::Memory)),
        ..entry(
            1,
            Peak {
                memory: 600 * MIB,
                writes: 6 * GIB,
            },
        )
    };
    let current = owner("agent", true, vec![command(1, 0, 0)], vec![(1, waiting)]);
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(4 * GIB, false, &volume),
    );
    assert_eq!(
        wait(&current, 1),
        Some(Wait::Held(WaitReason::Memory)),
        "a command held for memory resumed onto a full disk"
    );
    free.store(30 * GIB, Ordering::SeqCst);
    unsettle(&mut ledger);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(4 * GIB, false, &volume),
    );
    assert_eq!(wait(&current, 1), None, "it did not resume with both free");
}
#[test]
fn a_rejected_start_now_leaves_the_running_command_pausable() {
    let mut running = entry(2, Peak::default());
    running.expected = None;
    running.admitted = true;
    running.resident = 700 * MIB;
    assert!(!running.force(), "a running command read as waiting");
    assert!(!running.forced && running.admitted, "flags changed");
    let mut older = entry(1, Peak::default());
    older.expected = None;
    older.admitted = true;
    older.resident = GIB;
    let current = owner(
        "agent",
        true,
        vec![command(1, GIB, 0), command(2, 700 * MIB, 8 * MIB)],
        vec![(1, older), (2, running)],
    );
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let may_stop = Ledger::new(None).pass(
        std::slice::from_ref(&current),
        &current,
        &room(100 * MIB, true, &volume),
    );
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Paused(WaitReason::Memory)),
        "pressure no longer pauses a command whose Start now was refused"
    );
    assert!(!may_stop);
}

/// Starts `pid` under the owner's current policy, as a sample would.
fn start(ledger: &mut Ledger, member: &Arc<Mutex<Observation>>, pid: u32, resident: u64) {
    member
        .lock()
        .unwrap()
        .commands
        .push(command(pid, resident, 8 * MIB));
    ledger.register(member, 512 * MIB);
}

/// An older heavy command of another owner: never the one paused.
fn oldest(ledger: &mut Ledger) -> Arc<Mutex<Observation>> {
    let other = owner("other", true, Vec::new(), Vec::new());
    start(ledger, &other, 4_100_001, GIB);
    other
}

#[test]
fn a_persons_job_stays_unpausable_when_an_agent_types_after_it() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    // An agent's job, then a person's own (the newest), then an agent types
    // at the prompt the person's job returned.
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    terminal.lock().unwrap().holdable = false;
    start(&mut ledger, &terminal, 4_100_003, 700 * MIB);
    terminal.lock().unwrap().holdable = true;
    let members = [other, terminal.clone()];
    ledger.pass(&members, &terminal, &room(100 * MIB, true, &volume));
    assert_eq!(
        wait(&terminal, 4_100_003),
        None,
        "a person's job was paused after an agent typed"
    );
    assert_eq!(
        wait(&terminal, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory)),
        "the agent's job was spared"
    );
}

#[test]
fn an_agents_job_stays_pausable_when_the_person_types_after_it() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    // The person types at the prompt the agent's job returned.
    terminal.lock().unwrap().holdable = false;
    let members = [other, terminal.clone()];
    ledger.pass(&members, &terminal, &room(100 * MIB, true, &volume));
    assert_eq!(
        wait(&terminal, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory)),
        "an agent's job was exempted by a person's later keystroke"
    );
}
