use super::*;
use std::collections::BTreeMap;
mod hosted;

fn limits() -> BTreeMap<&'static str, String> {
    [
        ("memory.max", MEMORY_BYTES.to_string()),
        ("memory.swap.max", "0".into()),
        ("memory.oom.group", "1".into()),
        ("pids.max", TASKS.to_string()),
        ("cpu.max", "100000 100000".into()),
    ]
    .into_iter()
    .collect()
}

#[test]
fn finite_exact_limits_reject_widened_memory_swap_cpu_or_tasks() {
    assert!(policy::limits(&limits()).is_ok());
    for (name, value) in [
        ("memory.max", "max"),
        ("memory.max", "1073741824"),
        ("memory.swap.max", "1"),
        ("memory.oom.group", "0"),
        ("pids.max", "65"),
        ("cpu.max", "200000 100000"),
    ] {
        let mut values = limits();
        values.insert(name, value.into());
        assert!(policy::limits(&values).is_err(), "{name}");
    }
    for missing in limits().keys() {
        let mut values = limits();
        values.remove(missing);
        assert!(policy::limits(&values).is_err());
    }
}

#[test]
fn bounded_kernel_records_reject_duplicates_ambiguity_and_old_baselines() {
    for invalid in ["", "00", "+1", "-1", "1\n2", "max", "18446744073709551616"] {
        assert!(policy::number(invalid).is_err());
    }
    assert!(policy::fields("populated 0\npopulated 1\n", ' ').is_err());
    assert!(policy::fields(&format!("x {}", "a".repeat(32768)), ' ').is_err());
    for invalid in ["1\n", "12\n12\n", "2147483648\n", "01\n"] {
        assert!(policy::pids(invalid).is_err());
    }
    assert_eq!(policy::pids("101\n102\n").unwrap(), [101, 102]);
    assert!(policy::kernel("6.1.0-debian\n").is_ok());
    assert!(policy::kernel("6.8.0-ubuntu\n").is_ok());
    assert!(policy::kernel("5.15.0\n").is_err());
}

#[test]
fn worker_identity_requires_private_pid_one_zero_authority_and_exact_parent() {
    let status = "Name:\tchariox-app\nPid:\t102\nPPid:\t101\nUid:\t1000 1000 1000 1000\nGid:\t1000 1000 1000 1000\nGroups:\t\nNoNewPrivs:\t1\nSeccomp:\t2\nNSpid:\t102 1\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapAmb:\t0000000000000000\n";
    assert!(policy::worker_status(status, 102, 101, 1000, 1000).is_ok());
    for (old, new) in [
        ("PPid:\t101", "PPid:\t103"),
        ("Groups:\t\n", "Groups:\t10\n"),
        ("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
        ("Seccomp:\t2", "Seccomp:\t0"),
        ("NSpid:\t102 1", "NSpid:\t102"),
        ("CapEff:\t0000000000000000", "CapEff:\t0000000000000001"),
    ] {
        assert!(
            policy::worker_status(&status.replace(old, new), 102, 101, 1000, 1000).is_err(),
            "{old}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn mount_observation_requires_each_exact_root_without_duplicate_shadow_mounts() {
    let text = [
        "1 0 0:1 / / ro - tmpfs tmpfs ro",
        "2 1 0:2 / /app/package ro,nosuid,nodev,noexec - tmpfs tmpfs ro",
        "3 1 0:3 / /app/data rw,nosuid,nodev,noexec - ext4 data rw",
        "4 1 0:4 / /app/tmp rw,nosuid,nodev,noexec - tmpfs tmpfs rw",
        "5 1 0:5 / /runtime ro,nosuid,nodev - ext4 runtime ro",
    ]
    .join("\n");
    assert!(inspection::mounts(&text).is_ok());
    assert!(
        inspection::mounts(&text.replace("rw,nosuid,nodev,noexec", "rw,nosuid,nodev")).is_err()
    );
    assert!(inspection::mounts(&format!("{text}\n1 0 0:1 / / ro - tmpfs tmpfs ro")).is_err());
}
