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
    let prepared = PreparedDomain {
        limit: 24 * MIB,
        #[cfg(target_os = "linux")]
        cgroup: None,
    };
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
    let prepared = PreparedDomain::with_limit(32 * MIB).unwrap();
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
    let prepared = PreparedDomain {
        limit: 64 * MIB,
        #[cfg(target_os = "linux")]
        cgroup: None,
    };
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
    while domain.samples().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let id = domain
        .samples()
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
