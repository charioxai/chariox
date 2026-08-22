use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;

fn node_stdio_config(script: &str) -> Option<CharioxMcpServerConfig> {
    if Command::new("node").arg("--version").output().is_err() {
        return None;
    }
    Some(CharioxMcpServerConfig::stdio(
        format!("stdio-concurrency-test-{}", crate::session::unix_epoch_ms()),
        "node",
        vec![
            "--input-type=module".to_string(),
            "-e".to_string(),
            script.to_string(),
        ],
    ))
}

fn process_for(
    supervisor: &mut StdioMcpSupervisor,
    provider_run_id: &str,
    session_id: &str,
    config: &CharioxMcpServerConfig,
) -> Arc<Mutex<StdioMcpProcess>> {
    let key = config.definition_hash().expect("definition hash");
    supervisor
        .process(&key, provider_run_id, session_id, config)
        .expect("stdio MCP process should start")
}

#[test]
fn request_waits_for_matching_response_after_notification_and_stale_id() {
    let Some(config) = node_stdio_config(
        r#"
let buffer = Buffer.alloc(0)
function write(message) { process.stdout.write(`${JSON.stringify(message)}\n`) }
process.stdin.on('data', (chunk) => {
  buffer = Buffer.concat([buffer, chunk])
  while (true) {
    const newline = buffer.indexOf('\n')
    if (newline < 0) return
    const line = buffer.subarray(0, newline).toString('utf8').trim()
    buffer = buffer.subarray(newline + 1)
    if (!line) continue
    const message = JSON.parse(line)
    write({ jsonrpc: '2.0', method: 'notifications/progress', params: { progress: 1 } })
    write({ jsonrpc: '2.0', id: 'stale-request', result: { stale: true } })
    write({ jsonrpc: '2.0', id: message.id, result: { matched: true } })
  }
})
"#,
    ) else {
        return;
    };
    let mut supervisor = StdioMcpSupervisor::default();
    let process = process_for(
        &mut supervisor,
        "provider-run-match",
        "session-match",
        &config,
    );

    let response = process
        .lock()
        .expect("process lock")
        .dispatch(json!({"jsonrpc": "2.0", "id": "wanted", "method": "tools/call"}))
        .expect("matching response should be returned");

    assert_eq!(response.get("id"), Some(&json!("wanted")));
    assert_eq!(response.pointer("/result/matched"), Some(&json!(true)));
    drop(process);
    supervisor.stop_all();
}

#[test]
fn busy_process_lookup_does_not_hold_the_supervisor_lock() {
    let Some(config) = node_stdio_config("process.stdin.resume()") else {
        return;
    };
    let supervisor = Arc::new(Mutex::new(StdioMcpSupervisor::default()));
    let process = {
        let mut supervisor = supervisor.lock().expect("supervisor lock");
        process_for(
            &mut supervisor,
            "provider-run-busy-a",
            "session-busy-a",
            &config,
        )
    };
    let process_guard = process.lock().expect("process lock");
    let lookup_supervisor = Arc::clone(&supervisor);
    let lookup_config = config.clone();
    let (completed_tx, completed_rx) = mpsc::channel();
    let lookup = std::thread::spawn(move || {
        let key = lookup_config.definition_hash().expect("definition hash");
        let result = lookup_supervisor
            .lock()
            .expect("supervisor lock")
            .process(
                &key,
                "provider-run-busy-b",
                "session-busy-b",
                &lookup_config,
            )
            .is_ok();
        let _ = completed_tx.send(result);
    });

    let completed_without_process_lock = completed_rx
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or(false);
    let shutdown_started = Instant::now();
    let released = supervisor
        .lock()
        .expect("supervisor lock")
        .release_run("provider-run-busy-b");
    let shutdown_elapsed = shutdown_started.elapsed();

    drop(process_guard);
    lookup.join().expect("lookup thread should stop");
    drop(process);
    drop(released);
    supervisor.lock().expect("supervisor lock").stop_all();
    assert!(
        completed_without_process_lock,
        "liveness lookup must not wait on a busy process while holding the supervisor lock"
    );
    assert!(
        shutdown_elapsed < Duration::from_millis(500),
        "run shutdown must not wait behind a process mutex held by a tool call"
    );
}

#[test]
fn closed_run_and_session_reject_late_process_ownership() {
    let config = CharioxMcpServerConfig::stdio(
        "stdio-closed-owner-test",
        "command-must-not-run",
        Vec::new(),
    );
    let key = config.definition_hash().expect("definition hash");
    let mut supervisor = StdioMcpSupervisor::default();

    assert!(supervisor.release_run("provider-run-closed").is_empty());
    let run_error = supervisor.process(&key, "provider-run-closed", "session-open", &config);
    assert!(
        run_error.is_err(),
        "a released run must not reacquire ownership"
    );

    assert!(supervisor.release_session("session-closed").is_empty());
    let session_error = supervisor.process(&key, "provider-run-open", "session-closed", &config);
    assert!(
        session_error.is_err(),
        "a released session must not reacquire ownership"
    );
    assert_eq!(supervisor.process_count(), 0);
}

#[test]
fn request_timeout_kills_the_unresponsive_stdio_child() {
    let Some(mut config) = node_stdio_config(
        r#"
import fs from 'node:fs'
fs.writeFileSync(process.env.CHARIOX_TEST_PID_FILE, String(process.pid))
process.stdin.resume()
"#,
    ) else {
        return;
    };
    let pid_file = std::env::temp_dir().join(format!(
        "chariox-stdio-timeout-{}-{}.pid",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    config.tool_timeout_sec = Some(1);
    if let CharioxMcpTransportConfig::Stdio { env, .. } = &mut config.transport {
        env.insert(
            "CHARIOX_TEST_PID_FILE".to_string(),
            pid_file.to_string_lossy().to_string(),
        );
    }
    let mut supervisor = StdioMcpSupervisor::default();
    let process = process_for(
        &mut supervisor,
        "provider-run-timeout",
        "session-timeout",
        &config,
    );
    let pid_deadline = Instant::now() + Duration::from_secs(2);
    let child_pid = loop {
        if let Ok(contents) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = contents.parse::<u32>() {
                break pid;
            }
        }
        assert!(
            Instant::now() < pid_deadline,
            "stdio child should write a complete PID before the deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let dispatch_process = Arc::clone(&process);
    let (result_tx, result_rx) = mpsc::channel();
    let started = Instant::now();
    let dispatch = std::thread::spawn(move || {
        let result = dispatch_process
            .lock()
            .expect("process lock")
            .dispatch(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call"}));
        let _ = result_tx.send(result.map(|_| ()));
    });
    let result = match result_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(error) => {
            #[cfg(unix)]
            let _ = crate::runtime::process_health::terminate_process_tree(child_pid);
            dispatch
                .join()
                .expect("dispatch thread should stop after kill");
            panic!("stdio request timeout did not fire: {error}");
        }
    };
    let elapsed = started.elapsed();
    dispatch.join().expect("dispatch thread should stop");

    let message = result
        .expect_err("unresponsive stdio MCP should time out")
        .to_string();
    assert!(message.contains("did not respond within 1 seconds"));
    assert!(elapsed >= Duration::from_millis(900));
    assert!(elapsed < Duration::from_secs(2));
    #[cfg(unix)]
    assert!(
        !crate::runtime::process_health::process_running(child_pid),
        "timed-out stdio MCP child must be killed and reaped"
    );
    drop(process);
    supervisor.stop_all();
    let _ = std::fs::remove_file(pid_file);
}
