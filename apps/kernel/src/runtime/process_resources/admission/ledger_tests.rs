//! The ledger's decisions on observed commands, without processes: what a
//! pass holds, pauses and resumes given the room it is shown.
use super::super::submissions::Submissions;
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const GIB: u64 = 1024 * MIB;
static EMPTY_LAUNCHES: std::sync::LazyLock<BTreeMap<String, super::super::launch::Reservation>> =
    std::sync::LazyLock::new(BTreeMap::new);

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
        margin: if tight { memory + 1 } else { 0 },
        growing: 0,
        pressured: true,
        memory_protection: true,
        disk_protection: true,
        host_bound: false,
        light: 512 * MIB,
        growth_room: GIB,
        capacity: 64 * GIB,
        volume,
        launches: std::sync::LazyLock::force(&EMPTY_LAUNCHES),
        launch_memory: BTreeMap::new(),
    }
}

fn command(pid: u32, resident: u64, growth: u64) -> pool::CommandUse {
    pool::CommandUse {
        pid,
        deleted_open: 0,
        group: None,
        resident,
        growth,
        steady_growth: growth,
        measured_growth: growth,
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
            exact: 0,
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
        submissions: Submissions::new(holdable),
        disk_writes: commands
            .iter()
            .filter(|command| command.write_growth > 0)
            .map(|command| {
                (
                    command.pid,
                    vec![disk_use::VolumeWrites {
                        id: 1,
                        path: "/scratch/output".into(),
                        written: command.writes,
                        rate: command.write_growth,
                        files: [u64::from(command.pid)].into(),
                    }],
                )
            })
            .collect(),
        commands,
        admission: entries.into_iter().collect(),
        ..Observation::default()
    }))
}

fn wait(member: &Arc<Mutex<Observation>>, pid: u32) -> Option<Wait> {
    member.lock().unwrap().admission[&pid].wait
}

/// A peak whose run wrote `writes` bytes to the volume under `/scratch`.
fn scratch(writes: u64) -> Peak {
    Peak {
        writes,
        volume: Some(ReceivingVolume {
            path: "/scratch/output".into(),
            id: 1,
        }),
        volume_writes: writes,
        ..Peak::default()
    }
}

/// Lets the next resume of any reason happen at once.
fn unsettle(ledger: &mut Ledger) {
    ledger.resumed_at.clear();
    ledger.filler_at = None;
}

#[test]
fn a_continued_command_settles_before_memory_can_pause_it_again() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let mut ledger = Ledger::new(None);
    let mut first = entry(1, Peak::default());
    first.admitted = true;
    first.resident = 600 * MIB;
    let oldest = owner(
        "oldest",
        true,
        vec![command(1, 600 * MIB, 50 * MIB)],
        vec![(1, first)],
    );
    let mut next = entry(2, Peak::default());
    next.admitted = true;
    next.resident = 600 * MIB;
    next.paused_rate = 50 * MIB;
    next.wait_for(Wait::Paused(WaitReason::Memory));
    next.stopped = true;
    let continued = owner(
        "continued",
        true,
        vec![command(2, 600 * MIB, 50 * MIB)],
        vec![(2, next)],
    );
    let members = vec![oldest.clone(), continued.clone()];
    let ample = room(4 * GIB, false, &volume);
    let mut state = State::read(&members, &ample);
    assert!(ledger
        .resume(&members, WaitReason::Memory, &mut state, &ample)
        .is_some());
    let tight = room(0, true, &volume);

    // The continue may still be waiting in the watch's control channel.
    assert!(!ledger.pass(&members, &continued, &tight));
    assert_eq!(wait(&continued, 2), None);
    continued
        .lock()
        .unwrap()
        .admission
        .get_mut(&2)
        .unwrap()
        .continued();
    assert!(!ledger.pass(&members, &continued, &tight));
    assert_eq!(wait(&continued, 2), None);

    // Pressure still pauses another growing command during this one's settle.
    let mut latest = entry(3, Peak::default());
    latest.admitted = true;
    latest.resident = 600 * MIB;
    let newer = owner(
        "newer",
        true,
        vec![command(3, 600 * MIB, 50 * MIB)],
        vec![(3, latest)],
    );
    let all = vec![oldest.clone(), continued.clone(), newer.clone()];
    assert!(!ledger.pass(&all, &newer, &tight));
    assert_eq!(wait(&newer, 3), Some(Wait::Paused(WaitReason::Memory)));
    assert_eq!(wait(&continued, 2), None);

    continued
        .lock()
        .unwrap()
        .admission
        .get_mut(&2)
        .unwrap()
        .resumed_at = Some(Instant::now() - SETTLE);
    assert!(!ledger.pass(&all, &continued, &tight));
    assert_eq!(wait(&continued, 2), Some(Wait::Paused(WaitReason::Memory)));
    assert_eq!(wait(&oldest, 1), None);
}

fn sample(memory: u64) -> ProcessSample {
    ProcessSample {
        process_id: "binary".into(),
        pid: 1,
        command: "binary".into(),
        resident_set_bytes: memory,
        cpu_millis: 0,
        disk_io_bytes: None,
        sampled_at_ms: 0,
        age_ms: 0,

        disk_written_bytes: None,
        deleted_open_bytes: 0,
        outlived: false,
    }
}

fn reserved_binary(prediction: u64) -> (pool::Pool, Arc<Mutex<Observation>>) {
    let pool = pool::Pool::with_volume(10 * GIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    pool.launches.lock().unwrap().insert(
        "binary".into(),
        super::super::launch::Reservation {
            memory: prediction,
            writes: 0,
            volume: 1,
        },
    );
    let entries = [1, 2].map(|pid| {
        let mut entry = entry(
            pid as u64,
            Peak {
                memory: 4 * GIB,
                writes: 0,

                ..Default::default()
            },
        );
        entry.admitted = true;
        (pid, entry)
    });
    let member = owner(
        "binary",
        true,
        vec![command(1, GIB, 32 * MIB), command(2, GIB, 32 * MIB)],
        entries.into(),
    );
    member.lock().unwrap().samples = vec![sample(2 * GIB)];
    pool.join(&member);
    (pool, member)
}

#[test]
fn binary_descendants_with_unused_reservations_keep_growing() {
    for prediction in [9 * GIB, 9 * GIB + 256 * MIB, 9 * GIB + 512 * MIB] {
        let (pool, member) = reserved_binary(prediction);
        for _ in 0..3 {
            pool.enforce(&member, None);
            assert_eq!(wait(&member, 1), None);
            assert_eq!(
                wait(&member, 2),
                None,
                "prediction {prediction} caused false pressure"
            );
        }
    }
}

#[test]
fn paused_binary_descendant_resumes_with_actual_headroom_and_live_reservation() {
    for prediction in [9 * GIB, 9 * GIB + 256 * MIB, 9 * GIB + 512 * MIB] {
        let (pool, member) = reserved_binary(prediction);
        let pressure = owner("pressure", false, vec![], vec![]);
        pressure.lock().unwrap().samples = vec![sample(8 * GIB - 32 * MIB)];
        pool.join(&pressure);
        pool.enforce(&member, None);
        assert_eq!(wait(&member, 1), None, "oldest descendant must progress");
        assert_eq!(wait(&member, 2), Some(Wait::Paused(WaitReason::Memory)));
        pressure.lock().unwrap().samples.clear();
        pool.enforce(&member, None);
        assert_eq!(
            wait(&member, 2),
            None,
            "prediction {prediction} stranded a paused descendant"
        );
        assert_eq!(pool.launches.lock().unwrap()["binary"].memory, prediction);
    }
}

/// A 1500 MiB pool the oldest running command has filled while two younger
/// commands wait, paused, on what they hold: the newest paused command ends
/// for it, with its reason; the middle one and the oldest are not touched,
/// and once the newest's memory is free nothing else is stopped.
#[test]
fn the_newest_paused_command_yields_before_the_oldest_running_one() {
    let pool = pool::Pool::with_volume(1500 * MIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    let command_of = |unit: &str, pid: u32, resident: u64, growth: u64, wait: Option<Wait>| {
        let mut waiting = entry(
            pid as u64,
            Peak {
                memory: 800 * MIB,
                ..Default::default()
            },
        );
        waiting.admitted = true;
        waiting.wait = wait;
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, growth)],
            vec![(pid, waiting)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let paused = Some(Wait::Paused(WaitReason::Memory));
    let (oldest, oldest_heard) = command_of("a", 1, 700 * MIB, 48 * MIB, None);
    let (middle, middle_heard) = command_of("b", 2, 350 * MIB, 0, paused);
    let (newest, newest_heard) = command_of("c", 3, 350 * MIB, 0, paused);
    let stops = |heard: &std::sync::mpsc::Receiver<WatchCommand>| {
        heard
            .try_iter()
            .filter_map(|command| match command {
                WatchCommand::MemoryStop { pid, notice, .. } => Some((pid, notice)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    pool.enforce(&oldest, None);
    let ended = stops(&newest_heard);
    assert_eq!(ended.len(), 1, "the newest paused command was not stopped");
    assert_eq!(ended[0].0, 3);
    assert!(
        ended[0].1.contains("stopped this waiting command"),
        "{}",
        ended[0].1
    );
    assert!(
        stops(&middle_heard).is_empty(),
        "more than the newest was stopped"
    );
    assert!(
        stops(&oldest_heard).is_empty(),
        "the oldest running command was stopped"
    );
    assert_eq!(wait(&oldest, 1), None, "the oldest command was paused");
    assert_eq!(wait(&middle, 2), paused);

    // The newest's memory is free: the pool has room and nobody else yields.
    {
        let mut newest = newest.lock().unwrap();
        newest.samples.clear();
        newest.commands.clear();
    }
    pool.enforce(&oldest, None);
    assert!(stops(&middle_heard).is_empty() && stops(&oldest_heard).is_empty());
}

/// A paused command continues only with room to grow ahead. When memory is
/// above the pressure line but never reaches that room, as the one heavy
/// running command holds what it holds, the pause is no deadlock: after the
/// stall window the paused command ends, and it is not paused again at once.
#[test]
fn a_paused_command_that_never_gets_room_to_grow_ends_after_the_stall_window() {
    let pool = pool::Pool::with_volume(8 * GIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    pool.ledger.lock().unwrap().stall_window = Duration::from_millis(50);
    let command_of = |unit: &str, pid: u32, resident: u64, wait: Option<Wait>| {
        let mut entry = entry(pid as u64, Peak::default());
        entry.admitted = true;
        entry.wait = wait;
        entry.since_ms = crate::session::unix_epoch_ms();
        entry.paused_rate = 2 * GIB;
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, 0)],
            vec![(pid, entry)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let (running, _) = command_of("a", 1, 3 * GIB, None);
    let (paused, heard) = command_of("b", 2, 600 * MIB, Some(Wait::Paused(WaitReason::Memory)));
    let ended = || {
        heard
            .try_iter()
            .filter(|command| matches!(command, WatchCommand::MemoryStop { .. }))
            .count()
    };

    pool.enforce(&running, None);
    assert!(
        matches!(
            paused.lock().unwrap().admission[&2].wait,
            Some(Wait::Paused(_))
        ),
        "the command continued without room to grow"
    );
    assert_eq!(ended(), 0, "it ended before the stall window passed");
    std::thread::sleep(Duration::from_millis(80));
    pool.enforce(&running, None);
    assert_eq!(ended(), 1, "a pause with no way out was never ended");
}

/// A stopped command measures a growth that halves at every sample, but the
/// rate it was paused at stays its own: the room to continue still counts it,
/// so the pause does not end by continuing once the samples have fallen.
#[test]
fn a_stopped_command_whose_measured_growth_halves_stays_paused_and_ends_after_the_stall_window() {
    let pool = pool::Pool::with_volume(8 * GIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    pool.ledger.lock().unwrap().stall_window = Duration::from_millis(50);
    let command_of = |unit: &str, pid: u32, resident: u64, wait: Option<Wait>| {
        let mut entry = entry(pid as u64, Peak::default());
        entry.admitted = true;
        entry.wait = wait;
        entry.since_ms = crate::session::unix_epoch_ms();
        entry.paused_rate = 2 * GIB;
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, 0)],
            vec![(pid, entry)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let (running, _) = command_of("a", 1, 3 * GIB, None);
    let (paused, heard) = command_of("b", 2, 600 * MIB, Some(Wait::Paused(WaitReason::Memory)));
    let ended = || {
        heard
            .try_iter()
            .filter(|command| matches!(command, WatchCommand::MemoryStop { .. }))
            .count()
    };

    for measured in [2 * GIB, GIB, 512 * MIB, 256 * MIB, 128 * MIB, 64 * MIB] {
        paused.lock().unwrap().commands[0].growth = measured;
        pool.enforce(&running, None);
        assert!(
            matches!(
                paused.lock().unwrap().admission[&2].wait,
                Some(Wait::Paused(_))
            ),
            "the command continued once its measured growth fell to {measured}"
        );
    }
    assert_eq!(ended(), 0, "it ended before the stall window passed");
    std::thread::sleep(Duration::from_millis(80));
    pool.enforce(&running, None);
    assert_eq!(ended(), 1, "a pause with no way out was never ended");
}

/// Memory-paused commands keep their memory: when the one running command
/// does not grow or finish either, nothing frees by itself. After the stall
/// window the largest paused command ends, with its reason.
#[test]
fn paused_commands_that_free_nothing_end_the_largest_after_the_stall_window() {
    let pool = pool::Pool::with_volume(1500 * MIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    pool.ledger.lock().unwrap().stall_window = Duration::from_millis(50);
    let command_of = |unit: &str, pid: u32, resident: u64, wait: Option<Wait>| {
        let mut waiting = entry(
            pid as u64,
            Peak {
                memory: 800 * MIB,
                ..Default::default()
            },
        );
        waiting.admitted = true;
        waiting.wait = wait;
        waiting.since_ms = crate::session::unix_epoch_ms();
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, 0)],
            vec![(pid, waiting)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let paused = Some(Wait::Paused(WaitReason::Memory));
    let (running, _running_heard) = command_of("a", 1, 650 * MIB, None);
    let (_middle, middle_heard) = command_of("b", 2, 400 * MIB, paused);
    let (_newest, newest_heard) = command_of("c", 3, 350 * MIB, paused);
    let stops = |heard: &std::sync::mpsc::Receiver<WatchCommand>| {
        heard
            .try_iter()
            .filter_map(|command| match command {
                WatchCommand::MemoryStop { pid, notice, .. } => Some((pid, notice)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    pool.enforce(&running, None);
    assert!(
        stops(&middle_heard).is_empty() && stops(&newest_heard).is_empty(),
        "a command ended before the stall window passed"
    );
    std::thread::sleep(Duration::from_millis(80));
    pool.enforce(&running, None);
    let ended = stops(&middle_heard);
    assert_eq!(ended.len(), 1, "the largest paused command was not ended");
    assert!(
        ended[0].1.contains("paused for memory") && ended[0].1.contains("freed nothing"),
        "{}",
        ended[0].1
    );
    assert!(stops(&newest_heard).is_empty(), "more than one was ended");
}

/// A command paused on memory that the disk, not the memory, keeps from
/// continuing (its known writes do not fit above the floor) ends after 60 s
/// with the disk as the reason: the notice does not print memory figures that
/// did not decide it, such as a memory room larger than the need.
#[test]
fn a_pause_the_disk_keeps_ends_naming_the_disk() {
    // Below the last-resort disk reserve; the paused command writes 6 GiB.
    let pool = pool::Pool::with_volume(1500 * MIB, volume(&Arc::new(AtomicU64::new(128 * MIB))));
    let command_of = |unit: &str, pid: u32, expected: Peak, wait: Option<Wait>, since_ms: u64| {
        let mut waiting = entry(pid as u64, expected);
        waiting.admitted = true;
        waiting.wait = wait;
        waiting.since_ms = since_ms;
        let member = owner(unit, true, vec![command(pid, 0, 0)], vec![(pid, waiting)]);
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(0)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let heavy = Peak {
        memory: 800 * MIB,
        ..Default::default()
    };
    let (running, _running_heard) = command_of("a", 1, heavy, None, 0);
    let writer = Peak {
        memory: 400 * MIB,
        ..scratch(6 * GIB)
    };
    let waited = crate::session::unix_epoch_ms() - 61_000;
    let (_paused, paused_heard) = command_of(
        "c",
        3,
        writer,
        Some(Wait::Paused(WaitReason::Memory)),
        waited,
    );

    pool.enforce(&running, None);
    let notice = paused_heard
        .try_iter()
        .find_map(|command| match command {
            WatchCommand::MemoryStop { notice, .. } => Some(notice),
            _ => None,
        })
        .expect("the paused command was not ended");
    assert!(
        notice.contains("it still needs 6 GB of disk above the floor")
            && notice.contains("only 0 GB are free on the volume"),
        "the disk kept it, but the notice names memory: {notice}"
    );
}

/// A pool of 1536 MiB with one heavy command running (300 MiB) and one
/// paused for memory 61 s ago (200 MiB resident, no expected peak) that grew
/// at `paused_rate` when it was paused. The watch measures a stopped command
/// at no growth. Returns what the paused command was told.
fn overdue_pause_that_grew_at(paused_rate: u64) -> Vec<(u32, String)> {
    let pool = pool::Pool::with_volume(1536 * MIB, volume(&Arc::new(AtomicU64::new(100 * GIB))));
    let command_of = |unit: &str, pid: u32, resident: u64, entry: Entry| {
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, 0)],
            vec![(pid, entry)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let running = Entry {
        admitted: true,
        expected: None,
        ..entry(1, Peak::default())
    };
    let (running, _running_heard) = command_of("a", 1, 300 * MIB, running);
    let paused = Entry {
        admitted: true,
        expected: None,
        wait: Some(Wait::Paused(WaitReason::Memory)),
        paused_rate,
        since_ms: crate::session::unix_epoch_ms() - 61_000,
        ..entry(2, Peak::default())
    };
    let (_paused, paused_heard) = command_of("c", 2, 200 * MIB, paused);
    pool.enforce(&running, None);
    paused_heard
        .try_iter()
        .filter_map(|command| match command {
            WatchCommand::MemoryStop { pid, notice, .. } => Some((pid, notice)),
            _ => None,
        })
        .collect()
}

/// A paused command keeps the rate it grew at when it was paused, although
/// it is stopped and measures none: it needs room for that growth, and ends
/// after 60 s when the room does not come back.
#[test]
fn a_pause_that_grew_fast_is_ended_when_the_room_does_not_return() {
    let stops = overdue_pause_that_grew_at(700 * MIB);
    assert_eq!(stops.len(), 1, "the growing paused command was not ended");
    assert!(stops[0].1.contains("still needs"), "{}", stops[0].1);
}

#[test]
fn predicted_writes_do_not_serialize_work_on_a_healthy_volume() {
    // Predictions may overlap; a healthy receiving volume is not pressure.
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
        None,
        "predictions held a writer on a healthy volume"
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
fn a_recovered_volume_does_not_keep_writers_waiting_for_predictions() {
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
        wait(&current, 2).is_none(),
        "the healthy volume left a writer waiting on predictions"
    );
}

#[test]
fn disk_emergency_only_holds_writers_on_the_affected_volume() {
    // One receiving volume is nearly full; /other is healthy.
    let volume: Box<Volume> = Box::new(|path| {
        Some(Space {
            id: if path.starts_with("/other") { 2 } else { 1 },
            free: if path.starts_with("/other") {
                18 * GIB
            } else {
                128 * MIB
            },
            total: 100 * GIB,
        })
    });
    let mut elsewhere = entry(3, scratch(6 * GIB));
    elsewhere.expected.as_mut().unwrap().volume = Some(ReceivingVolume {
        path: "/other/output".into(),
        id: 2,
    });
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
    assert_eq!(wait(&current, 1), Some(Wait::Held(WaitReason::Disk)));
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
            exact: 0,
            ..Peak::default()
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
                volume: Some(ReceivingVolume {
                    path: "/scratch/output".into(),
                    id: 1,
                }),
                volume_writes: GIB,
                exact: 0,
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
    // The receiving volume is below its last-resort reserve.
    let free = Arc::new(AtomicU64::new(128 * MIB));
    let volume = volume(&free);
    let waiting = Entry {
        wait: Some(Wait::Held(WaitReason::Memory)),
        ..entry(
            1,
            Peak {
                memory: 600 * MIB,
                writes: 6 * GIB,
                volume: Some(ReceivingVolume {
                    path: "/scratch/output".into(),
                    id: 1,
                }),
                volume_writes: 6 * GIB,
                exact: 0,
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

#[test]
fn two_builds_waiting_for_different_resources_do_not_resume_onto_the_same_memory() {
    // Nothing heavy runs; 8 GiB are free and each build needs 6 GiB. One
    // waits for memory, the other for disk, and both become runnable in one pass.
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let build = |arrived, reason| Entry {
        wait: Some(Wait::Held(reason)),
        ..entry(
            arrived,
            Peak {
                memory: 6 * GIB,
                writes: GIB,
                volume: Some(ReceivingVolume {
                    path: "/scratch/output".into(),
                    id: 1,
                }),
                volume_writes: GIB,
                exact: 0,
            },
        )
    };
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![
            (1, build(1, WaitReason::Memory)),
            (2, build(2, WaitReason::Disk)),
        ],
    );
    Ledger::new(None).pass(
        std::slice::from_ref(&current),
        &current,
        &room(8 * GIB, false, &volume),
    );
    assert_eq!(wait(&current, 1), None, "the first build did not resume");
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Held(WaitReason::Disk)),
        "the second build resumed onto the memory the first one took"
    );
}

#[test]
fn two_new_known_builds_do_not_both_start_onto_one_free_machine() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let peak = Peak {
        memory: 6 * GIB,
        writes: 0,
        exact: 0,
        ..Peak::default()
    };
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![(1, entry(1, peak.clone())), (2, entry(2, peak))],
    );
    Ledger::new(None).pass(
        std::slice::from_ref(&current),
        &current,
        &room(8 * GIB, false, &volume),
    );
    assert_eq!(wait(&current, 1), None, "the first build was held");
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Held(WaitReason::Memory)),
        "the second build started onto the memory the first one took"
    );
}

/// A sample finds `pid`. No platform reports when it started: a command is
/// tied to its line by the shell's reports alone.
fn start(ledger: &mut Ledger, member: &Arc<Mutex<Observation>>, pid: u32, resident: u64) {
    member
        .lock()
        .unwrap()
        .commands
        .push(command(pid, resident, 8 * MIB));
    ledger.register(member, 512 * MIB);
}

/// A line accepted at the terminal.
fn submit(member: &Arc<Mutex<Observation>>, holdable: bool) {
    member
        .lock()
        .unwrap()
        .submissions
        .submit(holdable, std::time::Instant::now(), &[]);
}

/// The shell reads the next accepted line, runs it and returns to the prompt;
/// `jobs` are the process groups its job table holds.
fn run_line(member: &Arc<Mutex<Observation>>, jobs: &[u32]) {
    let mut member = member.lock().unwrap();
    let now = std::time::Instant::now();
    member.submissions.line_started(now);
    member.submissions.line_finished(jobs, now);
}

/// An older heavy command of another owner: never the one paused.
fn oldest(ledger: &mut Ledger) -> Arc<Mutex<Observation>> {
    let other = owner("other", true, Vec::new(), Vec::new());
    start(ledger, &other, 4_100_001, GIB);
    other
}

/// Memory is short and `pid` of the terminal is the newest growing command.
fn pressed(
    ledger: &mut Ledger,
    other: Arc<Mutex<Observation>>,
    terminal: &Arc<Mutex<Observation>>,
) {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let members = [other, terminal.clone()];
    ledger.pass(&members, terminal, &room(100 * MIB, true, &volume));
}

#[test]
fn a_persons_job_stays_unpausable_when_an_agent_types_after_it() {
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    // An agent's job, then a person's own (the newest), then an agent types
    // at the prompt the person's job returned.
    submit(&terminal, true);
    run_line(&terminal, &[4_100_002]);
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    submit(&terminal, false);
    run_line(&terminal, &[4_100_003]);
    start(&mut ledger, &terminal, 4_100_003, 700 * MIB);
    submit(&terminal, true);
    pressed(&mut ledger, other, &terminal);
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
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    submit(&terminal, true);
    run_line(&terminal, &[4_100_002]);
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    // The person types at the prompt the agent's job returned.
    submit(&terminal, false);
    pressed(&mut ledger, other, &terminal);
    assert_eq!(
        wait(&terminal, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory)),
        "an agent's job was exempted by a person's later keystroke"
    );
}

#[test]
fn an_agents_job_first_sampled_after_a_persons_line_is_still_the_agents() {
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    // Both lines are accepted before the shell launches the build, and both
    // ran before the sampler first sees it.
    submit(&terminal, true);
    submit(&terminal, false);
    run_line(&terminal, &[4_100_002]);
    run_line(&terminal, &[4_100_002]);
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    pressed(&mut ledger, other, &terminal);
    assert_eq!(
        wait(&terminal, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory)),
        "the agent's build took the person's later line"
    );
}

#[test]
fn a_persons_job_first_sampled_after_an_agents_line_is_still_the_persons() {
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    submit(&terminal, false);
    submit(&terminal, true);
    run_line(&terminal, &[4_100_002]);
    run_line(&terminal, &[4_100_002]);
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    pressed(&mut ledger, other, &terminal);
    assert_eq!(
        wait(&terminal, 4_100_002),
        None,
        "the person's job took the agent's later line"
    );
}

#[test]
fn a_command_no_line_explains_follows_the_terminals_default() {
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    // A terminal's own shell start precedes every line: the person's.
    let terminal = owner("terminal", false, Vec::new(), Vec::new());
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    pressed(&mut ledger, other, &terminal);
    assert_eq!(wait(&terminal, 4_100_002), None);
}

#[test]
fn a_command_waits_to_be_decided_while_the_shell_has_not_reported_its_line() {
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let terminal = owner("terminal", true, Vec::new(), Vec::new());
    // The sampler finds the agent's build before the shell's start mark is read.
    submit(&terminal, true);
    start(&mut ledger, &terminal, 4_100_002, 700 * MIB);
    assert!(
        !terminal.lock().unwrap().admission.contains_key(&4_100_002),
        "a command was decided before its line was known"
    );
    terminal
        .lock()
        .unwrap()
        .submissions
        .line_started(std::time::Instant::now());
    ledger.register(&terminal, 512 * MIB);
    pressed(&mut ledger, other, &terminal);
    assert_eq!(
        wait(&terminal, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory))
    );
}

#[test]
fn owners_keep_their_turn_while_commands_resume_one_at_a_time() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let held = |arrived| Entry {
        wait: Some(Wait::Held(WaitReason::Memory)),
        ..entry(arrived, scratch(0))
    };
    let owner = |unit: &str, arrivals: &[u64]| {
        owner(
            unit,
            true,
            arrivals
                .iter()
                .map(|pid| command(*pid as u32, 0, 0))
                .collect(),
            arrivals
                .iter()
                .map(|pid| (*pid as u32, held(*pid)))
                .collect(),
        )
    };
    let a = owner("a", &[1, 2, 3]);
    let b = owner("b", &[4]);
    let members = [a.clone(), b.clone()];
    let mut ledger = Ledger::new(None);
    let mut started = Vec::new();
    for _ in 0..4 {
        unsettle(&mut ledger);
        ledger.pass(&members, &a, &room(GIB, false, &volume));
        for pid in 1..=4 {
            let member = if pid == 4 { &b } else { &a };
            if wait(member, pid).is_none() && !started.contains(&pid) {
                started.push(pid);
            }
        }
        let position =
            |member: &Arc<Mutex<Observation>>, pid| member.lock().unwrap().admission[&pid].position;
        if started == [1] {
            assert_eq!(
                (position(&b, 4), position(&a, 2), position(&a, 3)),
                (1, 2, 3),
                "the displayed line is not the order they resume in"
            );
        }
    }
    assert_eq!(started, [1, 4, 2, 3]);
}

#[test]
fn a_running_command_nobody_forced_is_not_handed_over() {
    let mut running = entry(1, scratch(0));
    running.admitted = true;
    assert!(HeldCommand::of(std::process::id(), &running).is_none());
    running.forced = true;
    assert!(HeldCommand::of(std::process::id(), &running).is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn a_running_start_now_override_cannot_pass_to_a_reused_pid() {
    let pid = std::process::id();
    let mut handed_over = HeldCommand::of(
        pid,
        &Entry {
            forced: true,
            admitted: true,
            ..Entry::default()
        },
    )
    .expect("the running Start now policy must be handed over");
    handed_over.identity.start = handed_over.identity.start.wrapping_add(1);
    adopt_admission(vec![handed_over]);
    assert!(
        take_adopted(pid).is_none(),
        "a different process birth inherited the Start now override"
    );
}

/// Five builds grow together on a machine that stays short of memory while
/// paused builds keep what they hold. Each tick every owner's sample runs one
/// pass, as the pool does.
#[test]
fn growing_builds_finish_one_after_another_under_tight_memory() {
    const BUILDS: u32 = 5;
    const MEMORY: u64 = 60 * GIB;
    const TIGHT_ABOVE: u64 = 16 * GIB;
    const PEAK: u64 = 16 * GIB;
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let mut ledger = Ledger::new(None);
    let owners = (1..=BUILDS)
        .map(|pid| owner(&format!("build-{pid}"), true, Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let mut resident = BTreeMap::<u32, u64>::new();
    for (pid, member) in (1..=BUILDS).zip(&owners) {
        submit(member, true);
        run_line(member, &[pid]);
        resident.insert(pid, GIB);
        start(&mut ledger, member, pid, GIB);
    }
    let mut finished = Vec::new();
    for _tick in 0..400 {
        for (pid, member) in (1..=BUILDS).zip(&owners) {
            if finished.contains(&pid) {
                continue;
            }
            let paused = member
                .lock()
                .unwrap()
                .admission
                .get(&pid)
                .is_some_and(|entry| entry.wait.is_some());
            let grown = if paused { 0 } else { 2 * GIB };
            let held = resident[&pid] + grown;
            resident.insert(pid, held);
            if held >= PEAK {
                member.lock().unwrap().commands.clear();
                resident.insert(pid, 0);
                finished.push(pid);
            } else {
                member.lock().unwrap().commands = vec![command(pid, held, grown)];
            }
        }
        let total = resident.values().sum::<u64>();
        for member in &owners {
            ledger.register(member, 512 * MIB);
            unsettle(&mut ledger);
            ledger.pass(
                &owners,
                member,
                &room(MEMORY.saturating_sub(total), total > TIGHT_ABOVE, &volume),
            );
        }
        if finished.len() == BUILDS as usize {
            return;
        }
    }
    let paused = (1..=BUILDS)
        .filter(|pid| !finished.contains(pid))
        .collect::<Vec<_>>();
    panic!("builds {paused:?} never finished: all paused under tight memory");
}

/// What five growing builds do in one scope that cannot hold them together,
/// tick by tick (the 2026-11-10 b3 incident: every build was paused, each
/// kept its memory and nothing ever resumed).
#[derive(Default)]
struct ScopeTrace {
    /// Builds paused on one tick, with the builds still unfinished.
    all_paused_at: Option<u64>,
    /// The first build (the oldest) was ever waiting.
    oldest_paused_at: Option<u64>,
    finished: Vec<u32>,
    /// Pauses (SIGSTOP) and resumes (SIGCONT) in order: (tick, pid).
    stops: Vec<(u64, u32)>,
    resumes: Vec<(u64, u32)>,
}

/// Whether the command waits; an ended command does not.
fn waits(member: &Arc<Mutex<Observation>>, pid: u32) -> bool {
    member
        .lock()
        .unwrap()
        .admission
        .get(&pid)
        .is_some_and(|entry| entry.wait.is_some())
}

fn builds_in_a_scope_too_small_for_them() -> ScopeTrace {
    const BUILDS: u32 = 5;
    // The pool's own rule: room is the ceiling less the reserve left for
    // the machine, and it is tight below the reserve or the growth of
    // everything that runs.
    const CEILING: u64 = 20 * GIB;
    const RESERVE: u64 = GIB;
    const PEAK: u64 = 16 * GIB;
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let mut ledger = Ledger::new(None);
    let owners = (1..=BUILDS)
        .map(|pid| owner(&format!("build-{pid}"), true, Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let mut resident = BTreeMap::<u32, u64>::new();
    for (pid, member) in (1..=BUILDS).zip(&owners) {
        submit(member, true);
        run_line(member, &[pid]);
        resident.insert(pid, GIB);
        start(&mut ledger, member, pid, GIB);
    }
    let mut trace = ScopeTrace::default();
    let mut waiting = BTreeMap::<u32, bool>::new();
    for tick in 0..400u64 {
        for (pid, member) in (1..=BUILDS).zip(&owners) {
            if trace.finished.contains(&pid) {
                continue;
            }
            let paused = waits(member, pid);
            let grown = if paused { 0 } else { 2 * GIB };
            let held = resident[&pid] + grown;
            resident.insert(pid, held);
            if held >= PEAK {
                member.lock().unwrap().commands.clear();
                resident.insert(pid, 0);
                trace.finished.push(pid);
            } else {
                member.lock().unwrap().commands = vec![command(pid, held, grown)];
            }
        }
        let total = resident.values().sum::<u64>();
        // A tick is a second: one command continues per second.
        unsettle(&mut ledger);
        for member in &owners {
            ledger.register(member, 512 * MIB);
            // Waiting commands do not count as growing.
            let growing = owners
                .iter()
                .map(|member| {
                    let member = member.lock().unwrap();
                    member
                        .commands
                        .iter()
                        .filter(|command| {
                            member
                                .admission
                                .get(&command.pid)
                                .is_none_or(|entry| entry.wait.is_none())
                        })
                        .map(|command| command.growth)
                        .sum::<u64>()
                })
                .sum::<u64>();
            let memory = (CEILING - RESERVE).saturating_sub(total);
            let room = Room {
                margin: RESERVE,
                growing,
                ..room(memory, false, &volume)
            };
            ledger.pass(&owners, member, &room);
        }
        let unfinished = (1..=BUILDS)
            .filter(|pid| !trace.finished.contains(pid))
            .collect::<Vec<_>>();
        for (pid, member) in (1..=BUILDS).zip(&owners) {
            if trace.finished.contains(&pid) {
                continue;
            }
            let now = waits(member, pid);
            match (waiting.insert(pid, now).unwrap_or(false), now) {
                (false, true) => trace.stops.push((tick, pid)),
                (true, false) => trace.resumes.push((tick, pid)),
                _ => {}
            }
        }
        if !unfinished.is_empty()
            && unfinished
                .iter()
                .all(|pid| waits(&owners[*pid as usize - 1], *pid))
        {
            trace.all_paused_at.get_or_insert(tick);
        }
        if waits(&owners[0], 1) {
            trace.oldest_paused_at.get_or_insert(tick);
        }
        if unfinished.is_empty() {
            break;
        }
    }
    trace
}

#[test]
fn builds_too_big_for_the_scope_are_never_all_paused_and_the_oldest_never_waits() {
    let trace = builds_in_a_scope_too_small_for_them();
    assert!(
        trace.all_paused_at.is_none(),
        "every unfinished build was paused on tick {:?}: nothing could finish",
        trace.all_paused_at
    );
    assert!(
        trace.oldest_paused_at.is_none(),
        "the oldest build was paused on tick {:?}",
        trace.oldest_paused_at
    );
    assert_eq!(trace.finished.len(), 5, "finished: {:?}", trace.finished);
    assert_eq!(trace.finished[0], 1, "the oldest build finishes first");
}

#[test]
fn paused_builds_resume_in_line_when_memory_frees_and_are_not_stopped_again_at_once() {
    let trace = builds_in_a_scope_too_small_for_them();
    // The builds continue in line, the one that arrived first (and made the
    // most progress) first.
    let paused = trace.stops.iter().map(|(_, pid)| *pid).collect::<Vec<_>>();
    let resumed = trace
        .resumes
        .iter()
        .map(|(_, pid)| *pid)
        .collect::<Vec<_>>();
    let mut first_pauses = Vec::new();
    for pid in &paused {
        if !first_pauses.contains(pid) {
            first_pauses.push(*pid);
        }
    }
    let mut first_resumes = Vec::new();
    for pid in &resumed {
        if !first_resumes.contains(pid) {
            first_resumes.push(*pid);
        }
    }
    let mut by_arrival = first_pauses.clone();
    by_arrival.sort();
    assert_eq!(
        first_resumes, by_arrival,
        "resumes {:?} stops {:?}",
        trace.resumes, trace.stops
    );
    // A build that just continued is not stopped on the next tick, and no
    // build is stopped more often than there are builds that can run before it.
    for (resumed_at, pid) in &trace.resumes {
        assert!(
            !trace.stops.iter().any(|(tick, stopped)| stopped == pid
                && tick <= &(resumed_at + 1)
                && tick > resumed_at),
            "build {pid} was stopped again right after it continued on tick {resumed_at}: {:?}",
            trace.stops
        );
    }
    assert!(trace.stops.len() <= 2 * 5, "pause storm: {:?}", trace.stops);
}

#[test]
fn a_paused_and_a_held_command_are_one_row_apart_from_the_kind() {
    let waiting = |wait| {
        let mut waiting = entry(
            1,
            Peak {
                memory: 6 * GIB,
                writes: 0,
                exact: 0,
                ..Peak::default()
            },
        );
        waiting.wait = Some(wait);
        waiting.position = 2;
        waiting.need = 6 * GIB;
        waiting.free = GIB;
        waiting.since_ms = 42;
        projection::state_of(&waiting)
    };
    match (
        waiting(Wait::Held(WaitReason::Memory)),
        waiting(Wait::Paused(WaitReason::Memory)),
    ) {
        (SessionProcessState::Held(held), SessionProcessState::Paused(paused)) => {
            assert_eq!(
                held, paused,
                "paused and held rows differ in more than the kind"
            );
            assert_eq!(held.command_line, "job 1");
            assert_eq!(held.cwd.as_deref(), Some("/scratch"));
            assert_eq!((held.position, held.since_ms), (2, 42));
        }
        other => panic!("rows read {other:?}"),
    }
}

/// Five builds write onto a volume that cannot hold all of them; a build that
/// ends, or is stopped for filling the disk, deletes its scratch. Each tick
/// every owner's sample runs one pass.
#[test]
fn the_disk_never_fills_up_while_five_builds_write_and_the_filler_goes() {
    const BUILDS: u32 = 5;
    const PEAK: u64 = GIB;
    const START_FREE: u64 = 4600 * MIB;
    const STEP: u64 = 8 * MIB;
    let written = Arc::new(Mutex::new(BTreeMap::<u32, u64>::new()));
    let least_free = Arc::new(AtomicU64::new(u64::MAX));
    let free = {
        let (written, least_free) = (written.clone(), least_free.clone());
        Box::new(move |_: &Path| {
            let used = written.lock().unwrap().values().sum::<u64>();
            let free = START_FREE.saturating_sub(used);
            least_free.fetch_min(free, Ordering::SeqCst);
            Some(Space {
                id: 1,
                free,
                total: 100 * GIB,
            })
        }) as Box<Volume>
    };
    let mut ledger = Ledger::new(None);
    let owners = (1..=BUILDS)
        .map(|pid| {
            let mut build = entry(pid as u64, Peak::default());
            build.expected = None;
            owner(
                &format!("build-{pid}"),
                true,
                vec![writer(pid, 0, 0)],
                vec![(pid, build)],
            )
        })
        .collect::<Vec<_>>();
    let (mut finished, mut stopped) = (Vec::new(), Vec::new());
    for _tick in 0..400 {
        for (pid, member) in (1..=BUILDS).zip(&owners) {
            if finished.contains(&pid) || stopped.contains(&pid) {
                continue;
            }
            let total = {
                let mut written = written.lock().unwrap();
                let total = written.entry(pid).or_default();
                *total += STEP;
                *total
            };
            if total >= PEAK {
                written.lock().unwrap().remove(&pid);
                member.lock().unwrap().commands.clear();
                finished.push(pid);
            } else {
                let mut observation = member.lock().unwrap();
                observation.commands = vec![writer(pid, total, STEP)];
                observation.disk_writes.insert(
                    pid,
                    vec![disk_use::VolumeWrites {
                        id: 1,
                        path: "/scratch/output".into(),
                        written: total,
                        rate: STEP,
                        files: [u64::from(pid)].into(),
                    }],
                );
            }
        }
        for member in &owners {
            ledger.register(member, 512 * MIB);
            unsettle(&mut ledger);
            ledger.pass(&owners, member, &room(GIB, false, &*free));
            for (pid, _) in ledger.take_fillers(member) {
                written.lock().unwrap().remove(&pid);
                member.lock().unwrap().commands.clear();
                stopped.push(pid);
            }
        }
        if finished.len() + stopped.len() == BUILDS as usize {
            break;
        }
    }
    assert_eq!(
        finished.len() + stopped.len(),
        BUILDS as usize,
        "builds are still running: finished {finished:?}, stopped {stopped:?}"
    );
    assert!(
        !stopped.is_empty(),
        "nothing was stopped on a disk too small for five builds"
    );
    assert!(
        !finished.is_empty(),
        "every build was stopped: the others should finish once the filler is gone"
    );
    assert!(
        least_free.load(Ordering::SeqCst) > 0,
        "the disk filled up completely"
    );
}

fn writer(pid: u32, writes: u64, write_growth: u64) -> pool::CommandUse {
    pool::CommandUse {
        pid,
        deleted_open: 0,
        group: None,
        resident: 100 * MIB,
        growth: 0,
        steady_growth: 0,
        measured_growth: 0,
        writes,
        write_growth,
        effective: pid,
        root: false,
    }
}

fn writing(pid: u32, writes: u64, write_growth: u64) -> (u32, Entry, pool::CommandUse) {
    let mut build = entry(pid as u64, Peak::default());
    build.expected = None;
    build.admitted = true;
    build.writes = writes;
    (pid, build, writer(pid, writes, write_growth))
}

#[test]
fn the_writer_filling_a_nearly_full_disk_is_stopped_never_paused() {
    // 128 MiB is below half the bounded 512 MiB reserve.
    let free = Arc::new(AtomicU64::new(128 * MIB));
    let volume = volume(&free);
    let (_, orphan, orphan_use) = writing(1, 6 * GIB, 8 * MIB);
    let (_, build, build_use) = writing(2, 100 * MIB, 50 * MIB);
    let current = owner(
        "agent",
        true,
        vec![orphan_use, build_use],
        vec![(1, orphan), (2, build)],
    );
    let mut ledger = Ledger::new(None);
    let pass = |ledger: &mut Ledger| {
        ledger.pass(
            std::slice::from_ref(&current),
            &current,
            &room(GIB, false, &*volume),
        );
    };
    pass(&mut ledger);
    let fillers = ledger.take_fillers(&current);
    assert_eq!(
        fillers.iter().map(|(pid, _)| *pid).collect::<Vec<_>>(),
        vec![1],
        "the writer that wrote the most is the filler, not the fastest or the newest"
    );
    assert_eq!(fillers[0].1.written, 6 * GIB);
    assert_eq!(wait(&current, 1), None, "the filler was paused");
    assert_eq!(
        wait(&current, 2),
        None,
        "another owner's build was paused for it"
    );
    assert!(ledger.take_fillers(&current).is_empty(), "reported twice");
    // The next one is judged only after the first gave its space back.
    pass(&mut ledger);
    assert!(
        ledger.take_fillers(&current).is_empty(),
        "stopped a second at once"
    );
}

#[test]
fn writers_above_half_the_floor_and_a_persons_command_are_left_alone() {
    let free = Arc::new(AtomicU64::new(400 * MIB));
    let volume = volume(&free);
    let (_, build, build_use) = writing(1, 6 * GIB, 50 * MIB);
    let current = owner("agent", true, vec![build_use], vec![(1, build)]);
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert!(
        ledger.take_fillers(&current).is_empty(),
        "stopped a writer above half the floor"
    );
    free.store(128 * MIB, Ordering::SeqCst);
    let (_, mut own, own_use) = writing(2, 6 * GIB, 50 * MIB);
    own.holdable = false;
    let person = owner("person", false, vec![own_use], vec![(2, own)]);
    ledger.pass(
        std::slice::from_ref(&person),
        &person,
        &room(GIB, false, &*volume),
    );
    assert!(
        ledger.take_fillers(&person).is_empty(),
        "stopped a person's own command"
    );
}

#[test]
fn a_command_that_needs_more_than_the_whole_pool_is_refused_not_held() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    let big = entry(
        1,
        Peak {
            memory: 80 * GIB,
            writes: 0,
            exact: 0,
            ..Peak::default()
        },
    );
    let fits = entry(
        2,
        Peak {
            memory: 40 * GIB,
            writes: 0,
            exact: 0,
            ..Peak::default()
        },
    );
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![(1, big), (2, fits)],
    );
    let mut ledger = Ledger::new(None);
    // 64 GiB pool: the first can never fit, the second only has to wait.
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(10 * GIB, false, &volume),
    );
    assert_eq!(ledger.take_refused(&current), vec![(1, 80 * GIB)]);
    assert!(ledger.take_refused(&current).is_empty(), "reported twice");
    assert_eq!(wait(&current, 1), None, "an impossible command was held");
    assert_eq!(wait(&current, 2), None, "the one that fits was refused");
}

#[test]
fn a_smaller_run_of_a_command_is_not_refused_for_the_peak_of_a_bigger_one() {
    let free = Arc::new(AtomicU64::new(50 * GIB));
    let volume = volume(&free);
    // `-j16` reached 80 GiB; `-j1` has the same normalized key but another exact line.
    let mut smaller = entry(
        1,
        Peak {
            memory: 80 * GIB,
            writes: 0,
            exact: 7,
            ..Peak::default()
        },
    );
    smaller.seen.as_mut().unwrap().exact = 8;
    let mut same = entry(
        2,
        Peak {
            memory: 80 * GIB,
            writes: 0,
            exact: 7,
            ..Peak::default()
        },
    );
    same.seen.as_mut().unwrap().exact = 7;
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![(1, smaller), (2, same)],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(10 * GIB, false, &volume),
    );
    assert_eq!(
        ledger.take_refused(&current),
        vec![(2, 80 * GIB)],
        "only the line that reached the peak is refused"
    );
    assert_eq!(wait(&current, 1), None, "the smaller run was held");
}

#[test]
fn a_full_volume_stops_its_own_writer_and_leaves_one_on_a_roomy_volume_alone() {
    // Volume 1 has room; volume 2 has only 128 MiB left.
    let volume: Box<Volume> = Box::new(|path: &Path| {
        let (id, free) = if path.starts_with("/roomy") {
            (1, 50 * GIB)
        } else {
            (2, 128 * MIB)
        };
        Some(Space {
            id,
            free,
            total: 100 * GIB,
        })
    });
    let (_, mut roomy, roomy_use) = writing(1, 8 * GIB, 50 * MIB);
    roomy.seen.as_mut().unwrap().cwd = Some("/roomy/build".into());
    let (_, mut full, full_use) = writing(2, GIB, 50 * MIB);
    full.seen.as_mut().unwrap().cwd = Some("/full/build".into());
    let current = owner(
        "agent",
        true,
        vec![roomy_use, full_use],
        vec![(1, roomy), (2, full)],
    );
    current.lock().unwrap().disk_writes = [
        (
            1,
            vec![disk_use::VolumeWrites {
                id: 1,
                path: "/roomy/output".into(),
                written: 8 * GIB,
                rate: 50 * MIB,
                files: [1].into(),
            }],
        ),
        (
            2,
            vec![disk_use::VolumeWrites {
                id: 2,
                path: "/full/output".into(),
                written: GIB,
                rate: 50 * MIB,
                files: [2].into(),
            }],
        ),
    ]
    .into_iter()
    .collect();
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(
        ledger.take_fillers(&current).iter().map(|(pid, _)| *pid).collect::<Vec<_>>(),
        vec![2],
        "the writer on the full volume is the filler, the older bigger one on the roomy volume is not"
    );
}

#[test]
fn cwd_on_a_full_volume_does_not_make_a_writer_elsewhere_the_filler() {
    crossed_volume_writer("/full/build", "/roomy/output", 1, false);
}

#[test]
fn cwd_on_a_roomy_volume_does_not_hide_a_writer_filling_another_volume() {
    crossed_volume_writer("/roomy/build", "/full/output", 2, true);
}

fn crossed_volume_writer(cwd: &str, output: &str, id: u64, should_stop: bool) {
    let volume: Box<Volume> = Box::new(|path: &Path| {
        let (id, free) = if path.starts_with("/roomy") {
            (1, 50 * GIB)
        } else {
            (2, 128 * MIB)
        };
        Some(Space {
            id,
            free,
            total: 100 * GIB,
        })
    });
    let (_, mut build, usage) = writing(1, 6 * GIB, 50 * MIB);
    build.seen.as_mut().unwrap().cwd = Some(cwd.into());
    let current = owner("agent", true, vec![usage], vec![(1, build)]);
    current.lock().unwrap().disk_writes.insert(
        1,
        vec![disk_use::VolumeWrites {
            id,
            files: [1].into(),
            path: output.into(),
            written: 6 * GIB,
            rate: 50 * MIB,
        }],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(
        !ledger.take_fillers(&current).is_empty(),
        should_stop,
        "the receiving volume, not cwd, decides which writer fills it"
    );
}

#[test]
fn a_shared_growing_file_is_not_attributed_to_either_command() {
    let free = Arc::new(AtomicU64::new(128 * MIB));
    let volume = volume(&free);
    let (_, first, first_use) = writing(1, 6 * GIB, 50 * MIB);
    let (_, second, second_use) = writing(2, 6 * GIB, 50 * MIB);
    let current = owner(
        "agent",
        true,
        vec![first_use, second_use],
        vec![(1, first), (2, second)],
    );
    for outputs in current.lock().unwrap().disk_writes.values_mut() {
        outputs[0].files = [42].into();
    }
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert!(
        ledger.take_fillers(&current).is_empty(),
        "shared file growth has no unambiguous writer"
    );
}

#[test]
fn unknown_or_reused_output_descriptors_cannot_justify_a_disk_stop() {
    let free = Arc::new(AtomicU64::new(128 * MIB));
    let volume = volume(&free);
    for reused in [false, true] {
        let (_, build, usage) = writing(1, 6 * GIB, 50 * MIB);
        let current = owner("agent", true, vec![usage], vec![(1, build)]);
        if reused {
            current.lock().unwrap().disk_writes.get_mut(&1).unwrap()[0].id = 2;
        } else {
            current.lock().unwrap().disk_writes.clear();
        }
        let mut ledger = Ledger::new(None);
        ledger.pass(
            std::slice::from_ref(&current),
            &current,
            &room(GIB, false, &*volume),
        );
        assert!(
            ledger.take_fillers(&current).is_empty(),
            "missing or changed output volume supplied destructive evidence"
        );
    }
}

#[test]
fn a_later_disk_wait_resumes_when_only_its_volume_recovers() {
    let recovered = Arc::new(AtomicU64::new(128 * MIB));
    let first_free = Arc::new(AtomicU64::new(128 * MIB));
    let first_space = first_free.clone();
    let free = recovered.clone();
    let volume: Box<Volume> = Box::new(move |path| {
        let other = path.starts_with("/other");
        Some(Space {
            id: if other { 2 } else { 1 },
            free: if other {
                free.load(Ordering::SeqCst)
            } else {
                first_space.load(Ordering::SeqCst)
            },
            total: 100 * GIB,
        })
    });
    let held = |arrived| Entry {
        wait: Some(Wait::Held(WaitReason::Disk)),
        ..entry(arrived, scratch(6 * GIB))
    };
    let mut later = held(2);
    later.expected.as_mut().unwrap().volume = Some(ReceivingVolume {
        path: "/other/output".into(),
        id: 2,
    });
    let first = owner("first", true, vec![command(1, 0, 0)], vec![(1, held(1))]);
    let second = owner(
        "second",
        true,
        vec![command(2, 0, 0), command(3, 0, 0)],
        vec![(2, later), (3, held(3))],
    );
    let members = [first.clone(), second.clone()];
    let mut ledger = Ledger::new(None);
    ledger.pass(&members, &first, &room(GIB, false, &*volume));
    assert!(wait(&first, 1).is_some());
    assert!(wait(&second, 2).is_some());
    recovered.store(18 * GIB, Ordering::SeqCst);
    ledger.pass(&members, &first, &room(GIB, false, &*volume));
    assert_eq!(
        wait(&second, 2),
        None,
        "another volume blocked a runnable writer"
    );
    assert_eq!(wait(&first, 1), Some(Wait::Held(WaitReason::Disk)));
    assert_eq!(wait(&second, 3), Some(Wait::Held(WaitReason::Disk)));
    // Skipping an ineligible owner must not consume its next fair turn.
    first_free.store(18 * GIB, Ordering::SeqCst);
    unsettle(&mut ledger);
    ledger.pass(&members, &first, &room(GIB, false, &*volume));
    assert_eq!(wait(&first, 1), None);
    assert!(wait(&second, 3).is_some());
}

#[test]
fn a_known_writer_waits_for_the_volume_its_output_goes_to_not_the_checkout_volume() {
    // The checkout (/scratch) is below its floor; the build wrote its 6 GiB
    // to a roomy volume under /data last time.
    let checkout = Arc::new(AtomicU64::new(128 * MIB));
    let output = Arc::new(AtomicU64::new(40 * GIB));
    let (checkout_space, output_space) = (checkout.clone(), output.clone());
    let volume: Box<Volume> = Box::new(move |path| {
        let data = path.starts_with("/data");
        Some(Space {
            id: if data { 2 } else { 1 },
            free: if data { &output_space } else { &checkout_space }.load(Ordering::SeqCst),
            total: 100 * GIB,
        })
    });
    let build = |arrived, volume: &str| {
        let mut build = entry(arrived, scratch(6 * GIB));
        build.expected.as_mut().unwrap().volume = Some(ReceivingVolume {
            path: volume.into(),
            id: if volume.starts_with("/data") { 2 } else { 1 },
        });
        build
    };
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0), command(2, 0, 0)],
        vec![
            (1, build(1, "/data/target/out.o")),
            (2, build(2, "/scratch/out.o")),
        ],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(
        wait(&current, 1),
        None,
        "a build whose output goes to a roomy volume was held for its full checkout volume"
    );
    assert_eq!(
        wait(&current, 2),
        Some(Wait::Held(WaitReason::Disk)),
        "a build whose output goes to the full volume started"
    );
    // The checkout volume recovering releases the one that writes there.
    checkout.store(30 * GIB, Ordering::SeqCst);
    unsettle(&mut ledger);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(wait(&current, 2), None);
}

#[test]
fn a_known_writer_whose_output_volume_was_never_seen_is_not_held_for_disk() {
    let free = Arc::new(AtomicU64::new(GIB));
    let volume = volume(&free);
    let mut unseen = entry(1, scratch(6 * GIB));
    unseen.expected.as_mut().unwrap().volume = None;
    let current = owner("agent", true, vec![command(1, 0, 0)], vec![(1, unseen)]);
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(wait(&current, 1), None);
}

#[test]
fn a_finished_run_remembers_the_volume_that_received_most_of_its_writes() {
    let output = std::env::temp_dir().join(format!("remembered-output-{}", std::process::id()));
    std::fs::write(&output, b"output").unwrap();
    let id = statvfs(&output).unwrap().id;
    let (_, build, usage) = writing(1, 6 * GIB, 50 * MIB);
    let current = owner("agent", true, vec![usage], vec![(1, build)]);
    {
        let mut observation = current.lock().unwrap();
        let outputs = observation.disk_writes.get_mut(&1).unwrap();
        outputs[0].path = output.clone();
        outputs[0].id = id;
        outputs.push(disk_use::VolumeWrites {
            id: 2,
            path: "/data/target/out.o".into(),
            written: 5 * GIB,
            rate: 50 * MIB,
            files: [9].into(),
        });
    }
    let mut ledger = Ledger::new(None);
    ledger.register(&current, 512 * MIB);
    current.lock().unwrap().commands.clear();
    ledger.register(&current, 512 * MIB);
    let peak = ledger
        .peaks
        .get("key-1")
        .expect("the heavy run was remembered");
    assert_eq!(
        peak.volume,
        Some(ReceivingVolume {
            path: std::fs::canonicalize(output.parent().unwrap()).unwrap(),
            id
        })
    );
    assert_eq!(peak.volume_writes, 6 * GIB);
    std::fs::remove_file(output).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_sampled_writer_can_exit_and_rerun_on_its_healthy_output_volume() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let scratch = std::env::temp_dir().join(format!("writer-volume-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let output = scratch.join("output");
    let peaks = scratch.join("peaks.json");
    let mut launch = Command::new("python3");
    launch
        .args([
            "-I",
            "-c",
            "import os,sys
f=open(sys.argv[1], 'wb')
print('ready',flush=True)
sys.stdin.readline()
f.write(b'x' * 2097152)
f.flush()
os.fsync(f.fileno())
print('written',flush=True)
sys.stdin.readline()",
            output.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut writer = launch.spawn().unwrap();
    let pid = writer.id();
    let mut stdout = BufReader::new(writer.stdout.take().unwrap());
    let mut stdin = writer.stdin.take().unwrap();
    let mut ready = String::new();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "ready");
    let seen = recognise(pid).unwrap();
    let key = seen.key.clone();
    let current = owner(
        "agent",
        true,
        vec![command(pid, 0, 0)],
        vec![(
            pid,
            Entry {
                arrived: 1,
                holdable: true,
                seen: Some(seen),
                ..Entry::default()
            },
        )],
    );
    let mut growth = disk_use::WriteGrowth::default();
    growth.sample(&[pid], 1000);
    writeln!(stdin).unwrap();
    ready.clear();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "written");
    let outputs = growth.sample(&[pid], 2000);
    let written = outputs.iter().map(|output| output.written).sum::<u64>();
    assert!(written >= 2 * MIB);
    current.lock().unwrap().disk_writes.insert(pid, outputs);
    let mut ledger = Ledger::new(Some(peaks.clone()));
    current.lock().unwrap().commands[0].writes = written;
    ledger.register(&current, MIB);
    writeln!(stdin).unwrap();
    assert!(writer.wait().unwrap().success());
    // Build outputs are often removed after the writer exits, too.
    std::fs::remove_file(&output).unwrap();
    current.lock().unwrap().commands.clear();
    ledger.register(&current, MIB);
    let mut ledger = Ledger::new(Some(peaks));
    let peak = ledger
        .peaks
        .get(&key)
        .expect("the sampled writer is remembered across restart");
    let mut rerun = launch.spawn().unwrap();
    let rerun_pid = rerun.id();
    let mut stdout = BufReader::new(rerun.stdout.take().unwrap());
    let mut stdin = rerun.stdin.take().unwrap();
    ready.clear();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "ready");
    let seen = recognise(rerun_pid).unwrap();
    assert_eq!(
        seen.key, key,
        "the exact command reruns in the same directory"
    );
    let mut next = entry(2, peak);
    next.seen = Some(seen);
    let next = owner(
        "rerun",
        true,
        vec![command(rerun_pid, 0, 0)],
        vec![(rerun_pid, next)],
    );
    // The actual filesystem is healthy. Procfs reports zero available space.
    assert!(statvfs(&scratch).unwrap().free > DISK_FLOOR + written);
    let mut room = room(GIB, false, &statvfs);
    room.light = MIB;
    ledger.register(&next, MIB);
    ledger.pass(std::slice::from_ref(&next), &next, &room);
    let held = wait(&next, rerun_pid);
    next.lock()
        .unwrap()
        .admission
        .get_mut(&rerun_pid)
        .unwrap()
        .wait = Some(Wait::Held(WaitReason::Disk));
    unsettle(&mut ledger);
    ledger.pass(std::slice::from_ref(&next), &next, &room);
    let still_held = wait(&next, rerun_pid);
    // Settle the real child even when either regression assertion fails.
    writeln!(stdin).unwrap();
    ready.clear();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "written");
    writeln!(stdin).unwrap();
    assert!(rerun.wait().unwrap().success());
    std::fs::remove_dir_all(scratch).unwrap();
    assert_eq!(
        held, None,
        "the exited writer's procfs descriptor must not hold its rerun"
    );
    assert_eq!(
        still_held, None,
        "a disk wait must also recover on the healthy output volume"
    );
}

#[test]
fn a_remembered_destination_on_a_replaced_volume_is_unknown_for_hold_and_resume() {
    let replaced: Box<Volume> = Box::new(|_| {
        Some(Space {
            id: 2,
            free: GIB,
            total: 100 * GIB,
        })
    });
    let next = owner(
        "agent",
        true,
        vec![command(1, 0, 0)],
        vec![(1, entry(1, scratch(6 * GIB)))],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&next),
        &next,
        &room(GIB, false, &*replaced),
    );
    assert_eq!(
        wait(&next, 1),
        None,
        "a replacement filesystem cannot justify a hold"
    );
    next.lock().unwrap().admission.get_mut(&1).unwrap().wait = Some(Wait::Held(WaitReason::Disk));
    unsettle(&mut ledger);
    ledger.pass(
        std::slice::from_ref(&next),
        &next,
        &room(GIB, false, &*replaced),
    );
    assert_eq!(
        wait(&next, 1),
        None,
        "a disk wait with an unresolved volume must resume"
    );
}

#[test]
fn unresolved_sampled_destinations_are_not_remembered_as_a_receiving_volume() {
    let output = std::env::temp_dir().join(format!("unresolved-volume-{}", std::process::id()));
    std::fs::write(&output, b"output").unwrap();
    let id = statvfs(&output).unwrap().id;
    let sample = disk_use::VolumeWrites {
        id: id.wrapping_add(1),
        path: output.clone(),
        written: GIB,
        rate: MIB,
        files: Default::default(),
    };
    assert_eq!(
        ReceivingVolume::capture(&sample),
        None,
        "a destination resolved onto a different device is unknown"
    );
    std::fs::remove_file(output).unwrap();
    assert_eq!(
        ReceivingVolume::capture(&sample),
        None,
        "an unresolved descriptor is unknown"
    );
}

#[test]
fn numeric_output_paths_do_not_reuse_another_commands_receiving_volume() {
    // Normalization gives output-1 and output-2 the same memory peak key.
    assert_eq!(normalize("/mnt/output-1"), normalize("/mnt/output-2"));
    let volume: Box<Volume> = Box::new(|path| {
        Some(Space {
            id: if path.starts_with("/mnt/output-1") {
                1
            } else {
                2
            },
            free: if path.starts_with("/mnt/output-1") {
                GIB
            } else {
                40 * GIB
            },
            total: 100 * GIB,
        })
    });
    let mut build = entry(1, scratch(6 * GIB));
    build.expected.as_mut().unwrap().exact = 1;
    build.expected.as_mut().unwrap().volume = Some(ReceivingVolume {
        path: "/mnt/output-1".into(),
        id: 1,
    });
    build.seen.as_mut().unwrap().exact = 2;
    build.seen.as_mut().unwrap().line = "cargo build --target-dir /mnt/output-2".into();
    let current = owner("agent", true, vec![command(1, 0, 0)], vec![(1, build)]);
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(
        wait(&current, 1),
        None,
        "a different exact command's output destination is unknown"
    );
}

/// A paused command that only the room to keep growing holds back (the memory
/// room covers what it needs, but everything running grows too fast for it to
/// grow ahead) is ended with that reason, not with memory figures that say it
/// fits.
#[test]
fn a_pause_only_the_room_to_continue_keeps_does_not_name_a_memory_shortage() {
    let volume = volume(&Arc::new(AtomicU64::new(100 * GIB)));
    let launches = BTreeMap::from([(
        "binary".to_string(),
        super::super::launch::Reservation {
            memory: GIB,
            writes: 0,
            volume: 1,
        },
    )]);
    let room = Room {
        margin: GIB,
        growing: 512 * MIB,
        launches: &launches,
        ..room(3 * GIB, false, &volume)
    };
    let mut paused = entry(1, Peak::default());
    paused.admitted = true;
    paused.wait = Some(Wait::Paused(WaitReason::Memory));
    paused.since_ms = crate::session::unix_epoch_ms() - 61_000;
    paused.paused_rate = 600 * MIB;
    let member = owner("a", true, vec![command(1, 600 * MIB, 0)], vec![(1, paused)]);
    let (control, _heard) = std::sync::mpsc::channel();
    member.lock().unwrap().control = Some(control);

    let (_, blocked) =
        blocked_waiting(&[member], 60_000, &room).expect("the pause cannot continue");
    // 512 MiB/s running plus its own 600 MiB/s: 1 GiB margin or the rate, and two seconds ahead.
    let growing = 512 * MIB + 600 * MIB;
    let Blocked::Growth {
        left,
        need,
        growing: reported,
    } = blocked
    else {
        panic!("the memory room covers its need; the growth is what it lacks");
    };
    assert_eq!((left, reported), (3 * GIB, growing));
    assert_eq!(need, growing + 2 * growing);
    let line = super::super::waits::overdue_line(60_000, &blocked, 600 * MIB, Some(64 * GIB));
    assert!(
        line.contains("no room to keep growing") && !line.contains("still needs"),
        "{line}"
    );
}

#[test]
fn a_pause_uses_the_pre_pause_window_and_ignores_stopped_samples() {
    let volume = volume(&Arc::new(AtomicU64::new(100 * GIB)));
    let mut ledger = Ledger::new(None);
    let other = oldest(&mut ledger);
    let current = owner("grower", true, Vec::new(), Vec::new());
    start(&mut ledger, &current, 4_100_002, 700 * MIB);
    let mut window = super::super::growth_window::Window::default();
    for (at, rate) in [4 * MIB, 4 * MIB, 4 * MIB, 1200 * MIB]
        .into_iter()
        .enumerate()
    {
        window.record(at as u64 * 250, rate);
    }
    {
        let mut current = current.lock().unwrap();
        current.commands[0].growth = 1200 * MIB;
        current.commands[0].steady_growth = window.typical().unwrap();
    }
    let members = [other, current.clone()];
    ledger.pass(&members, &current, &room(100 * MIB, true, &volume));
    assert_eq!(
        wait(&current, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory))
    );
    assert_eq!(
        current.lock().unwrap().admission[&4_100_002].paused_rate,
        4 * MIB
    );
    current.lock().unwrap().commands[0].growth = 0;
    ledger.pass(&members, &current, &room(100 * MIB, false, &volume));
    assert_eq!(
        wait(&current, 4_100_002),
        Some(Wait::Paused(WaitReason::Memory))
    );
    assert_eq!(
        current.lock().unwrap().admission[&4_100_002].paused_rate,
        4 * MIB
    );
    unsettle(&mut ledger);
    ledger.pass(&members, &current, &room(2 * GIB, false, &volume));
    assert_eq!(
        wait(&current, 4_100_002),
        None,
        "the startup spike kept a slow grower waiting"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_pause_resumes_when_physical_room_returns_despite_full_rss_accounting() {
    let pool = pool::Pool::new(1536 * MIB, true).with_machine_pressure(Box::new(|| false));
    let command_of = |unit: &str, pid: u32, resident: u64, paused: bool| {
        let mut entry = entry(pid as u64, Peak::default());
        entry.admitted = true;
        entry.expected = None;
        if paused {
            entry.wait = Some(Wait::Paused(WaitReason::Memory));
            entry.paused_rate = 700 * MIB;
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
        let member = owner(
            unit,
            true,
            vec![command(pid, resident, 0)],
            vec![(pid, entry)],
        );
        let (control, heard) = std::sync::mpsc::channel();
        {
            let mut member = member.lock().unwrap();
            member.samples = vec![sample(resident)];
            member.control = Some(control);
        }
        pool.join(&member);
        (member, heard)
    };
    let (running, _running_heard) = command_of("a", 1, 800 * MIB, false);
    let (paused, heard) = command_of("b", 2, 700 * MIB, true);
    pool.enforce(&running, None);
    assert_eq!(
        wait(&paused, 2),
        None,
        "RSS accounting stranded a pause after physical memory returned"
    );
    assert!(
        !heard
            .try_iter()
            .any(|command| matches!(command, WatchCommand::MemoryStop { .. })),
        "a healthy machine ended a paused command on RSS accounting alone"
    );
}

#[test]
fn an_old_allocation_rate_cannot_end_a_command_that_just_paused() {
    let pool = pool::Pool::new(1536 * MIB, false);
    let mut running = command(4_100_001, 800 * MIB, 16 * MIB);
    running.measured_growth = 0;
    let paused = command(4_100_002, 700 * MIB, 48 * MIB);
    let entries = [(4_100_001, false), (4_100_002, true)]
        .into_iter()
        .map(|(pid, paused)| {
            let mut entry = entry(pid as u64, Peak::default());
            entry.admitted = true;
            entry.expected = None;
            if paused {
                entry.wait_for(Wait::Paused(WaitReason::Memory));
                entry.paused_rate = 48 * MIB;
            }
            (pid, entry)
        })
        .collect();
    let member = owner("a", true, vec![running, paused], entries);
    let (control, heard) = std::sync::mpsc::channel();
    {
        let mut observation = member.lock().unwrap();
        observation.samples = vec![sample(1500 * MIB)];
        observation.control = Some(control);
    }
    pool.join(&member);
    pool.enforce(&member, None);
    assert!(matches!(wait(&member, 4_100_002), Some(Wait::Paused(_))));
    assert!(
        !heard
            .try_iter()
            .any(|command| matches!(command, WatchCommand::MemoryStop { .. })),
        "a decaying rate was counted as continued allocation after the pause"
    );
}

#[test]
fn resset_healthy_host_never_holds_or_refuses_predicted_memory() {
    let volume = volume(&Arc::new(AtomicU64::new(100 * GIB)));
    let current = owner(
        "agent",
        true,
        vec![command(1, GIB, 0), command(2, 0, 0), command(3, 0, 0)],
        vec![
            (
                1,
                entry(
                    1,
                    Peak {
                        memory: GIB,
                        ..Peak::default()
                    },
                ),
            ),
            (
                2,
                entry(
                    2,
                    Peak {
                        memory: 8 * GIB,
                        ..Peak::default()
                    },
                ),
            ),
            (
                3,
                entry(
                    3,
                    Peak {
                        memory: 80 * GIB,
                        ..Peak::default()
                    },
                ),
            ),
        ],
    );
    let mut ledger = Ledger::new(None);
    let mut healthy = room(GIB, false, &*volume);
    healthy.pressured = false;
    ledger.pass(std::slice::from_ref(&current), &current, &healthy);
    assert_eq!(
        wait(&current, 2),
        None,
        "predictions must not stop healthy work"
    );
    assert!(
        ledger.take_refused(&current).is_empty(),
        "predictions must not kill healthy work"
    );
}

#[test]
fn resset_disk_with_gibibytes_free_does_not_hold_a_predicted_writer() {
    let volume = volume(&Arc::new(AtomicU64::new(4 * GIB)));
    let current = owner(
        "agent",
        true,
        vec![command(1, 0, 0)],
        vec![(1, entry(1, scratch(8 * GIB)))],
    );
    let mut ledger = Ledger::new(None);
    ledger.pass(
        std::slice::from_ref(&current),
        &current,
        &room(GIB, false, &*volume),
    );
    assert_eq!(wait(&current, 1), None);
}

#[test]
fn resset_memory_off_releases_all_waits_and_never_refuses_or_pauses() {
    let volume = volume(&Arc::new(AtomicU64::new(100 * GIB)));
    let mut held = entry(
        1,
        Peak {
            memory: 80 * GIB,
            ..Peak::default()
        },
    );
    held.wait = Some(Wait::Held(WaitReason::Memory));
    let mut paused = entry(2, Peak::default());
    paused.wait = Some(Wait::Paused(WaitReason::Memory));
    let current = owner(
        "agent",
        true,
        vec![
            command(1, GIB, 0),
            command(2, GIB, 128 * MIB),
            command(3, GIB, 128 * MIB),
        ],
        vec![
            (1, held),
            (2, paused),
            (
                3,
                entry(
                    3,
                    Peak {
                        memory: 80 * GIB,
                        ..Peak::default()
                    },
                ),
            ),
        ],
    );
    let mut room = room(0, true, &*volume);
    room.memory_protection = false;
    let mut ledger = Ledger::new(None);
    for _ in 0..3 {
        ledger.pass(std::slice::from_ref(&current), &current, &room);
    }
    for pid in 1..=3 {
        assert_eq!(wait(&current, pid), None);
    }
    assert!(ledger.take_refused(&current).is_empty());
}

#[test]
fn resset_disk_off_releases_waits_and_never_stops_a_filler() {
    let volume = volume(&Arc::new(AtomicU64::new(1)));
    let (_, mut build, usage) = writing(1, 6 * GIB, 50 * MIB);
    build.wait = Some(Wait::Held(WaitReason::Disk));
    let current = owner("agent", true, vec![usage], vec![(1, build)]);
    let mut room = room(GIB, false, &*volume);
    room.disk_protection = false;
    let mut ledger = Ledger::new(None);
    for _ in 0..3 {
        ledger.pass(std::slice::from_ref(&current), &current, &room);
    }
    assert_eq!(wait(&current, 1), None);
    assert!(ledger.take_fillers(&current).is_empty());
}
