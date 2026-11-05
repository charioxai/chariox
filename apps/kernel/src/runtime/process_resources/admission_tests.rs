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

const GIB: u64 = 1024 * MIB;

/// A writer on a volume below its floor (a 100 GiB volume has a 10 GiB floor),
/// paused by admission. The volume's free space is the test's to change.
#[cfg(target_os = "linux")]
fn paused_writer() -> (Workload, Arc<std::sync::atomic::AtomicU64>, Arc<pool::Pool>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let free = Arc::new(AtomicU64::new(5 * GIB));
    let reported = free.clone();
    let pool = Arc::new(pool::Pool::with_volume(
        1 << 40,
        Box::new(move |_| {
            Some(admission::Space {
                id: 1,
                free: reported.load(Ordering::SeqCst),
                total: 100 * GIB,
            })
        }),
    ));
    let writer = Workload::run(
        domain(&pool, "writer", false),
        &[(
            "write",
            "python3 -c 'import os,sys,time\nf=open(sys.argv[1],\"wb\")\nwhile True:\n f.write(os.urandom(4194304)); f.flush(); os.fsync(f.fileno()); time.sleep(0.5)' \"$0/out\""
                .into(),
        )],
    );
    settle(3);
    assert!(
        writer.waiting("write"),
        "a writer past the floor kept writing"
    );
    (writer, free, pool)
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_writing_past_the_disk_floor_is_paused_until_space_frees() {
    use std::sync::atomic::Ordering;
    let (mut writer, free, _pool) = paused_writer();
    assert!(writer.alive("write"), "a writer on a full disk was stopped");
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

/// What the previous program's admission handed over for the paused writer.
#[cfg(target_os = "linux")]
fn paused_by_the_previous_program(writer: &Workload) -> admission::HeldCommand {
    admission::HeldCommand {
        identity: crate::runtime::kernel_access::process::inspect(writer.pid("write") as u32)
            .unwrap()
            .0,
        wait: Some(admission::Wait::Paused(WaitReason::Disk)),
        admitted: true,
        forced: false,
        stopped: true,
        paused_rate: 4 * MIB,
        peak: admission::Peak::default(),
        arrived: 1,
    }
}

/// The next program's watch starts with the handover and no admission state
/// of its own.
#[cfg(target_os = "linux")]
fn reload(writer: &Workload, handed_over: Vec<admission::HeldCommand>) {
    admission::adopt_admission(handed_over);
    writer.domain.state.lock().unwrap().admission.clear();
}

#[cfg(target_os = "linux")]
#[test]
fn the_commands_admission_stopped_are_handed_over() {
    let (writer, _free, pool) = paused_writer();
    let handed_over: Vec<admission::HeldCommand> =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();
    assert_eq!(handed_over.len(), 1);
    assert_eq!(handed_over[0].identity.pid, writer.pid("write") as u32);
    assert_eq!(
        handed_over[0].wait,
        Some(admission::Wait::Paused(WaitReason::Disk))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_admission_paused_resumes_when_space_frees_after_a_hot_reload() {
    use std::sync::atomic::Ordering;
    let (writer, free, _pool) = paused_writer();
    reload(&writer, vec![paused_by_the_previous_program(&writer)]);
    settle(2);
    assert!(
        writer.waiting("write"),
        "the reload resumed the command into a full disk"
    );
    free.store(50 * GIB, Ordering::SeqCst);
    settle(3);
    assert!(
        !writer.waiting("write"),
        "the command stayed stopped after the reload although space is free"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_admission_paused_is_still_listed_and_startable_after_a_hot_reload() {
    let (writer, _free, _pool) = paused_writer();
    reload(&writer, vec![paused_by_the_previous_program(&writer)]);
    settle(2);
    assert!(
        matches!(
            state(&writer, "write"),
            SessionProcessState::Paused {
                reason: WaitReason::Disk,
                ..
            }
        ),
        "Health does not show the stopped command as waiting"
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    writer.domain.bind_key(&key);
    let process_id = samples_for_key(&key)
        .into_iter()
        .find(|sample| sample.pid == writer.pid("write") as u32)
        .unwrap()
        .process_id;
    assert_eq!(
        start_now_for_key(&key, process_id.clone()),
        Ok(vec![process_id])
    );
    settle(1);
    assert!(!writer.waiting("write"), "Start now did not start it");
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_the_person_started_now_still_never_waits_after_a_hot_reload() {
    let (writer, _free, pool) = paused_writer();
    let key = format!("admission-{:016x}", rand::random::<u64>());
    writer.domain.bind_key(&key);
    let process_id = samples_for_key(&key)
        .into_iter()
        .find(|sample| sample.pid == writer.pid("write") as u32)
        .unwrap()
        .process_id;
    assert_eq!(
        start_now_for_key(&key, process_id.clone()),
        Ok(vec![process_id])
    );
    settle(2);
    assert!(!writer.waiting("write"), "Start now did not start it");
    // The disk stays below the floor across the reload.
    let handed_over: Vec<admission::HeldCommand> =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();
    assert_eq!(handed_over.len(), 1, "the override was not handed over");
    reload(&writer, handed_over);
    settle(4);
    assert!(
        !writer.waiting("write"),
        "the reload paused the command the person started now"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn start_now_keeps_a_running_writer_admitted_after_a_hot_reload() {
    let (writer, _free, pool) = paused_writer();
    let key = format!("admission-{:016x}", rand::random::<u64>());
    writer.domain.bind_key(&key);
    let pid = writer.pid("write") as u32;
    let process_id = samples_for_key(&key)
        .into_iter()
        .find(|sample| sample.pid == pid)
        .unwrap()
        .process_id;
    assert_eq!(
        start_now_for_key(&key, process_id.clone()),
        Ok(vec![process_id])
    );
    settle(1);
    assert!(!writer.waiting("write"), "Start now did not start it");
    assert!(
        !writer.domain.state.lock().unwrap().admission[&pid].stopped,
        "the handover must happen after admission continued the writer"
    );

    // The running override crosses the same JSON boundary as a hot reload;
    // a new watch must honour it while the disk remains below its floor.
    let handed_over =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();
    admission::adopt_admission(handed_over);
    let written = std::fs::metadata(writer.dir.join("out")).unwrap().len();
    let next = writer.watched_again(domain(&pool, "writer", false));
    settle(4);
    assert!(
        !writer.waiting("write"),
        "the reload forgot Start now and paused the running writer again"
    );
    assert_eq!(state(&writer, "write"), SessionProcessState::Running);
    assert!(
        std::fs::metadata(writer.dir.join("out")).unwrap().len() > written,
        "the writer made no progress after the reload"
    );
    assert!(
        next.state.lock().unwrap().admission.contains_key(&pid),
        "the adopted watch never registered the writer"
    );
    writer.domain.unbind_key(&key);
}

/// How the terminal's shell found the line that started the writer.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum Launch {
    PersonInTheForeground,
    PersonInTheBackground,
    PersonInAShellWithoutMarks,
    AgentInTheForeground,
}

/// A terminal's writer crosses a hot reload onto a volume past its floor:
/// whether the next program's admission paused it.
#[cfg(target_os = "linux")]
fn paused_after_a_hot_reload(launch: Launch) -> bool {
    let volume = |free: u64| {
        Arc::new(pool::Pool::with_volume(
            1 << 40,
            Box::new(move |_| {
                Some(admission::Space {
                    id: 1,
                    free,
                    total: 100 * GIB,
                })
            }),
        ))
    };
    let writer = Workload::gated(
        domain(&volume(50 * GIB), "writer", false),
        &[(
            "write",
            "python3 -c 'import os,sys,time\nf=open(sys.argv[1],\"wb\")\nwhile True:\n f.write(os.urandom(4194304)); f.flush(); os.fsync(f.fileno()); time.sleep(0.5)' \"$0/out\""
                .into(),
        )],
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    writer.domain.bind_key(&key);
    let (reports_lines, agent) = match launch {
        Launch::PersonInAShellWithoutMarks => (false, false),
        Launch::AgentInTheForeground => (true, true),
        _ => (true, false),
    };
    bind_terminal_for_key(&key, reports_lines);
    if reports_lines {
        line_finished_for_key(&key, &[]);
    }
    submit_line_for_key(&key, agent);
    if reports_lines {
        line_started_for_key(&key);
    }
    writer.go(&["write"]);
    if matches!(launch, Launch::PersonInTheBackground) {
        line_finished_for_key(&key, &[writer.pid("write") as u32]);
    }
    settle(2);
    // The next program reads what the previous one handed over, then watches
    // the same processes on a volume past its floor.
    let handed_over: Submissions =
        serde_json::from_slice(&serde_json::to_vec(&writer.domain.submissions()).unwrap()).unwrap();
    let _next =
        writer.watched_again(domain(&volume(5 * GIB), "writer", false).launched_by(handed_over));
    settle(4);
    writer.domain.unbind_key(&key);
    writer.waiting("write")
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_foreground_command_is_still_never_paused_after_a_hot_reload() {
    assert!(
        !paused_after_a_hot_reload(Launch::PersonInTheForeground),
        "the reload made a person's running command pausable"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_background_writer_is_still_never_paused_after_a_hot_reload() {
    assert!(
        !paused_after_a_hot_reload(Launch::PersonInTheBackground),
        "the reload made a person's background writer pausable"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_command_in_a_shell_without_marks_is_still_never_paused_after_a_hot_reload() {
    assert!(
        !paused_after_a_hot_reload(Launch::PersonInAShellWithoutMarks),
        "the reload forgot that the shell reports no lines"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_agents_command_stays_pausable_after_a_hot_reload() {
    assert!(
        paused_after_a_hot_reload(Launch::AgentInTheForeground),
        "the reload exempted an agent's command"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_own_terminal_command_never_waits() {
    let pool = pool(1536);
    let _a = Workload::start(domain(&pool, "a", false), &[("big", 800, 0, 0)]);
    settle(1);
    let prepared = domain(&pool, "person", false);
    let person = Workload::gated(
        prepared,
        &[(
            "grow",
            "sleep 1; exec python3 \"$0/alloc.py\" 200 48 520".into(),
        )],
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    person.domain.bind_key(&key);
    // The person submits the line, the shell starts it, then the command starts.
    bind_terminal_for_key(&key, true);
    line_finished_for_key(&key, &[]);
    submit_line_for_key(&key, false);
    line_started_for_key(&key);
    person.go(&["grow"]);
    settle(6);
    assert!(!person.waiting("grow"), "a person's own command was paused");
    person.domain.unbind_key(&key);
}

#[cfg(target_os = "linux")]
#[test]
fn an_agents_command_stays_pausable_when_a_person_submits_before_its_first_sample() {
    let pool = pool(1536);
    let _a = Workload::start(domain(&pool, "a", false), &[("big", 800, 0, 0)]);
    settle(1);
    let agent = Workload::gated(
        domain(&pool, "agent", false),
        &[(
            "grow",
            "sleep 1; exec python3 \"$0/alloc.py\" 200 48 520".into(),
        )],
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    agent.domain.bind_key(&key);
    // The agent submits the line that starts a background build and the
    // person queues `true` behind it; both lines have run before the sampler
    // sees the build.
    bind_terminal_for_key(&key, true);
    line_finished_for_key(&key, &[]);
    submit_line_for_key(&key, true);
    submit_line_for_key(&key, false);
    line_started_for_key(&key);
    agent.go(&["grow"]);
    let build = agent.pid("grow") as u32;
    line_finished_for_key(&key, &[build]);
    line_started_for_key(&key);
    line_finished_for_key(&key, &[build]);
    settle(6);
    assert!(
        agent.waiting("grow"),
        "the agent's build took the person's later line and ran unpaused"
    );
    agent.domain.unbind_key(&key);
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
