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
    }
}

fn learn(admission: &LaunchAdmission, argv: &[String], cwd: &Path, memory: u64) {
    let key = admission::seen_argv(0, argv, Some(cwd.to_path_buf())).key;
    admission.0.ledger.lock().unwrap().peaks.remember(
        &key,
        admission::Peak { memory, writes: 0 },
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
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
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
