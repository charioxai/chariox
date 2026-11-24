//! Real memory exhaustion on real processes: the command that grows most is
//! throttled, paused and stopped, told each step, and nobody else is touched.
//! The machine's reading is injected; the processes, signals and the lines
//! they are told are real. The wait-event queue is global and other tests
//! drain it, so what the room and the agent hear is checked where the events
//! are delivered (state/outside_events/process_wait_tests.rs) and in the drill.
use super::pressure_tests::{domain, settle, Workload};
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn hot_gauge() -> exhaustion::Gauge {
    // 25 GiB of a 100 GiB pool is free: the old 512 MiB line never fires here.
    exhaustion::Gauge {
        psi: 40.0,
        pool_bytes: 100 * 1024 * MIB,
        pool_used: 75 * 1024 * MIB,
    }
}

fn calm_gauge() -> exhaustion::Gauge {
    exhaustion::Gauge {
        psi: 1.0,
        pool_bytes: 100 * 1024 * MIB,
        pool_used: 30 * 1024 * MIB,
    }
}

/// A pool whose machine reads hot while `hot` is set, calm otherwise.
fn pool_reading(hot: Arc<AtomicBool>) -> Arc<pool::Pool> {
    Arc::new(
        pool::Pool::new(8 * 1024 * MIB, false).with_gauge(Box::new(move || {
            Some(if hot.load(Ordering::Relaxed) {
                hot_gauge()
            } else {
                calm_gauge()
            })
        })),
    )
}

/// M2, M5, M6, M7 on real processes: only the runaway is acted on, its own
/// error stream and the room's events carry each step with the numbers.
#[cfg(target_os = "linux")]
#[test]
fn the_runaway_is_throttled_paused_and_stopped_and_told_while_others_run_on() {
    let hot = Arc::new(AtomicBool::new(true));
    let pool = pool_reading(hot);
    let mut a = Workload::start(
        domain(&pool, "agent-a", false),
        &[("grow", 100, 160, 3000), ("steady", 300, 0, 0)],
    );
    let mut b = Workload::start(domain(&pool, "agent-b", false), &[("other", 400, 0, 0)]);
    // 5 s of pressure, then throttled.
    settle(7);
    let told = a.told("grow");
    assert!(
        told.contains("was throttled") && told.contains("out of memory") && told.contains("-j"),
        "throttle not told: {told:?}"
    );
    assert!(
        a.alive("grow") && !a.waiting("grow"),
        "throttle is not a pause"
    );
    // 3 s after the throttle: paused.
    settle(4);
    assert!(a.waiting("grow"), "the runaway was not paused");
    assert!(
        a.told("grow").contains("was paused"),
        "{:?}",
        a.told("grow")
    );
    assert!(a.alive("steady") && b.alive("other"));
    // 13 s after the throttle: stopped, with the reason as its result.
    settle(10);
    assert!(!a.alive("grow"), "the runaway was not stopped");
    let told = a.told("grow");
    for part in ["was stopped", "ran out of memory", "grew", "Run it again"] {
        assert!(told.contains(part), "{part}: {told:?}");
    }
    assert!(
        a.alive("steady") && b.alive("other"),
        "an innocent command was touched"
    );
    assert!(a.root_alive() && b.root_alive());
}

/// Pressure that ends lets the paused command go, told as running again.
#[cfg(target_os = "linux")]
#[test]
fn pressure_that_ends_lets_the_paused_runaway_continue() {
    let hot = Arc::new(AtomicBool::new(true));
    let pool = pool_reading(hot.clone());
    let mut a = Workload::start(domain(&pool, "agent-a", false), &[("grow", 100, 160, 3000)]);
    settle(10);
    assert!(a.waiting("grow"), "not paused while pressure holds");
    hot.store(false, Ordering::Relaxed);
    // 10 s below the clear line, then it continues.
    settle(13);
    assert!(a.alive("grow") && !a.waiting("grow"), "it was not let go");
    assert!(a.told("grow").contains("continues"), "{:?}", a.told("grow"));
}

/// A heavy build that does not grow into pressure, or a machine with no
/// pressure, is never touched (M1's rule on real processes).
#[cfg(target_os = "linux")]
#[test]
fn a_large_command_is_left_alone_without_pressure() {
    let hot = Arc::new(AtomicBool::new(false));
    let pool = pool_reading(hot);
    let mut a = Workload::start(domain(&pool, "agent-a", false), &[("big", 100, 160, 1200)]);
    settle(14);
    assert!(a.alive("big") && !a.waiting("big"));
    assert!(!a.told("big").contains("chariox"), "{:?}", a.told("big"));
    assert!(a.root_alive());
}

/// Memory protection off lets a command exhaustion paused go.
#[cfg(target_os = "linux")]
#[test]
fn turning_memory_protection_off_releases_a_paused_runaway() {
    let hot = Arc::new(AtomicBool::new(true));
    let pool = pool_reading(hot);
    let mut a = Workload::start(domain(&pool, "agent-a", false), &[("grow", 100, 160, 3000)]);
    settle(10);
    assert!(a.waiting("grow"));
    pool.configure(crate::config::ResourceProtection {
        memory: false,
        ..Default::default()
    });
    settle(3);
    assert!(
        a.alive("grow") && !a.waiting("grow"),
        "protection off left it paused"
    );
}

/// A workload that is itself the runaway (a job is its own domain): its root
/// allocates without bound.
#[cfg(target_os = "linux")]
fn root_runaway(
    pool: &Arc<pool::Pool>,
) -> (
    std::process::Child,
    ProcessDomain,
    std::path::PathBuf,
    String,
) {
    use std::os::unix::process::CommandExt;
    let dir = std::env::temp_dir().join(format!("pc-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let prepared = domain(pool, "job-owner", false);
    let (program, args) = prepared.command(
        "python3".into(),
        vec![
            "-c".into(),
            super::pressure_tests::ALLOC.into(),
            "100".into(),
            "160".into(),
            "3000".into(),
        ],
    );
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stderr(std::fs::File::create(dir.join("job.err")).unwrap());
    // SAFETY: only `prctl` runs between fork and exec; the job dies with the test.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    let domain = prepared.start(child.id()).unwrap();
    // One key per workload: the registry maps a key to one domain, and the tests run together.
    let key = format!("root-runaway-{}", child.id());
    domain.bind_key(&key);
    (child, domain, dir, key)
}

#[cfg(target_os = "linux")]
fn process_state(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_string))
}

/// Waits until `check` holds, for at most `seconds`: the tests wait on what the
/// policy recorded or did, never on a fixed time.
#[cfg(target_os = "linux")]
fn wait_for(seconds: u64, what: &str, mut check: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while !check() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "linux")]
fn listed_paused(key: &str, pid: u32) -> bool {
    matches!(
        super::projection::states_for_key(key).get(&pid),
        Some(SessionProcessState::Paused(row)) if row.reason == WaitReason::Memory
    )
}

/// The workload itself is the runaway: its whole domain is frozen, the process
/// list shows it paused, and it is then stopped with its reason on its own
/// error stream.
#[cfg(target_os = "linux")]
#[test]
fn a_workload_that_is_itself_the_runaway_is_frozen_listed_as_paused_then_stopped() {
    let pool = pool_reading(Arc::new(AtomicBool::new(true)));
    let (mut child, domain, dir, key) = root_runaway(&pool);
    let pid = child.id();
    wait_for(60, "the workload to be listed as paused", || {
        listed_paused(&key, pid)
    });
    assert_eq!(
        process_state(pid).as_deref(),
        Some("T"),
        "listed as paused but not held"
    );
    wait_for(60, "the runaway workload to be stopped", || {
        child.try_wait().unwrap().is_some()
    });
    let told = std::fs::read_to_string(dir.join("job.err")).unwrap_or_default();
    for part in ["was throttled", "was paused", "was stopped"] {
        assert!(told.contains(part), "{part}: {told:?}");
    }
    drop(domain);
    let _ = std::fs::remove_dir_all(dir);
}

/// A freeze queued before the hot reload began, handled after the handover
/// released everything, must not hold the workload under a program that is
/// about to be replaced.
#[cfg(target_os = "linux")]
#[test]
fn a_freeze_queued_before_a_hot_reload_is_dropped_after_the_handover_release() {
    // The machine reads calm: nothing freezes it but the queued command below.
    let pool = pool_reading(Arc::new(AtomicBool::new(false)));
    let (mut child, domain, dir, _key) = root_runaway(&pool);
    let pid = child.id();
    settle(2);
    pool.release_for_handover();
    domain.freeze_for_test(pid);
    settle(2);
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "a queued freeze held the workload after the handover began"
    );
    drop(domain);
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(dir);
}

/// A hold in flight when the handover starts: the release waits for it to be
/// recorded, then lets it go, so no workload stays held when the program is
/// replaced (the hold is signalled and recorded under the gate the release takes).
#[cfg(target_os = "linux")]
#[test]
fn the_handover_release_waits_for_a_hold_in_flight_and_then_lets_it_go() {
    let pool = pool_reading(Arc::new(AtomicBool::new(false)));
    let (mut child, domain, dir, key) = root_runaway(&pool);
    let pid = child.id();
    settle(2);
    // A hold in flight: signalled and recorded under the gate, as the watch does.
    let gate = pool.hold_gate();
    unsafe { libc::kill(pid as i32, libc::SIGSTOP) };
    domain.record_frozen_for_test(pid);
    let releaser = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.release_for_handover())
    };
    settle(1);
    assert!(
        !releaser.is_finished(),
        "the release did not wait for the hold in flight"
    );
    assert_eq!(process_state(pid).as_deref(), Some("T"));
    drop(gate);
    releaser.join().unwrap();
    settle(1);
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "the hold in flight survived the release"
    );
    assert!(!super::projection::states_for_key(&key).contains_key(&pid));
    drop(domain);
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(dir);
}

/// A hot reload replaces the program while a workload is frozen: it is let go
/// first (the next program starts without the state), listed running, and
/// nothing is frozen again under this program.
#[cfg(target_os = "linux")]
#[test]
fn a_frozen_workload_is_released_before_a_hot_reload_and_not_frozen_again() {
    let pool = pool_reading(Arc::new(AtomicBool::new(true)));
    let (mut child, domain, dir, key) = root_runaway(&pool);
    let pid = child.id();
    wait_for(60, "the workload to be listed as paused", || {
        listed_paused(&key, pid)
    });
    pool.release_for_handover();
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "still stopped after the handover release"
    );
    assert!(
        !super::projection::states_for_key(&key).contains_key(&pid),
        "still listed as paused"
    );
    // The machine is still out of memory, but this program holds nothing more:
    // give the policy several passes (it acts within a pass of its trigger).
    settle(8);
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "frozen again while handing over"
    );
    assert!(
        child.try_wait().unwrap().is_none(),
        "stopped while handing over"
    );
    drop(domain);
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(dir);
}

/// The owner starts a frozen workload again by hand: it runs, is listed
/// running, and the policy never holds or stops it again.
#[cfg(target_os = "linux")]
#[test]
fn a_frozen_workload_its_owner_starts_again_by_hand_is_not_held_or_stopped_again() {
    let pool = pool_reading(Arc::new(AtomicBool::new(true)));
    let (mut child, domain, dir, key) = root_runaway(&pool);
    let pid = child.id();
    wait_for(60, "the workload to be listed as paused", || {
        listed_paused(&key, pid)
    });
    let process_id = domain
        .state
        .lock()
        .unwrap()
        .samples
        .iter()
        .find(|sample| sample.pid == pid)
        .map(|sample| sample.process_id.clone())
        .unwrap();
    super::start_now_for_key(&key, process_id).expect("start now");
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "still held after Start now"
    );
    assert!(!super::projection::states_for_key(&key).contains_key(&pid));
    // Well past the policy's stop time, the machine still out of memory.
    settle(20);
    assert!(
        child.try_wait().unwrap().is_none(),
        "stopped after its owner started it"
    );
    assert_ne!(
        process_state(pid).as_deref(),
        Some("T"),
        "held again after its owner started it"
    );
    drop(domain);
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(dir);
}

/// A workload that ends while held leaves no hold in the process list.
#[cfg(target_os = "linux")]
#[test]
fn a_workload_that_ends_while_held_leaves_no_hold_behind() {
    let pool = pool_reading(Arc::new(AtomicBool::new(true)));
    let (mut child, domain, dir, key) = root_runaway(&pool);
    let pid = child.id();
    wait_for(60, "the workload to be listed as paused", || {
        listed_paused(&key, pid)
    });
    // Its owner cancels it while it is held.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let _ = child.wait();
    wait_for(10, "the hold to leave the list", || {
        super::projection::states_for_key(&key).is_empty()
    });
    assert!(domain.state.lock().unwrap().frozen.is_empty());
    drop(domain);
    let _ = std::fs::remove_dir_all(dir);
}
