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
        host_bound: false,
        tight,
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
        pressure.lock().unwrap().samples = vec![sample(7 * GIB + 256 * MIB)];
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

/// A command paused on memory that the disk, not the memory, keeps from
/// continuing (its known writes do not fit above the floor) ends after 60 s
/// with the disk as the reason: the notice does not print memory figures that
/// did not decide it, such as a memory room larger than the need.
#[test]
fn a_pause_the_disk_keeps_ends_naming_the_disk() {
    // 2 GiB above the floor; the paused command writes 6 GiB.
    let pool = pool::Pool::with_volume(1500 * MIB, volume(&Arc::new(AtomicU64::new(12 * GIB))));
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
        notice.contains("of disk") && !notice.contains("still needs"),
        "the disk kept it, but the notice names memory: {notice}"
    );
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
    const PEAK: u64 = 10 * GIB;
    const START_FREE: u64 = 46 * GIB;
    const STEP: u64 = GIB / 2;
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
    // A 100 GiB volume has a 10 GiB floor: 4 GiB is below its half.
    let free = Arc::new(AtomicU64::new(4 * GIB));
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
    let free = Arc::new(AtomicU64::new(9 * GIB));
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
    free.store(2 * GIB, Ordering::SeqCst);
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
    // Volume 1 has room; volume 2 (a 100 GiB volume with a 10 GiB floor) is below half its floor.
    let volume: Box<Volume> = Box::new(|path: &Path| {
        let (id, free) = if path.starts_with("/roomy") {
            (1, 50 * GIB)
        } else {
            (2, 4 * GIB)
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
            (2, 4 * GIB)
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
    let free = Arc::new(AtomicU64::new(4 * GIB));
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
    let free = Arc::new(AtomicU64::new(4 * GIB));
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
    let recovered = Arc::new(AtomicU64::new(10 * GIB));
    let first_free = Arc::new(AtomicU64::new(10 * GIB));
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
    let checkout = Arc::new(AtomicU64::new(GIB));
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
