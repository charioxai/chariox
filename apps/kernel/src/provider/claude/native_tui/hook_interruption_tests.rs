use super::*;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn mp08_mp10_user_prompt_hook_reports_provider_termination() {
    let root = std::env::temp_dir().join(format!(
        "native-hook-interruption-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).unwrap();
    let handler = root.join("hook.mjs");
    let events = root.join("events.jsonl");
    fs::write(&handler, claude_native_hook_handler()).unwrap();
    fs::write(root.join("context"), "").unwrap();
    fs::create_dir(root.join("responses")).unwrap();
    let mut child = Command::new("node")
        .arg(&handler)
        .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &events)
        .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", root.join("context"))
        .env(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
            root.join("responses"),
        )
        .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"hook_event_name":"UserPromptSubmit","prompt":"Read the public documentation"}"#,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let initial = loop {
        if let Ok(raw) = fs::read_to_string(&events) {
            if let Some(line) = raw.lines().next() {
                break serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
        if Instant::now() >= deadline {
            assert!(child.id() > 1);
            child.kill().unwrap();
            child.wait().unwrap();
            fs::remove_dir_all(&root).unwrap();
            panic!("native prompt hook did not publish its admission request");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    // MP-11: only this test's exact owned positive process ID may receive a signal.
    assert!(child.id() > 1);
    assert!(Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap()
        .success());
    let status = child.wait().unwrap();
    let records: Vec<serde_json::Value> = fs::read_to_string(&events)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    fs::remove_dir_all(&root).unwrap();
    assert!(!status.success());
    let receipt = records
        .iter()
        .find(|r| r["hook_event_name"] == "UserPromptSubmitInterrupted")
        .expect("provider termination during the prompt hook needs an exact request receipt");
    assert_eq!(
        receipt["hook_context_request_id"],
        initial["hook_context_request_id"]
    );
    assert_eq!(receipt["signal"], "SIGTERM");
    assert!(receipt["at"].as_str().is_some());
    assert!(receipt.get("prompt").is_none());
}

#[test]
fn mp08_mp10_primary_hook_remains_alive_until_context_chunks_finish() {
    let root = std::env::temp_dir().join(format!(
        "native-hook-batch-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).unwrap();
    let handler = root.join("hook.mjs");
    let events = root.join("events.jsonl");
    fs::write(&handler, claude_native_hook_handler()).unwrap();
    fs::write(root.join("context"), "").unwrap();
    fs::create_dir(root.join("responses")).unwrap();
    let mut child = Command::new("node")
        .arg(&handler)
        .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &events)
        .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", root.join("context"))
        .env(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
            root.join("responses"),
        )
        .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK", "0")
        .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNKS", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"hook_event_name":"UserPromptSubmit","prompt":"Read the public documentation"}"#,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let request = loop {
        if let Ok(raw) = fs::read_to_string(&events) {
            if let Some(line) = raw.lines().next() {
                break serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
        if Instant::now() >= deadline {
            assert!(child.id() > 1);
            child.kill().unwrap();
            child.wait().unwrap();
            fs::remove_dir_all(&root).unwrap();
            panic!("native prompt hook did not publish its admission request");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let request_id = request["hook_context_request_id"].as_str().unwrap();
    fs::write(root.join("responses").join(format!("{request_id}.txt")), "").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let retained = child.try_wait().unwrap().is_none();
    if retained {
        assert!(child.id() > 1);
        assert!(Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status()
            .unwrap()
            .success());
    }
    child.wait().unwrap();
    let records: Vec<serde_json::Value> = fs::read_to_string(&events)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    fs::remove_dir_all(&root).unwrap();
    assert!(
        retained,
        "primary hook exited while its context chunk was still pending"
    );
    assert!(records
        .iter()
        .any(|r| r["hook_event_name"] == "UserPromptSubmitInterrupted"
            && r["hook_context_request_id"] == request["hook_context_request_id"]));
}
