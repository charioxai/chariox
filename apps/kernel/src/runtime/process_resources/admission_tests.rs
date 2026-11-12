//! Admission (OF-01, OF-04, OF-05): a known heavy command waits in line
//! before it grows and starts by itself when room frees; Start now admits it
//! at once; a command filling a nearly full disk is stopped with its reason.
use super::pressure_tests::{domain, pool, settle, writer_command, Workload, DIE_WITH_PARENT};
use super::*;

/// Sleeps `delay` seconds (a build's start), takes `mib`, holds it `seconds`.
fn heavy(mib: u64, delay: u64, seconds: u64) -> String {
    format!("python3 -c '{DIE_WITH_PARENT}import time,sys; time.sleep({delay}); b=bytearray({mib}*1048576); time.sleep({seconds})' heavy")
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
    take_wait_events();
    let (a, mut b, _pool) = held();
    assert!(b.waiting("job"), "the heavy command was not held");
    match state(&b, "job") {
        SessionProcessState::Held(Waiting {
            reason: WaitReason::Memory,
            position: 1,
            need_bytes,
            command_line,
            since_ms,
            ..
        }) => {
            assert!(need_bytes >= 590 * MIB, "need {need_bytes}");
            assert!(command_line.starts_with("python3 -c"), "{command_line}");
            assert!(since_ms > 0, "the row does not say since when it waits");
        }
        other => panic!("held command reads {other:?}"),
    }
    assert!(b.alive("job"), "a held command was stopped");
    // Its own error stream says it waits, why and its place in line.
    b.assert_told("job", "waiting to start for memory (1st in line");
    let began = take_wait_events()
        .into_iter()
        .filter(|event| matches!(event.state, SessionProcessState::Held(_)))
        .collect::<Vec<_>>();
    assert_eq!(began.len(), 1, "the room was not told: {began:?}");
    assert_eq!(began[0].change, WaitChange::Began);
    // A's job ends: B starts by itself and finishes normally.
    unsafe { libc::kill(a.pid("big"), libc::SIGKILL) };
    settle(3);
    assert!(
        !b.waiting("job"),
        "the held command did not start with room"
    );
    b.assert_told("job", "continues after waiting");
    let resumed = take_wait_events()
        .into_iter()
        .filter(|event| event.process_id == began[0].process_id)
        .collect::<Vec<_>>();
    assert!(
        matches!(resumed.as_slice(), [event] if matches!(event.change, WaitChange::Resumed { .. })
            && event.state == SessionProcessState::Running),
        "the room was not told it runs again: {resumed:?}"
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

/// The volume's free space is the test's to change (a 100 GiB volume has a
/// 10 GiB floor, so 4 GiB is below half of it).
#[cfg(target_os = "linux")]
fn volume_with(free: u64) -> Arc<pool::Pool> {
    Arc::new(pool::Pool::with_volume(
        1 << 40,
        Box::new(move |path| {
            use std::os::unix::fs::MetadataExt;
            Some(admission::Space {
                id: std::fs::metadata(path).ok()?.dev(),
                free,
                total: 100 * GIB,
            })
        }),
    ))
}

#[cfg(target_os = "linux")]
#[test]
fn a_command_filling_the_disk_is_stopped_with_its_reason_never_paused() {
    take_wait_events();
    let pool = volume_with(4 * GIB);
    let mut writer = Workload::run(
        domain(&pool, "writer", false),
        &[("write", writer_command())],
    );
    settle(4);
    assert!(
        !writer.alive("write"),
        "a writer filling the disk kept writing"
    );
    let events = take_wait_events();
    assert!(
        events.iter().any(|event| matches!(
            &event.change,
            WaitChange::Stopped { reason } if reason.contains("was filling it")
        )),
        "the room was not told why the writer ended: {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.change, WaitChange::Began)),
        "the writer was paused first: {events:?}"
    );
}

/// What the previous program's admission handed over for a held command.
#[cfg(target_os = "linux")]
fn reload(command: &Workload, handed_over: Vec<admission::HeldCommand>) {
    admission::adopt_admission(handed_over);
    command.domain.state.lock().unwrap().admission.clear();
}

#[cfg(target_os = "linux")]
#[test]
fn a_held_command_is_handed_over_and_still_startable_after_a_hot_reload() {
    let (_a, b, pool) = held();
    let handed_over: Vec<admission::HeldCommand> =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();
    assert_eq!(handed_over.len(), 1);
    reload(&b, handed_over);
    settle(2);
    assert!(
        matches!(state(&b, "job"), SessionProcessState::Held(_)),
        "the reload forgot the command was held"
    );
    let key = format!("admission-{:016x}", rand::random::<u64>());
    b.domain.bind_key(&key);
    let process_id = samples_for_key(&key)
        .into_iter()
        .find(|sample| sample.pid == b.pid("job") as u32)
        .unwrap()
        .process_id;
    assert_eq!(
        start_now_for_key(&key, process_id.clone()),
        Ok(vec![process_id])
    );
    settle(1);
    assert!(!b.waiting("job"), "Start now did not start it");
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

/// A terminal's writer crosses a hot reload onto a volume below half its
/// floor: whether the next program's admission stopped it.
#[cfg(target_os = "linux")]
fn stopped_after_a_hot_reload(launch: Launch) -> bool {
    let mut writer = Workload::gated(
        domain(&volume_with(50 * GIB), "writer", false),
        &[("write", writer_command())],
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
    let _next = writer
        .watched_again(domain(&volume_with(4 * GIB), "writer", false).launched_by(handed_over));
    settle(4);
    writer.domain.unbind_key(&key);
    !writer.alive("write")
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_foreground_command_is_still_never_stopped_after_a_hot_reload() {
    assert!(
        !stopped_after_a_hot_reload(Launch::PersonInTheForeground),
        "the reload made a person's running command stoppable"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_background_writer_is_still_never_stopped_after_a_hot_reload() {
    assert!(
        !stopped_after_a_hot_reload(Launch::PersonInTheBackground),
        "the reload made a person's background writer stoppable"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_persons_command_in_a_shell_without_marks_is_still_never_stopped_after_a_hot_reload() {
    assert!(
        !stopped_after_a_hot_reload(Launch::PersonInAShellWithoutMarks),
        "the reload forgot that the shell reports no lines"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_agents_command_is_still_stopped_when_it_fills_the_disk_after_a_hot_reload() {
    assert!(
        stopped_after_a_hot_reload(Launch::AgentInTheForeground),
        "the reload exempted an agent's command from the disk stop"
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

#[cfg(target_os = "linux")]
#[test]
fn a_long_wait_inside_the_limit_does_not_emit_a_notice() {
    let (_a, writer, _pool) = held();
    take_wait_events();
    {
        let mut observation = writer.domain.state.lock().unwrap();
        for entry in observation.admission.values_mut() {
            entry.since_ms = crate::session::unix_epoch_ms() - 50_000;
        }
    }
    settle(2);
    assert!(
        take_wait_events().is_empty(),
        "an unchanged wait must not emit the retired long-wait notice"
    );
    assert!(
        writer.waiting("job"),
        "the waiting row must remain available"
    );
}

/// A tool shell that runs one job whose peak (1000 MiB) is known and is now at
/// 600 MiB, then a second one after a sampling barrier: the shell becomes a launcher and the first job a command
/// of its own. Its admission came with it, so the job is not held for room
/// the shell still reserves.
#[cfg(target_os = "linux")]
#[test]
fn a_known_heavy_job_keeps_its_admission_when_its_shell_runs_a_second_job() {
    let pool = pool(1536);
    let mut first = Workload::run(domain(&pool, "first", false), &[("job", heavy(1000, 0, 2))]);
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

    let mut shell = Workload::gated(
        domain(&pool, "agent", false),
        &[("jobs", "sh \"$0/jobs.sh\" \"$0\"".into())],
    );
    std::fs::write(
        shell.dir.join("jobs.sh"),
        format!(
            "{} 2>\"$1/first.err\" &\necho $! > \"$1/first.pid\"\nwhile [ ! -e \"$1/second\" ]; do :; done\nsleep 1 2>\"$1/second.err\" &\necho $! > \"$1/second.pid\"\nwait\n",
            heavy(600, 0, 8)
        ),
    )
    .unwrap();
    shell.go(&["jobs"]);
    while !shell.dir.join("first.pid").exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(3);
    {
        let observation = shell.domain.state.lock().unwrap();
        let entry = &observation.admission[&(shell.pid("jobs") as u32)];
        assert_eq!(
            entry.admitted_with_prediction(),
            (true, true),
            "the shell did not start as the heavy job's command"
        );
    }

    // The sampling barrier is behind: the shell starts its second job.
    std::fs::write(shell.dir.join("second"), "").unwrap();
    while !shell.dir.join("second.pid").exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(3);
    assert!(
        !shell.waiting("first"),
        "the first job was held for room its own shell still reserved"
    );
    {
        let observation = shell.domain.state.lock().unwrap();
        let shell_entry = &observation.admission[&(shell.pid("jobs") as u32)];
        assert!(
            !shell_entry.admitted_with_prediction().1,
            "the shell still counts as the job it handed to its own command"
        );
        let job = &observation.admission[&(shell.pid("first") as u32)];
        assert!(
            job.admitted_with_prediction().0 && job.wait.is_none(),
            "the job lost its admission"
        );
    }
    settle(5);
    assert!(
        !shell.alive("first"),
        "the first job did not run to its end without Start now"
    );
    assert!(!shell.alive("second"), "the short second job did not exit");
}

/// A command held before it started has no outcome either while the room
/// does not come: after the limit it ends naming what it needs.
#[cfg(target_os = "linux")]
#[test]
fn a_held_command_whose_room_does_not_come_ends_with_the_room_and_need() {
    let (mut a, mut b, _pool) = held();
    assert!(b.waiting("job"), "the heavy command was not held");
    {
        let mut observation = b.domain.state.lock().unwrap();
        for entry in observation
            .admission
            .values_mut()
            .filter(|e| e.wait.is_some())
        {
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
    }
    settle(3);
    assert!(
        !b.alive("job"),
        "the held command stayed held with no outcome"
    );
    let notice = b.told("job");
    assert!(
        notice.contains("it still needs ") && notice.contains("are left"),
        "the notice does not explain the block: {notice:?}"
    );
    assert!(a.root_alive(), "the other workload was stopped");
}
