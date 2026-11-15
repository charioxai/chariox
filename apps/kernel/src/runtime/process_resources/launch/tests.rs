use super::*;
use crate::runtime::process_resources::{Observation, ProcessSample};
use std::sync::Mutex;

fn sample(memory: u64) -> ProcessSample {
    ProcessSample {
        process_id: "sample".into(),
        pid: 42,
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

fn learn(admission: &LaunchAdmission, argv: &[String], cwd: &Path, memory: u64) {
    let key = admission::seen_argv(0, argv, Some(cwd.to_path_buf())).key;
    admission.0.ledger.lock().unwrap().peaks.remember(
        &key,
        admission::Peak {
            memory,
            writes: 0,
            ..Default::default()
        },
        admission::light(admission.0.ceiling),
    );
}

#[test]
fn running_binary_consumes_its_reservation_without_being_charged_twice() {
    for consumed_gib in [0, 2, 4, 5] {
        let admission = LaunchAdmission::limited(10 * 1024 * MIB, u64::MAX);
        let argv = vec!["binary".into()];
        let cwd = Path::new("/");
        learn(&admission, &argv, cwd, 4 * 1024 * MIB);
        let _first = admission.reserve("first", &argv, cwd, false).unwrap();
        let member = Arc::new(Mutex::new(Observation {
            unit: "first".into(),
            samples: vec![sample(consumed_gib * 1024 * MIB)],
            ..Observation::default()
        }));
        admission.0.join(&member);
        let _second = admission
            .reserve("second", &argv, cwd, false)
            .unwrap_or_else(|wait| {
                panic!("a binary consuming {consumed_gib} GiB was charged twice: {wait:?}")
            });
        assert!(
            admission.reserve("third", &argv, cwd, false).is_err(),
            "unconsumed reservations must stay atomic"
        );
    }
}

#[test]
fn completed_binary_peak_survives_cache_reload_and_holds_the_same_command() {
    let dir = std::env::temp_dir().join(format!("binary-peak-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let cache = dir.join("admission-peaks.json");
    let admission = LaunchAdmission::limited(512 * MIB, u64::MAX);
    *admission.0.ledger.lock().unwrap() = admission::Ledger::new(Some(cache.clone()));
    let argv = vec![
        "python3".into(),
        "-c".into(),
        "import time; b=bytearray(96*1048576); print('ready', flush=True); time.sleep(1)".into(),
    ];
    let mut permit = admission.reserve("learn", &argv, &dir, false).unwrap();
    let prepared = permit.prepare().unwrap();
    let (program, args) = prepared.command(argv[0].clone(), argv[1..].to_vec());
    let mut child = std::process::Command::new(program)
        .args(args)
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    domain.bind_key(permit.id());
    use std::io::BufRead;
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while permit.peak.memory < 96 * MIB {
        assert!(
            std::time::Instant::now() < deadline,
            "binary peak was not sampled"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
        permit.observe();
    }
    assert!(child.wait().unwrap().success());
    drop(domain);
    drop(permit);
    let reloaded = LaunchAdmission::limited(512 * MIB, u64::MAX);
    *reloaded.0.ledger.lock().unwrap() = admission::Ledger::new(Some(cache));
    let pressure = Arc::new(Mutex::new(Observation {
        unit: "other".into(),
        samples: vec![sample(420 * MIB)],
        ..Observation::default()
    }));
    reloaded.0.join(&pressure);
    let wait = reloaded
        .reserve("same-command", &argv, &dir, false)
        .err()
        .expect("reloaded binary must wait for its learned peak");
    assert_eq!(wait.reason, admission::WaitReason::Memory);
    assert!(wait.need_bytes >= 96 * MIB);
    assert!(
        wait.free_bytes > 16 * MIB,
        "unknown command would have fitted"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn running_binary_consumes_the_unit_charge_shared_by_its_process_leaf() {
    let admission = LaunchAdmission::limited(512 * MIB, u64::MAX);
    let argv = vec![
        "python3".into(),
        "-c".into(),
        "import time; b=bytearray(128*1048576); print('ready', flush=True); time.sleep(1)".into(),
    ];
    let cwd = Path::new("/");
    learn(&admission, &argv, cwd, 200 * MIB);
    let mut first = admission.reserve("first", &argv, cwd, false).unwrap();
    let prepared = first.prepare().unwrap();
    let (program, args) = prepared.command(argv[0].clone(), argv[1..].to_vec());
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    domain.bind_key(first.id());
    use std::io::BufRead;
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while first.peak.memory < 128 * MIB {
        assert!(
            std::time::Instant::now() < deadline,
            "binary peak was not sampled"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
        first.observe();
    }
    let second = admission.reserve("second", &argv, cwd, false);
    assert!(child.wait().unwrap().success());
    drop(domain);
    assert!(
        second.is_ok(),
        "the process leaf did not credit its unit's charge: {:?}",
        second.err()
    );
}

fn descendant(unit: &str, pid: u32, admitted: bool) -> Arc<Mutex<Observation>> {
    let resident = 16 * MIB;
    Arc::new(Mutex::new(Observation {
        unit: unit.into(),
        samples: vec![sample(resident)],
        commands: vec![super::super::pool::CommandUse {
            pid,
            group: None,
            resident,
            growth: 0,
            steady_growth: 0,
            measured_growth: 0,
            writes: 0,
            write_growth: 0,
            effective: pid,
            root: false,

            deleted_open: 0,
        }],
        admission: [(
            pid,
            admission::Entry::predicted(
                pid,
                admission::Peak {
                    memory: 4 * 1024 * MIB,
                    writes: 0,

                    ..Default::default()
                },
                admitted,
                Path::new("/"),
            ),
        )]
        .into(),
        ..Observation::default()
    }))
}

#[test]
fn binary_descendant_prediction_is_reserved_once_at_launch_admission() {
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, u64::MAX);
    let argv = vec!["parent".into()];
    learn(&admission, &argv, Path::new("/"), 4 * 1024 * MIB);
    let _first = admission
        .reserve("first", &argv, Path::new("/"), false)
        .unwrap();
    let child = descendant("first", 42, true);
    admission.0.join(&child);
    let _second = admission
        .reserve("second", &argv, Path::new("/"), false)
        .unwrap_or_else(|wait| panic!("descendant growth charged twice: {wait:?}"));
    assert!(admission
        .reserve("third", &argv, Path::new("/"), false)
        .is_err());
}

#[test]
fn binary_descendant_prediction_is_reserved_once_during_enforcement() {
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, u64::MAX);
    let argv = vec!["parent".into()];
    learn(&admission, &argv, Path::new("/"), 4 * 1024 * MIB);
    let _first = admission
        .reserve("first", &argv, Path::new("/"), false)
        .unwrap();
    let child = descendant("first", 42, true);
    let other = descendant("other", 84, false);
    admission.0.join(&child);
    admission.0.join(&other);
    admission.0.enforce(&other, None);
    assert!(
        other.lock().unwrap().admission[&84].wait.is_none(),
        "enforcement charged the child's prediction twice"
    );
    let third = descendant("third", 126, false);
    admission.0.join(&third);
    admission.0.enforce(&third, None);
    assert!(
        third.lock().unwrap().admission[&126].wait.is_some(),
        "unrelated command reservations must still count"
    );
}

#[test]
fn running_writer_consumes_disk_prediction_before_free_space_is_sampled() {
    use std::sync::atomic::AtomicU64;
    let floor = 10 * 1024 * MIB;
    let free = Arc::new(AtomicU64::new(floor + 400 * MIB));
    let sampled = free.clone();
    let volume_id = admission::statvfs(&std::env::temp_dir()).unwrap().id;
    let admission = LaunchAdmission(Arc::new(Pool::with_volume(
        1024 * MIB,
        Box::new(move |_| {
            Some(admission::Space {
                id: volume_id,
                free: sampled.load(Ordering::SeqCst),
                total: 100 * 1024 * MIB,
            })
        }),
    )));
    let dir = std::env::temp_dir().join(format!("binary-writes-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    // The fsync makes the writes reach storage: macOS counts only those, Linux counts them as the pages are dirtied.
    let argv = vec!["/usr/bin/python3".into(), "-c".into(), "import os,sys,time; f=open('output','wb'); print('ready',flush=True); [(f.write(b'x'*(10*1048576)),f.flush(),os.fsync(f.fileno()),time.sleep(0.1)) for _ in range(20)]; sys.stdin.readline()".into()];
    let second = vec!["second-writer".into()];
    {
        let mut ledger = admission.0.ledger.lock().unwrap();
        for (command, writes) in [(&argv, 300 * MIB), (&second, 80 * MIB)] {
            let key = admission::seen_argv(0, command, Some(dir.clone())).key;
            ledger.peaks.remember(
                &key,
                admission::Peak {
                    memory: 64 * MIB,
                    writes,
                    exact: admission::seen_argv(0, command, Some(dir.clone())).exact,
                    volume: Some(admission::ReceivingVolume {
                        path: dir.clone(),
                        id: volume_id,
                    }),
                    volume_writes: writes,
                },
                admission::light(admission.0.ceiling),
            );
        }
    }
    let mut first = admission.reserve("writer", &argv, &dir, false).unwrap();
    let prepared = first.prepare().unwrap();
    let (program, args) = prepared.command(argv[0].clone(), argv[1..].to_vec());
    let mut child = std::process::Command::new(program)
        .args(args)
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    domain.bind_key(first.id());
    use std::io::{BufRead, Write};
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while first.reserved_volume_written < 100 * MIB {
        assert!(
            std::time::Instant::now() < deadline,
            "writer was not sampled"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
        first.observe();
    }
    free.store(
        floor + 300 * MIB - first.reserved_volume_written + 80 * MIB,
        Ordering::SeqCst,
    );
    let result = admission.reserve("second", &second, &dir, false);
    child.stdin.take().unwrap().write_all(b"done\n").unwrap();
    assert!(child.wait().unwrap().success());
    drop(domain);
    drop(first);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        result.is_ok(),
        "already-written output was charged twice: {:?}",
        result.err()
    );
}

fn writer_entry(unit: &str, pid: u32, writes: u64, admitted: bool) -> Arc<Mutex<Observation>> {
    let member = descendant(unit, pid, admitted);
    member.lock().unwrap().admission.insert(
        pid,
        admission::Entry::predicted(
            pid,
            admission::Peak {
                memory: 0,
                writes,
                volume: Some(admission::ReceivingVolume {
                    path: "/".into(),
                    id: 1,
                }),
                volume_writes: writes,
                ..Default::default()
            },
            admitted,
            Path::new("/"),
        ),
    );
    member
}

#[test]
fn healthy_disk_predictions_do_not_hold_observed_commands() {
    let floor = 10 * 1024 * MIB;
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, floor + 15 * 1024 * MIB);
    let argv = vec!["writer".into()];
    let key = admission::seen_argv(0, &argv, Some(Path::new("/").into())).key;
    admission.0.ledger.lock().unwrap().peaks.remember(
        &key,
        admission::Peak {
            memory: 0,
            writes: 12 * 1024 * MIB,
            exact: admission::seen_argv(0, &argv, Some(Path::new("/").into())).exact,
            volume: Some(admission::ReceivingVolume {
                path: "/".into(),
                id: 1,
            }),
            volume_writes: 12 * 1024 * MIB,
        },
        512 * MIB,
    );
    let _first = admission
        .reserve("first", &argv, Path::new("/"), false)
        .unwrap();
    let other = writer_entry("other", 84, 8 * 1024 * MIB, false);
    admission.0.join(&other);
    admission.0.enforce(&other, None);
    assert!(
        other.lock().unwrap().admission[&84].wait.is_none(),
        "binary predictions held a command on a healthy disk"
    );
}

#[test]
fn healthy_observed_disk_predictions_do_not_hold_binary_launches() {
    let floor = 10 * 1024 * MIB;
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, floor + 15 * 1024 * MIB);
    let argv = vec!["writer".into()];
    let key = admission::seen_argv(0, &argv, Some(Path::new("/").into())).key;
    admission.0.ledger.lock().unwrap().peaks.remember(
        &key,
        admission::Peak {
            memory: 0,
            writes: 8 * 1024 * MIB,
            exact: admission::seen_argv(0, &argv, Some(Path::new("/").into())).exact,
            volume: Some(admission::ReceivingVolume {
                path: "/".into(),
                id: 1,
            }),
            volume_writes: 8 * 1024 * MIB,
        },
        512 * MIB,
    );
    let other = writer_entry("other", 84, 12 * 1024 * MIB, true);
    admission.0.join(&other);
    assert!(
        admission
            .reserve("binary", &argv, Path::new("/"), false)
            .is_ok(),
        "observed predictions held a binary on a healthy disk"
    );
}

#[test]
fn descendants_reserve_growth_beyond_a_smaller_binary_prediction() {
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, u64::MAX);
    let argv = vec!["parent".into()];
    learn(&admission, &argv, Path::new("/"), 1024 * MIB);
    let _first = admission
        .reserve("first", &argv, Path::new("/"), false)
        .unwrap();
    let child = descendant("first", 42, true);
    child.lock().unwrap().admission.insert(
        42,
        admission::Entry::predicted(
            42,
            admission::Peak {
                memory: 8 * 1024 * MIB,
                writes: 0,

                ..Default::default()
            },
            true,
            Path::new("/"),
        ),
    );
    admission.0.join(&child);
    let second = vec!["second".into()];
    learn(&admission, &second, Path::new("/"), 4 * 1024 * MIB);
    assert!(
        admission
            .reserve("second", &second, Path::new("/"), false)
            .is_err(),
        "larger descendant prediction lost its excess reservation"
    );
    let other = descendant("other", 84, false);
    admission.0.join(&other);
    admission.0.enforce(&other, None);
    assert!(other.lock().unwrap().admission[&84].wait.is_some());
}

#[test]
fn waiting_descendant_can_use_its_binary_memory_reservation() {
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, u64::MAX);
    let argv = vec!["parent".into()];
    learn(&admission, &argv, Path::new("/"), 4 * 1024 * MIB);
    let _first = admission
        .reserve("first", &argv, Path::new("/"), false)
        .unwrap();
    let _second = admission
        .reserve("second", &argv, Path::new("/"), false)
        .unwrap();
    let child = descendant("first", 42, false);
    child.lock().unwrap().admission.get_mut(&42).unwrap().wait =
        Some(admission::Wait::Held(admission::WaitReason::Memory));
    admission.0.join(&child);
    admission.0.enforce(&child, None);
    assert!(
        child.lock().unwrap().admission[&42].wait.is_none(),
        "descendant could not use its domain's reserved memory"
    );
    assert!(admission
        .reserve("third", &argv, Path::new("/"), false)
        .is_err());
}

#[test]
fn waiting_descendant_can_use_its_binary_disk_reservation() {
    let floor = 10 * 1024 * MIB;
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, floor + 20 * 1024 * MIB);
    let cwd = Path::new("/");
    let argv = vec!["writer".into()];
    let key = admission::seen_argv(0, &argv, Some(cwd.into())).key;
    admission.0.ledger.lock().unwrap().peaks.remember(
        &key,
        admission::Peak {
            memory: 0,
            writes: 10 * 1024 * MIB,
            exact: admission::seen_argv(0, &argv, Some(cwd.into())).exact,
            volume: Some(admission::ReceivingVolume {
                path: cwd.into(),
                id: 1,
            }),
            volume_writes: 10 * 1024 * MIB,
        },
        512 * MIB,
    );
    let _first = admission.reserve("first", &argv, cwd, false).unwrap();
    let _second = admission.reserve("second", &argv, cwd, false).unwrap();
    let child = writer_entry("first", 42, 10 * 1024 * MIB, false);
    child.lock().unwrap().admission.get_mut(&42).unwrap().wait =
        Some(admission::Wait::Held(admission::WaitReason::Disk));
    admission.0.join(&child);
    admission.0.enforce(&child, None);
    assert!(
        child.lock().unwrap().admission[&42].wait.is_none(),
        "descendant could not use its domain's reserved disk"
    );
    assert!(admission.reserve("third", &argv, cwd, false).is_ok());
}

#[test]
fn binary_prediction_waits_for_its_receiving_volume_instead_of_its_working_directory() {
    let admission = LaunchAdmission(Arc::new(Pool::with_volume(
        10 * 1024 * MIB,
        Box::new(move |path| {
            Some(admission::Space {
                id: if path.starts_with("/output") { 2 } else { 1 },
                free: if path.starts_with("/output") {
                    128 * MIB
                } else {
                    100 * 1024 * MIB
                },
                total: 100 * 1024 * MIB,
            })
        }),
    )));
    let argv = vec!["writer".into(), "--output=/output/result".into()];
    let seen = admission::seen_argv(0, &argv, Some("/workspace".into()));
    admission.0.ledger.lock().unwrap().peaks.remember(
        &seen.key,
        admission::Peak {
            writes: 2 * 1024 * MIB,
            volume_writes: 2 * 1024 * MIB,
            volume: Some(admission::ReceivingVolume {
                path: "/output".into(),
                id: 2,
            }),
            exact: seen.exact,
            ..Default::default()
        },
        512 * MIB,
    );
    let wait = admission
        .reserve("writer", &argv, Path::new("/workspace"), false)
        .err()
        .expect("the output volume cannot fit the remembered writes");
    assert_eq!(wait.reason, admission::WaitReason::Disk);
    assert_eq!(wait.need_bytes, 2 * 1024 * MIB);
    assert_eq!(wait.free_bytes, 0);
}

#[test]
fn a_different_exact_binary_command_has_no_known_output_volume() {
    let admission = LaunchAdmission::limited(10 * 1024 * MIB, 0);
    let first = vec!["writer".into(), "--output=/output-1".into()];
    let next = vec!["writer".into(), "--output=/output-2".into()];
    let seen = admission::seen_argv(0, &first, Some("/workspace".into()));
    assert_eq!(
        seen.key,
        admission::seen_argv(0, &next, Some("/workspace".into())).key
    );
    admission.0.ledger.lock().unwrap().peaks.remember(
        &seen.key,
        admission::Peak {
            writes: 2 * 1024 * MIB,
            volume_writes: 2 * 1024 * MIB,
            volume: Some(admission::ReceivingVolume {
                path: "/output-1".into(),
                id: 1,
            }),
            exact: seen.exact,
            ..Default::default()
        },
        512 * MIB,
    );
    assert!(
        admission
            .reserve("next", &next, Path::new("/workspace"), false)
            .is_ok(),
        "a normalized command key cannot identify the next command's output volume"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn declared_job_memory_waits_before_spawn_on_a_healthy_machine() {
    let total = std::fs::read_to_string("/proc/meminfo")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024;
    let pool = Arc::new(Pool::new(total, true).with_machine_pressure(Box::new(|| false)));
    let admission = LaunchAdmission(pool.clone());
    let argv = vec!["/bin/true".into()];
    // An explicitly sized job can wait for a larger machine without either
    // starting a PID or stopping healthy work to manufacture room.
    let required = total.saturating_mul(2);
    let wait = admission
        .reserve_with_memory("declared", &argv, Path::new("/"), false, required)
        .err()
        .expect("declared memory needs must be admitted before a PID exists");
    assert_eq!(wait.reason, admission::WaitReason::Memory);
    assert_eq!(wait.need_bytes, required);
    assert!(pool.launches.lock().unwrap().is_empty());
    // Ordinary commands with no declared need remain governed by actual
    // pressure, and the owner's memory-off decision also applies to jobs.
    assert!(admission
        .reserve("ordinary", &argv, Path::new("/"), false)
        .is_ok());
    pool.configure(crate::config::ResourceProtection {
        memory: false,
        ..Default::default()
    });
    assert!(admission
        .reserve_with_memory("off", &argv, Path::new("/"), false, required)
        .is_ok());
}
