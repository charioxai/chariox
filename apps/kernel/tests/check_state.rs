use std::fs;
use std::process::Command;

use chariox_kernel::durable_state::DurableKernelStateStore;
use serde_json::json;

#[test]
fn checks_durable_state_and_reports_the_failing_record_without_writing_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("copied-home");
    fs::create_dir(&home).unwrap();
    let database = home.join("state.db");
    fs::write(
        home.join("config.toml"),
        "[state]\npath = \"~/.chariox/state.db\"\n",
    )
    .unwrap();
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_chariox-kernel"))
            .env_clear()
            .env("TMPDIR", root.path())
            .env("CHARIOX_DAEMON_ID", "test-kernel")
            .args(["--check-state", "--home"])
            .arg(&home)
            .output()
            .unwrap()
    };

    let missing = check();
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("state.db"));
    assert!(!database.exists());

    let store = DurableKernelStateStore::open(database.clone()).unwrap();
    let before = fs::read(&database).unwrap();
    let healthy = check();
    assert_eq!(
        healthy.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&healthy.stderr)
    );
    assert_eq!(fs::read(&database).unwrap(), before);

    let event = store
        .append_event("session.created", Some("room-1".into()), json!({}))
        .unwrap();
    // Keep the writer open on this quiescent fixture so the failing record
    // remains in SQLite's write-ahead log.
    let before = home_files(&home);
    let failed = check();
    assert_eq!(failed.status.code(), Some(1));
    let error = String::from_utf8_lossy(&failed.stderr);
    for expected in [
        "session.created",
        &event.event_id,
        &format!("sequence {}", event.sequence),
        "Keep the current kernel",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(home_files(&home), before);
}

fn home_files(home: &std::path::Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut files = fs::read_dir(home)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
