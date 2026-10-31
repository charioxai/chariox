//! Admission (OF-01, OF-04, OF-05): a known heavy command waits in line
//! before it grows and starts by itself when room frees; Start now admits it
//! at once; a command writing a full disk is paused, not stopped.
use super::pressure_tests::{domain, pool, settle, Workload};
use super::*;

/// Sleeps `delay` seconds (a build's start), takes `mib`, holds it `seconds`.
fn heavy(mib: u64, delay: u64, seconds: u64) -> String {
    format!("python3 -c 'import time,sys; time.sleep({delay}); b=bytearray({mib}*1048576); time.sleep({seconds})' heavy")
}

fn state(workload: &Workload, name: &str) -> SessionProcessState {
    let key = format!("admission-{:016x}", rand::random::<u64>());
    workload.domain.bind_key(&key);
    let pid = workload.pid(name) as u32;
    let state = projection::states_for_key(&key)
        .remove(&pid)
        .unwrap_or_default();
    workload.domain.unbind_key(&key);
    state
}

/// A 1536 MiB pool that has seen `heavy(600, ..)` peak at 600 MiB, with A
/// holding 900 MiB: B's run of that command does not fit and is held.
fn held() -> (Workload, Workload, Arc<pool::Pool>) {
    let pool = pool(1536);
    let mut first = Workload::run(domain(&pool, "first", false), &[("job", heavy(600, 0, 2))]);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while first.alive("job") || first.root_alive() {
        assert!(
            std::time::Instant::now() < deadline,
            "first run did not end"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    settle(1);
    drop(first);
    let a = Workload::start(domain(&pool, "a", false), &[("big", 900, 0, 0)]);
    settle(1);
    let b = Workload::run(domain(&pool, "b", false), &[("job", heavy(600, 1, 2))]);
    settle(2);
    (a, b, pool)
}

#[cfg(target_os = "linux")]
#[test]
fn a_known_heavy_command_waits_in_line_then_starts_by_itself() {
    let (a, mut b, _pool) = held();
    assert!(b.waiting("job"), "the heavy command was not held");
    match state(&b, "job") {
        SessionProcessState::Held {
            reason: WaitReason::Memory,
            position: 1,
            need_bytes,
            command_line,
            ..
        } => {
            assert!(need_bytes >= 590 * MIB, "need {need_bytes}");
            assert!(command_line.starts_with("python3 -c"), "{command_line}");
        }
        other => panic!("held command reads {other:?}"),
    }
    assert!(b.alive("job"), "a held command was stopped");
    // A's job ends: B starts by itself and finishes normally.
    unsafe { libc::kill(a.pid("big"), libc::SIGKILL) };
    settle(3);
    assert!(
        !b.waiting("job"),
        "the held command did not start with room"
    );
    assert_eq!(state(&b, "job"), SessionProcessState::Running);
    settle(3);
    assert!(
        !b.alive("job"),
        "the started command did not run to its end"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn start_now_admits_a_held_command_whatever_the_room() {
    let (_a, b, _pool) = held();
    assert!(b.waiting("job"), "the heavy command was not held");
    let key = format!("admission-{:016x}", rand::random::<u64>());
    b.domain.bind_key(&key);
    let process_id = samples_for_key(&key)
        .into_iter()
        .find(|sample| sample.pid == b.pid("job") as u32)
        .unwrap()
        .process_id;
    assert_eq!(
        start_now_for_key(&key, process_id.clone()),
        Ok(vec![process_id.clone()])
    );
    settle(1);
    assert!(!b.waiting("job"), "Start now did not start it");
    assert!(
        start_now_for_key(&key, process_id).is_err(),
        "a running command reads as waiting"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_writing_past_the_disk_floor_is_paused_until_space_frees() {
    use std::sync::atomic::{AtomicU64, Ordering};
    const GIB: u64 = 1024 * MIB;
    // A 100 GiB volume: its floor is 10 GiB.
    let free = Arc::new(AtomicU64::new(5 * GIB));
    let reported = free.clone();
    let pool = Arc::new(pool::Pool::with_volume(
        1 << 40,
        Box::new(move |_| Some((reported.load(Ordering::SeqCst), 100 * GIB))),
    ));
    let mut writer = Workload::run(
        domain(&pool, "writer", false),
        &[(
            "write",
            "python3 -c 'import os,sys,time\nf=open(sys.argv[1],\"wb\")\nwhile True:\n f.write(os.urandom(4194304)); f.flush(); os.fsync(f.fileno()); time.sleep(0.5)' \"$0/out\""
                .into(),
        )],
    );
    settle(3);
    assert!(writer.alive("write"), "a writer on a full disk was stopped");
    assert!(
        writer.waiting("write"),
        "a writer past the floor kept writing"
    );
    assert!(matches!(
        state(&writer, "write"),
        SessionProcessState::Paused {
            reason: WaitReason::Disk,
            ..
        }
    ));
    free.store(50 * GIB, Ordering::SeqCst);
    settle(3);
    assert!(
        !writer.waiting("write"),
        "the writer did not continue with space"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_own_terminal_command_never_waits() {
    let pool = pool(1536);
    let _a = Workload::start(domain(&pool, "a", false), &[("big", 800, 0, 0)]);
    settle(1);
    let prepared = domain(&pool, "person", false);
    let person = Workload::run(
        prepared,
        &[(
            "grow",
            "sleep 1; exec python3 \"$0/alloc.py\" 200 48 520".into(),
        )],
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    person.domain.bind_key(&key);
    set_holdable_for_key(&key, false);
    settle(6);
    assert!(!person.waiting("grow"), "a person's own command was paused");
    person.domain.unbind_key(&key);
}

#[cfg(target_os = "linux")]
#[test]
fn a_launcher_chain_is_the_harness_and_only_its_later_commands_wait() {
    // A wrapper that runs the provider's server (`node` → `codex app-server`)
    // starts with the root; the commands come from the server.
    let pool = pool(1536);
    let mut first = Workload::launched(domain(&pool, "first", false), &[("job", heavy(600, 0, 2))]);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while first.alive("job") {
        assert!(
            std::time::Instant::now() < deadline,
            "first run did not end"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    settle(1);
    drop(first);
    let a = Workload::start(domain(&pool, "a", false), &[("big", 900, 0, 0)]);
    settle(1);
    let b = Workload::launched(domain(&pool, "b", false), &[("job", heavy(600, 1, 2))]);
    settle(2);
    assert!(
        b.waiting("job"),
        "the heavy command under the launcher was not held"
    );
    let parent = |pid: String| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap()
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1).map(str::to_string))
            .unwrap()
    };
    // The job's shell is its command; the shell's parent is the server.
    let server = parent(parent(b.pid("job").to_string()));
    let server_state = std::fs::read_to_string(format!("/proc/{server}/stat")).unwrap();
    assert!(
        server_state.contains("(python3)")
            && !server_state
                .rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('T'),
        "the harness itself was stopped: {server_state}"
    );
    unsafe { libc::kill(a.pid("big"), libc::SIGKILL) };
    settle(3);
    assert!(
        !b.waiting("job"),
        "the held command did not start with room"
    );
}
