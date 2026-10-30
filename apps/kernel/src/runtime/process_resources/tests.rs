use super::*;

#[test]
fn memory_override_rejects_invalid_and_overflowing_limits() {
    for value in ["", "0", "-1", "NaN", "15", "18446744073709551615"] {
        assert!(configured_limit(Some(value)).is_err(), "{value}");
    }
    assert_eq!(configured_limit(None).unwrap(), 16384 * MIB);
    assert_eq!(configured_limit(Some("64")).unwrap(), 64 * MIB);
}

#[cfg(unix)]
#[test]
fn watchdog_kills_only_its_owned_memory_tree_and_names_the_limit() {
    // Force the portable watchdog; the Linux cgroup path has its own drill.
    let prepared = PreparedDomain::watchdog("watchdog-test", 24 * MIB);
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
    let prepared = PreparedDomain::with_limit("test-unit", 32 * MIB).unwrap();
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
    let prepared = PreparedDomain::watchdog("watchdog-test", 64 * MIB);
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
    let prepared = PreparedDomain::with_limit("test-unit", 64 * MIB).unwrap();
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
    let prepared = PreparedDomain::watchdog("watchdog-test", 64 * MIB);
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

#[cfg(unix)]
#[test]
fn watchdog_covers_children_created_after_shell_exec() {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "read ready; exec /bin/sh -c 'python3 -c \"import time; x=bytearray(48*1024*1024); time.sleep(5)\" & wait'"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let prepared = PreparedDomain::watchdog("watchdog-test", 24 * MIB);
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
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while domain.memory_limit_bytes().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let limit = domain.memory_limit_bytes();
    let _ = child.kill();
    let _ = child.wait();
    drop(domain);
    assert_eq!(limit, Some(24 * MIB));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn linux_controller_ends_a_runaway_before_the_oom_killer() {
    let prepared = PreparedDomain::with_limit("test-unit", 32 * MIB).unwrap();
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
    let events = std::fs::read_to_string(path.join("memory.events")).unwrap();
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
    let first_domain = PreparedDomain::watchdog("one-owner", 48 * MIB)
        .start(first.id())
        .unwrap();
    let second_domain = PreparedDomain::watchdog("one-owner", 48 * MIB)
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
