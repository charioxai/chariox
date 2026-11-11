use super::*;

/// A pool with room for everything: only owner-set caps apply.
pub(super) fn roomy() -> Arc<pool::Pool> {
    Arc::new(pool::Pool::new(1 << 50, false))
}

#[test]
fn memory_override_rejects_invalid_and_overflowing_limits() {
    for value in ["", "0", "-1", "NaN", "15", "18446744073709551615"] {
        assert!(configured_cap(Some(value)).is_err(), "{value}");
    }
    assert_eq!(configured_cap(Some("64")).unwrap(), Some(64 * MIB));
}

#[cfg(unix)]
#[test]
fn watchdog_kills_only_its_owned_memory_tree_and_names_the_limit() {
    // Force the portable watchdog; the Linux cgroup path has its own drill.
    let prepared = PreparedDomain::watchdog("watchdog-test", Some(24 * MIB), roomy());
    let mut sibling = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut child = std::process::Command::new("python3")
        .args([
            "-c",
            "import time; x=bytearray(48*1024*1024); time.sleep(30)",
        ])
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while domain.memory_limit_bytes().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let limited = domain.memory_limit_bytes();
    let sibling_alive = sibling.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = sibling.kill();
    let _ = sibling.wait();
    assert_eq!(limited, Some(24 * MIB));
    assert!(sibling_alive);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn linux_memory_limit_kills_the_entire_cgroup_and_cleans_it_up() {
    let prepared = PreparedDomain::in_pool("test-unit", Some(32 * MIB), roomy());
    let path = prepared
        .cgroup
        .as_ref()
        .expect("cgroup delegation required")
        .path()
        .to_owned();
    let (program, args) = prepared.command(
        "python3".into(),
        vec![
            "-c".into(),
            "import time; x=bytearray(64*1024*1024); time.sleep(30)".into(),
        ],
    );
    let mut child = std::process::Command::new(program)
        .args(args)
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while domain.memory_limit_bytes().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let limit = domain.memory_limit_bytes();
    let _ = child.kill();
    let _ = child.wait();
    drop(domain);
    assert_eq!(limit, Some(32 * MIB));
    assert!(!path.exists(), "owned cgroup was not reclaimed");
}

#[cfg(unix)]
#[test]
fn outside_end_uses_issued_ids_and_keeps_an_unrelated_process_alive() {
    let prepared = PreparedDomain::watchdog("watchdog-test", Some(64 * MIB), roomy());
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut sibling = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    domain.bind_key("outside-end-regression");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while samples_for_key("outside-end-regression").is_empty()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(25));
    }
    let id = samples_for_key("outside-end-regression")
        .into_iter()
        .find(|row| row.pid == child.id())
        .unwrap()
        .process_id;
    assert!(stop_for_key("outside-end-regression", Some(child.id().to_string())).is_err());
    let result = stop_for_key("outside-end-regression", Some(id.clone()));
    let outside = outside_ended_for_key("outside-end-regression");
    let sibling_alive = sibling.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = sibling.kill();
    let _ = sibling.wait();
    assert_eq!(result.unwrap(), vec![id]);
    assert!(outside);
    assert!(sibling_alive);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn cgroup_observations_include_children_that_detach_between_samples() {
    use std::io::{BufRead, Write};
    let prepared = PreparedDomain::in_pool("test-unit", Some(64 * MIB), roomy());
    assert!(prepared.cgroup.is_some(), "cgroup delegation required");
    let script = "import os,time,sys\nsys.stdin.readline()\np=os.fork()\nif p==0:\n c=os.fork()\n if c==0:\n  os.setsid(); time.sleep(30); os._exit(0)\n print(c,flush=True); os._exit(0)\nos.waitpid(p,0); time.sleep(30)";
    let (program, args) = prepared.command("python3".into(), vec!["-c".into(), script.into()]);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    let key = format!("detached-tree-{}", child.id());
    domain.bind_key(&key);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while samples_for_key(&key).is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    child.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let detached: u32 = line.trim().parse().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut observed = false;
    while std::time::Instant::now() < deadline {
        if samples_for_key(&key).iter().any(|row| row.pid == detached) {
            observed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(domain);
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        observed,
        "the cgroup-owned detached child was missing from Health observations"
    );
}

#[cfg(unix)]
#[test]
fn shell_exec_keeps_owned_root_and_subsequent_children_addressable() {
    use std::io::{BufRead, Write};
    let mut child = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "read ready; exec /bin/sh -c 'sleep 30 & echo $!; wait'",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let original = crate::runtime::kernel_access::process::inspect(child.id())
        .unwrap()
        .0;
    let prepared = PreparedDomain::watchdog("watchdog-test", Some(64 * MIB), roomy());
    let domain = prepared.start(child.id()).unwrap();
    let key = format!("exec-tree-{}", child.id());
    domain.bind_key(&key);
    let wait_samples = |expected| {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let rows = samples_for_key(&key);
            if rows.len() >= expected || std::time::Instant::now() >= deadline {
                break rows;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    let before = wait_samples(1);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"exec-now\n")
        .unwrap();
    let mut descendant = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut descendant)
        .unwrap();
    let descendant: u32 = descendant.trim().parse().unwrap();
    let after = wait_samples(2);
    #[cfg(target_os = "macos")]
    let exec_changed_version = crate::runtime::kernel_access::process::inspect(child.id())
        .is_ok_and(|(current, _)| current.version != original.version);
    let stopped = stop_for_key(&key, None);
    let child_exited = child.try_wait().unwrap().is_some();
    // Cleanup also runs on the buggy base, where the watchdog has lost the root.
    if descendant > 1 && descendant <= i32::MAX as u32 {
        if let Ok((identity, parent)) = crate::runtime::kernel_access::process::inspect(descendant)
        {
            if parent == original.pid && identity.alive() {
                unsafe {
                    libc::kill(descendant as i32, libc::SIGKILL);
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    drop(domain);
    assert_eq!(before.len(), 1);
    assert!(after
        .iter()
        .any(|row| row.pid == original.pid && row.process_id == before[0].process_id));
    assert!(
        after.iter().any(|row| row.pid == descendant),
        "children created after exec remain owned"
    );
    assert!(
        stopped.is_ok(),
        "the exec'd tree can be stopped through its retained owner"
    );
    assert!(child_exited, "stop waits for the owned tree to exit");
    #[cfg(target_os = "macos")]
    assert!(
        exec_changed_version,
        "the drill must cross Darwin's exec identity change"
    );
}

/// The cgroup-free twin of `cgroup_observations_include_children_that_detach_between_samples`.
#[cfg(target_os = "linux")]
#[test]
fn without_a_cgroup_a_child_that_detached_is_observed_and_ended_with_its_owner() {
    use std::io::{BufRead, Write};
    let prepared = PreparedDomain::watchdog("detached-test", None, roomy());
    let script = "import os,time,sys\nsys.stdin.readline()\np=os.fork()\nif p==0:\n c=os.fork()\n if c==0:\n  os.setsid(); time.sleep(30); os._exit(0)\n print(c,flush=True); os._exit(0)\nos.waitpid(p,0); time.sleep(30)";
    let (program, args) = prepared.command("python3".into(), vec!["-c".into(), script.into()]);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut domain = prepared.start(child.id()).unwrap();
    let key = format!("detached-watchdog-{}", child.id());
    domain.bind_key(&key);
    child.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let detached: u32 = line.trim().parse().unwrap();
    let until = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !condition() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        condition()
    };
    let observed = until(&|| samples_for_key(&key).iter().any(|row| row.pid == detached));
    // The workload's own process ends first, as when a shell returns.
    let _ = child.kill();
    let _ = child.wait();
    domain.release(
        "detached-session",
        SessionProcessOwner::Agent("detached-agent".into()),
        None,
    );
    std::thread::sleep(4 * INTERVAL);
    let owner = SessionProcessOwner::Agent("detached-agent".into());
    let still_observed = released_keys_for_session("detached-session")
        .into_iter()
        .filter(|(row_owner, _)| *row_owner == owner)
        .flat_map(|(_, key)| samples_for_key(&key))
        .any(|row| row.pid == detached);
    stop_released(Some("detached-session"), Some(&owner));
    let ended = until(&|| !crate::test_support::process_alive(detached as i32));
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(detached as i32, libc::SIGKILL) };
    assert!(observed, "the detached child was missing from observations");
    assert!(
        still_observed,
        "the detached child was dropped when its parent ended"
    );
    assert!(ended, "the detached child outlived its owner");
}

#[cfg(unix)]
#[test]
fn watchdog_covers_children_created_after_shell_exec() {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "read ready; exec /bin/sh -c 'python3 -c \"import time; x=bytearray(48*1024*1024); time.sleep(30)\" & wait'"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let prepared = PreparedDomain::watchdog("watchdog-test", Some(24 * MIB), roomy());
    let domain = prepared.start(child.id()).unwrap();
    let key = format!("watchdog-exec-{}", child.id());
    domain.bind_key(&key);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while samples_for_key(&key).is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"exec-now\n")
        .unwrap();
    // The child over the cap is the command stopped; the shell waiting on it
    // then ends by itself.
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    let mut exited = child.try_wait().unwrap().is_some();
    while !exited && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
        exited = child.try_wait().unwrap().is_some();
    }
    let _ = child.kill();
    let _ = child.wait();
    drop(domain);
    assert!(
        exited,
        "the child created after exec was not stopped at the cap"
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn linux_controller_ends_a_runaway_before_the_oom_killer() {
    let prepared = PreparedDomain::in_pool("test-unit", Some(32 * MIB), roomy());
    let path = prepared
        .cgroup
        .as_ref()
        .expect("cgroup delegation required")
        .path()
        .to_owned();
    let (program,args)=prepared.command("python3".into(),vec!["-c".into(),"import time; blocks=[]\nfor n in range(16):\n blocks.append(bytearray(4*1024*1024)); time.sleep(.04)\ntime.sleep(30)".into()]);
    let mut child = std::process::Command::new(program)
        .args(args)
        .spawn()
        .unwrap();
    let domain = prepared.start(child.id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while domain.memory_limit_bytes().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let limit = domain.memory_limit_bytes();
    let mut exited = child.try_wait().unwrap().is_some();
    while !exited && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
        exited = child.try_wait().unwrap().is_some();
    }
    // The owner's unit carries the memory controller; the leaf only tells
    // workloads apart.
    let events = std::fs::read_to_string(path.parent().unwrap().join("memory.events")).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    drop(domain);
    assert!(
        exited,
        "the resource controller must end the runaway without test cleanup"
    );
    assert_eq!(
        limit,
        Some(32 * MIB),
        "the controller must name the configured limit"
    );
    assert!(
        events.lines().any(|line| line == "oom 0"),
        "controller must end the tree without kernel OOM: {events}"
    );
    assert!(
        events.lines().any(|line| line == "oom_kill 0"),
        "controller must own the stop: {events}"
    );
    assert!(!path.exists());
}

// What an owner started before a restart still counts against the limit its
// current workload runs under.
#[cfg(unix)]
#[test]
fn watchdog_counts_an_owners_workloads_together() {
    let hog = || {
        std::process::Command::new("python3")
            .args([
                "-c",
                "import time; x=bytearray(28*1024*1024); time.sleep(30)",
            ])
            .spawn()
            .unwrap()
    };
    let (mut first, mut second) = (hog(), hog());
    let pool = roomy();
    let first_domain = PreparedDomain::watchdog("one-owner", Some(48 * MIB), pool.clone())
        .start(first.id())
        .unwrap();
    let second_domain = PreparedDomain::watchdog("one-owner", Some(48 * MIB), pool)
        .start(second.id())
        .unwrap();
    // Callers bind each workload's run key, which is what the registry lists.
    first_domain.bind_key("owner-run-1");
    second_domain.bind_key("owner-run-2");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while first_domain.memory_limit_bytes().is_none()
        && second_domain.memory_limit_bytes().is_none()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    // Each workload alone is under the limit; the owner's total is not. Ending
    // one workload brings the total back under it, so only one need be stopped.
    let limited = (
        first_domain.memory_limit_bytes(),
        second_domain.memory_limit_bytes(),
    );
    let _ = first.kill();
    let _ = second.kill();
    let _ = first.wait();
    let _ = second.wait();
    assert!(
        limited.0 == Some(48 * MIB) || limited.1 == Some(48 * MIB),
        "the owner's workloads together passed the limit and none was stopped: {limited:?}"
    );
}

/// The parent the kernel's budget cgroups live in.
#[cfg(target_os = "linux")]
fn budget_parent() -> std::path::PathBuf {
    let current = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let relative = current
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap();
    std::path::Path::new("/sys/fs/cgroup")
        .join(relative.trim_start_matches('/'))
        .parent()
        .unwrap()
        .to_owned()
}

/// The previous program's domain for a hot reload: the kernel's PID is kept
/// across it, so it is a `chariox-<pid>-*` budget.
#[cfg(target_os = "linux")]
fn previous_domain() -> std::path::PathBuf {
    let path = budget_parent()
        .join(format!("chariox-{}-previous", std::process::id()))
        .join("unit-previous")
        .join("process-previous");
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn a_takeover_that_moved_only_some_processes_leaves_them_running() {
    let previous = previous_domain();
    let prepared = PreparedDomain::in_pool("partial-takeover-test", None, roomy());
    let destination = prepared
        .cgroup
        .as_ref()
        .expect("cgroup delegation required")
        .path()
        .to_owned();
    let unit = destination.parent().unwrap().to_owned();
    let budget = unit.parent().unwrap().to_owned();
    let mut workload = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    std::fs::write(previous.join("cgroup.procs"), workload.id().to_string()).unwrap();
    // The takeover moves the process, then runs out of passes before it can
    // see the old domain empty, as it does for a workload that keeps forking.
    cgroup::DRAIN_PASSES_OVERRIDE.store(1, std::sync::atomic::Ordering::Relaxed);
    let taken = prepared.taking_over(Some(&previous));
    cgroup::DRAIN_PASSES_OVERRIDE.store(8, std::sync::atomic::Ordering::Relaxed);
    let resident_watch = taken.cgroup.is_none();
    drop(taken);
    std::thread::sleep(Duration::from_millis(300));
    let alive = workload.try_wait().unwrap().is_none();
    let _ = workload.kill();
    let _ = workload.wait();
    let previous_unit = previous.parent().unwrap().to_owned();
    let previous_budget = previous_unit.parent().unwrap().to_owned();
    for directory in [
        destination,
        unit,
        budget,
        previous,
        previous_unit,
        previous_budget,
    ] {
        let _ = std::fs::remove_dir(directory);
    }
    assert!(
        resident_watch,
        "a failed takeover must fall back to resident memory"
    );
    assert!(alive, "a failed takeover ended the process it was adopting");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn a_new_budget_removes_budgets_left_by_ended_kernels() {
    let parent = budget_parent();
    let mut ended = std::process::Command::new("true").spawn().unwrap();
    let dead = ended.id();
    ended.wait().unwrap();
    let mut running = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let leftover = parent.join(format!("chariox-{dead}-1"));
    std::fs::create_dir_all(leftover.join("unit-1").join("process-1")).unwrap();
    let live = parent.join(format!("chariox-{}-1", running.id()));
    std::fs::create_dir(&live).unwrap();
    let prepared = PreparedDomain::in_pool("leftover-test", None, roomy());
    let removed = !leftover.exists();
    let kept = live.exists();
    drop(prepared);
    let _ = std::fs::remove_dir(&live);
    let _ = std::fs::remove_dir(leftover.join("unit-1").join("process-1"));
    let _ = std::fs::remove_dir(leftover.join("unit-1"));
    let _ = std::fs::remove_dir(&leftover);
    let _ = running.kill();
    let _ = running.wait();
    assert!(removed, "a budget left by an ended kernel stayed");
    assert!(kept, "a running kernel's budget was removed");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn an_ending_budget_removes_what_was_left_inside_it() {
    let prepared = PreparedDomain::in_pool("stale-test", None, roomy());
    let budget = prepared
        .cgroup
        .as_ref()
        .expect("cgroup delegation required")
        .path()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .to_owned();
    // A leaf whose processes outlived its removal deadline, emptied later.
    std::fs::create_dir(budget.join("unit-stale")).unwrap();
    drop(prepared);
    let left = budget.exists();
    let _ = std::fs::remove_dir(budget.join("unit-stale"));
    let _ = std::fs::remove_dir(&budget);
    assert!(!left, "the budget cgroup outlived its last workload");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn an_owner_without_a_cap_has_no_default_limit() {
    let prepared = PreparedDomain::in_pool("uncapped-test", None, roomy());
    let unit = prepared
        .cgroup
        .as_ref()
        .expect("cgroup delegation required")
        .path()
        .parent()
        .unwrap()
        .to_owned();
    let read = |name: &str| std::fs::read_to_string(unit.join(name)).unwrap();
    let (high, max, group) = (
        read("memory.high"),
        read("memory.max"),
        read("memory.oom.group"),
    );
    drop(prepared);
    assert_eq!(high.trim(), "max", "a default per-owner throttle");
    assert_eq!(max.trim(), "max", "a default per-owner limit");
    assert_eq!(
        group.trim(),
        "0",
        "a breach would end every process of the owner"
    );
}

/// Samples until `pid` is listed, for up to three seconds. A watch reads a
/// table the sampler published, and the kernel warms the sampler at startup, so
/// the first sample here waits for it too instead of failing with `Short`.
#[cfg(unix)]
fn sample_until_listed(tree: &mut sampling::ProcessTree, pid: u32) -> Vec<ProcessSample> {
    sampling::warm_up().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let samples = tree.sample().unwrap();
        if samples.iter().any(|sample| sample.pid == pid) || std::time::Instant::now() >= deadline {
            return samples;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn a_root_unreadable_at_start_is_listed_and_counted_once_readable() {
    // A process still executing its program is unreadable for a moment, as a
    // fresh terminal shell is on a host without cgroup delegation (macOS has
    // none): the watch has no cgroup to find the root in.
    for errno in [libc::EACCES, libc::ENOENT] {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut tree = {
            let _unreadable =
                crate::runtime::kernel_access::process::fault::inject(child.id(), errno);
            sampling::ProcessTree::new(child.id())
        };
        let samples = sample_until_listed(&mut tree, child.id());
        let commands = tree.commands(&samples);
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            samples.iter().any(|sample| sample.pid == child.id()),
            "errno {errno}: the root never appears in the feed"
        );
        assert!(
            commands
                .get(&child.id())
                .is_some_and(|(resident, _)| *resident > 0),
            "errno {errno}: the root's memory is not counted for limits and pressure: {commands:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_root_unreadable_for_one_sample_stays_owned() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut tree = sampling::ProcessTree::new(child.id());
    let before = sample_until_listed(&mut tree, child.id());
    let during = {
        let _unreadable =
            crate::runtime::kernel_access::process::fault::inject(child.id(), libc::ENOENT);
        tree.sample().unwrap()
    };
    let after = tree.sample().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert!(before.iter().any(|sample| sample.pid == child.id()));
    assert!(during.iter().any(|sample| sample.pid == child.id()));
    assert!(
        after.iter().any(|sample| sample.pid == child.id()),
        "one unreadable sample ended ownership of the root"
    );
}

#[cfg(unix)]
#[test]
fn a_sampler_short_of_resources_keeps_ownership_beyond_the_unreadable_grace() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut tree = sampling::ProcessTree::new(child.id());
    let before = sample_until_listed(&mut tree, child.id());
    let mut skipped = 0;
    let mut initial_roots = Vec::new();
    for errno in [libc::EMFILE, libc::ENFILE, libc::EAGAIN, libc::ENOMEM] {
        let mut initial = {
            let _fault = crate::runtime::kernel_access::process::fault::inject(child.id(), errno);
            let mut initial = sampling::ProcessTree::new(child.id());
            for _ in 0..16 {
                if matches!(tree.sample(), Err(sampling::SamplingError::Short(code)) if code == errno)
                {
                    skipped += 1;
                }
                let _ = initial.sample();
            }
            initial
        };
        initial_roots.push(
            initial
                .sample()
                .unwrap()
                .iter()
                .any(|sample| sample.pid == child.id()),
        );
    }
    let after = tree.sample().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        skipped, 64,
        "resource failures must skip the sample, not age ownership"
    );
    assert!(
        initial_roots.into_iter().all(|listed| listed),
        "a shortage abandoned an initially unreadable root"
    );
    assert!(
        before.iter().any(|old| after
            .iter()
            .any(|new| old.pid == child.id() && new.process_id == old.process_id)),
        "resource pressure discarded the process identity"
    );
}

/// Starting `ps` fails with `errno` until dropped.
#[cfg(unix)]
struct PsFailing(u32);
#[cfg(unix)]
impl PsFailing {
    /// Only the tree rooted at `root` sees the failure.
    fn with(root: u32, errno: i32) -> Self {
        sampling::ps_failure::set(root, Some(errno));
        Self(root)
    }
}
#[cfg(unix)]
impl Drop for PsFailing {
    fn drop(&mut self) {
        sampling::ps_failure::set(self.0, None);
    }
}

#[cfg(unix)]
fn watched_sleep(unit: &str) -> (std::process::Child, ProcessDomain, String) {
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let domain = PreparedDomain::watchdog(unit, None, roomy())
        .start(child.id())
        .unwrap();
    let key = format!("{unit}-{:016x}", rand::random::<u64>());
    domain.bind_key(&key);
    (child, domain, key)
}

/// A hot reload ends the children the kernel leaves behind; a sample running
/// then must not stop the tree it watches, which the new program adopts.
#[cfg(unix)]
#[test]
fn a_kernel_short_of_resources_for_a_moment_keeps_the_tree_it_watches() {
    let (mut child, _domain, key) = watched_sleep("descriptors-short");
    let pid = child.id();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !samples_for_key(&key).iter().any(|sample| sample.pid == pid) {
        assert!(std::time::Instant::now() < deadline, "never sampled");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut outcomes = Vec::new();
    for errno in [libc::EMFILE, libc::ENFILE, libc::EAGAIN, libc::ENOMEM] {
        let failing = PsFailing::with(pid, errno);
        std::thread::sleep(Duration::from_millis(350));
        let held = samples_for_key(&key);
        std::thread::sleep(Duration::from_secs(1));
        let kept = child.try_wait().unwrap().is_none();
        let unchanged = samples_for_key(&key) == held;
        drop(failing);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let sampled_again = loop {
            if samples_for_key(&key).iter().any(|sample| {
                sample.pid == pid
                    && held.iter().any(|old| {
                        old.process_id == sample.process_id
                            && old.sampled_at_ms < sample.sampled_at_ms
                    })
            }) {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        outcomes.push((errno, kept, unchanged, sampled_again));
    }
    let _ = child.kill();
    let _ = child.wait();
    for (errno, kept, unchanged, sampled_again) in outcomes {
        assert!(
            kept,
            "errno {errno}: sampler ended the tree because ps could not start"
        );
        assert!(
            unchanged,
            "errno {errno}: last known samples were changed during a shortage"
        );
        assert!(
            sampled_again,
            "errno {errno}: sampling did not resume with the same process identity"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_missing_ps_keeps_the_last_snapshot_without_ending_work() {
    let (mut child, _domain, key) = watched_sleep("ps-missing");
    let before = samples_for_key(&key);
    let failing = PsFailing::with(child.id(), libc::ENOENT);
    std::thread::sleep(Duration::from_millis(600));
    let alive = child.try_wait().unwrap().is_none();
    let after = samples_for_key(&key);
    drop(failing);
    let _ = child.kill();
    let _ = child.wait();
    assert!(alive, "a failed sampler refresh ended running work");
    assert_eq!(
        before, after,
        "failed refresh replaced the last good sample"
    );
}

/// A tree whose `ps` fails does not make the trees of other tests (or other
/// workloads) lose their samples or refuse to start.
#[cfg(unix)]
#[test]
fn a_ps_failure_for_one_tree_leaves_every_other_tree_alone() {
    let (mut failing_child, _failing_domain, _) = watched_sleep("fault-scoped");
    let (mut other, _other_domain, key) = watched_sleep("fault-other");
    let failing = PsFailing::with(failing_child.id(), libc::ENOENT);
    let mut third = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let started = PreparedDomain::watchdog("fault-third", None, roomy()).start(third.id());
    let first = samples_for_key(&key)
        .iter()
        .map(|sample| sample.sampled_at_ms)
        .max()
        .unwrap_or(0);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let sampled_on = loop {
        let latest = samples_for_key(&key)
            .iter()
            .map(|sample| sample.sampled_at_ms)
            .max()
            .unwrap_or(0);
        if latest > first || std::time::Instant::now() >= deadline {
            break latest > first;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let other_alive = other.try_wait().unwrap().is_none();
    drop(failing);
    for child in [&mut failing_child, &mut other, &mut third] {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(started.is_ok(), "another tree's failure refused a start");
    assert!(
        sampled_on,
        "another tree's failure stopped this tree's samples"
    );
    assert!(other_alive, "another tree's failure ended this tree");
}

// MP-08 / MP-09 / MP-10 / MP-11: disk is active-process I/O, never filesystem capacity.
#[cfg(target_os = "linux")]
#[test]
fn disk_io_sample_reports_real_owned_process_writes() {
    use std::io::Write;
    let file = std::env::temp_dir().join(format!("machines-disk-{:016x}", rand::random::<u64>()));
    let mut child = std::process::Command::new("python3")
        .args(["-c", "import os,sys; f=open(sys.argv[1],'wb'); f.write(b'x'*1048576); f.flush(); os.fsync(f.fileno()); print('ready',flush=True); sys.stdin.readline()"])
        .arg(&file).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::BufRead;
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let rows = sample_until_listed(&mut sampling::ProcessTree::new(child.id()), child.id());
    child.stdin.take().unwrap().write_all(b"done\n").unwrap();
    child.wait().unwrap();
    std::fs::remove_file(file).unwrap();
    let row = rows.iter().find(|row| row.pid == child.id()).unwrap();
    let wire = serde_json::to_value(row).unwrap();
    assert!(
        wire["disk_io_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes >= 1048576),
        "the shared process sample carries actual disk I/O: {wire}"
    );
}

/// The line a paused command is told reaches the pipe an agent's tool reads
/// the command's error stream from, not only a file.
#[cfg(target_os = "linux")]
#[test]
fn a_notice_reaches_the_pipe_a_tool_reads_the_error_stream_from() {
    use std::io::Read;
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let notice = "chariox: this command is paused while it grows for memory.\n";
    announce(child.id(), notice);
    let mut heard = vec![0; notice.len()];
    child.stderr.take().unwrap().read_exact(&mut heard).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(String::from_utf8(heard).unwrap(), notice);
}
