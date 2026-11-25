use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{ChildProcess, RunningTools, ServerChildren, StoppedTool};
use crate::provider::opencode_client::OpenCodePart;

/// The children the fake server has right now.
#[derive(Clone, Default)]
struct Server(Arc<Mutex<Option<Vec<ChildProcess>>>>);

impl Server {
    fn with(children: Vec<ChildProcess>) -> Self {
        Self(Arc::new(Mutex::new(Some(children))))
    }
    fn set(&self, children: Option<Vec<ChildProcess>>) {
        *self.0.lock().unwrap() = children;
    }
}

impl ServerChildren for Server {
    fn children(&mut self, _port: u16) -> Option<Vec<ChildProcess>> {
        self.0.lock().unwrap().clone()
    }
}

fn child(pid: u32, started_ms: u64, argv: &[&str]) -> ChildProcess {
    ChildProcess {
        pid,
        started_ms,
        argv: argv.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn part(id: &str, status: &str, command: &str, started_ms: u64) -> OpenCodePart {
    serde_json::from_value(json!({
        "id": id, "sessionID": "s", "messageID": "m", "type": "tool", "tool": "bash",
        "state": {"status": status, "input": {"command": command}, "time": {"start": started_ms}},
    }))
    .unwrap()
}

/// A port of its own per test: processes paired to tools are kept by port.
fn tools(server: &Server) -> RunningTools {
    RunningTools::with_children(Box::new(server.clone()))
}

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

const COMMAND: &str = "python3 -c \"import time; time.sleep(240)\" mark";

#[test]
fn a_command_killed_while_opencode_shows_it_running_is_stopped_after_five_seconds() {
    let base = Instant::now();
    let server = Server::with(vec![child(
        10,
        1_000,
        &["python3", "-c", "import time; time.sleep(240)", "mark"],
    )]);
    let mut running = tools(&server);
    running.note(&part("prt_kill", "running", COMMAND, 1_000), base, 1_000);

    assert!(running.stopped(41_001, at(base, 3)).is_empty(), "alive");
    server.set(Some(Vec::new()));
    assert!(
        running.stopped(41_001, at(base, 4)).is_empty(),
        "gone for 0 s"
    );
    assert!(
        running.stopped(41_001, at(base, 8)).is_empty(),
        "gone for 4 s"
    );
    assert_eq!(
        running.stopped(41_001, at(base, 9)),
        vec![StoppedTool {
            part_id: "prt_kill".into(),
            input: json!({"command": COMMAND}),
        }]
    );
}

#[test]
fn a_command_that_ended_normally_is_never_stopped() {
    let base = Instant::now();
    let server = Server::with(Vec::new());
    let mut running = tools(&server);
    running.note(&part("prt_ok", "running", "echo hi", 1_000), base, 1_000);
    // OpenCode completes the part as soon as the process ends.
    running.note(
        &part("prt_ok", "completed", "echo hi", 1_000),
        at(base, 1),
        2_000,
    );

    assert!(running.is_empty());
    assert!(running.stopped(41_002, at(base, 30)).is_empty());
}

#[test]
fn a_running_command_with_its_process_alive_is_left_alone() {
    let base = Instant::now();
    let server = Server::with(vec![child(
        11,
        1_500,
        &["bash", "-c", "make -j8 && echo done"],
    )]);
    let mut running = tools(&server);
    running.note(
        &part("prt_live", "running", "make -j8 && echo done", 1_000),
        base,
        1_000,
    );

    for seconds in [3, 10, 60, 600] {
        assert!(running.stopped(41_003, at(base, seconds)).is_empty());
    }
}

#[test]
fn a_command_waiting_in_a_wrapper_shell_is_paired_by_its_command() {
    let base = Instant::now();
    let other = child(12, 1_100, &["bash", "-c", "cargo build"]);
    let mine = child(13, 1_200, &["bash", "-c", "sleep 100; echo $?"]);
    let server = Server::with(vec![other.clone(), mine]);
    let mut running = tools(&server);
    running.note(
        &part("prt_wrap", "running", "sleep 100; echo $?", 1_000),
        base,
        1_000,
    );

    assert!(running.stopped(41_004, at(base, 3)).is_empty());
    // Another command's process stays; this tool's own process is gone.
    server.set(Some(vec![other]));
    assert!(running.stopped(41_004, at(base, 4)).is_empty());
    assert_eq!(running.stopped(41_004, at(base, 10)).len(), 1);
}

#[test]
fn two_processes_that_cannot_be_told_apart_leave_the_command_alone() {
    let base = Instant::now();
    let server = Server::with(vec![
        child(14, 1_100, &["make"]),
        child(15, 1_200, &["node", "server.js"]),
    ]);
    let mut running = tools(&server);
    running.note(
        &part("prt_two", "running", "FOO=1 make", 1_000),
        base,
        1_000,
    );

    for seconds in [3, 10, 100] {
        assert!(running.stopped(41_005, at(base, seconds)).is_empty());
    }
}

#[test]
fn a_process_that_started_before_the_command_is_not_its_process() {
    let base = Instant::now();
    // An MCP server that has been up for an hour.
    let server = Server::with(vec![child(16, 1_000, &["npx", "playwright-mcp"])]);
    let mut running = tools(&server);
    running.note(
        &part("prt_early", "running", COMMAND, 3_600_000),
        base,
        3_600_000,
    );

    assert!(running.stopped(41_006, at(base, 3)).is_empty());
    assert_eq!(running.stopped(41_006, at(base, 9)).len(), 1);
}

#[test]
fn a_leaf_program_is_paired_with_its_command_despite_the_quotes() {
    let base = Instant::now();
    let server = Server::with(vec![
        child(
            17,
            1_100,
            &["python3", "-c", "import time; time.sleep(240)", "mark"],
        ),
        child(
            18,
            1_150,
            &["python3", "-c", "import time; time.sleep(9)", "other"],
        ),
    ]);
    let mut running = tools(&server);
    running.note(&part("prt_leaf", "running", COMMAND, 1_000), base, 1_000);
    assert!(running.stopped(41_007, at(base, 3)).is_empty());

    server.set(Some(vec![child(
        18,
        1_150,
        &["python3", "-c", "import time; time.sleep(9)", "other"],
    )]));
    assert!(running.stopped(41_007, at(base, 4)).is_empty());
    assert_eq!(running.stopped(41_007, at(base, 10)).len(), 1);
}

#[test]
fn a_server_that_cannot_be_read_stops_nothing() {
    let base = Instant::now();
    let server = Server::default();
    let mut running = tools(&server);
    running.note(&part("prt_blind", "running", COMMAND, 1_000), base, 1_000);

    for seconds in [3, 10, 100] {
        assert!(running.stopped(41_008, at(base, seconds)).is_empty());
    }
}

#[test]
fn another_commands_process_is_not_paired_twice() {
    let base = Instant::now();
    let server = Server::with(vec![child(19, 1_100, &["make"])]);
    let mut first = tools(&server);
    first.note(&part("prt_first", "running", "make", 1_000), base, 1_000);
    assert!(first.stopped(41_009, at(base, 3)).is_empty());

    // A second session's command has no process of its own: the one process
    // belongs to the first, so the second is stopped once it has been gone.
    let mut second = tools(&server);
    second.note(
        &part("prt_second", "running", "FOO=1 other", 1_000),
        base,
        1_000,
    );
    assert!(second.stopped(41_009, at(base, 4)).is_empty());
    assert_eq!(second.stopped(41_009, at(base, 10)).len(), 1);
    assert!(first.stopped(41_009, at(base, 11)).is_empty());
}

#[test]
fn a_tool_younger_than_two_seconds_is_not_looked_at() {
    let base = Instant::now();
    let server = Server::with(Vec::new());
    let mut running = tools(&server);
    running.note(&part("prt_new", "running", COMMAND, 1_000), base, 1_000);

    assert!(running.stopped(41_010, at(base, 1)).is_empty());
}

/// Review of #1745, P1: the same command twice on one server. Neither process
/// can be told from the other, so neither tool may take either one.
#[test]
fn identical_commands_are_never_paired_by_process_order() {
    let base = Instant::now();
    // The second command's process is listed first: the shorter one ends normally.
    let longer = child(30, 1_100, &["bash", "-c", "cargo build"]);
    let shorter = child(31, 1_200, &["bash", "-c", "cargo build"]);
    let server = Server::with(vec![longer.clone(), shorter]);
    let mut running = tools(&server);
    running.note(&part("prt_a", "running", "cargo build", 1_000), base, 1_000);
    running.note(&part("prt_b", "running", "cargo build", 1_000), base, 1_000);
    assert!(running.stopped(41_020, at(base, 3)).is_empty());

    // The shorter one ends normally and OpenCode completes its tool; the longer
    // one is still running with its own process.
    server.set(Some(vec![longer]));
    running.note(
        &part("prt_a", "completed", "cargo build", 1_000),
        at(base, 4),
        4_000,
    );
    for seconds in [5, 10, 20, 60] {
        assert!(
            running.stopped(41_020, at(base, seconds)).is_empty(),
            "the longer command's process is alive (after {seconds} s)"
        );
    }
}

/// Review of #1745, P1: another session's identical command is not this tool's
/// process, even when it is the only candidate.
#[test]
fn another_sessions_identical_command_is_not_taken_as_this_tools_process() {
    let base = Instant::now();
    let theirs = child(32, 1_100, &["bash", "-c", "cargo build"]);
    let server = Server::with(vec![theirs]);
    let mut dead = tools(&server);
    let mut alive = tools(&server);
    dead.note(&part("prt_dead", "running", "cargo build", 1_000), base, 1_000);
    alive.note(&part("prt_live", "running", "cargo build", 1_000), base, 1_000);

    // The first tool has no process of its own; the one process is the other's.
    for seconds in [3, 10, 20] {
        assert!(dead.stopped(41_021, at(base, seconds)).is_empty());
        assert!(
            alive.stopped(41_021, at(base, seconds)).is_empty(),
            "the live command is never stopped (after {seconds} s)"
        );
    }
}

/// Review of #1745, P1: a lone unclaimed child is no proof of ownership.
#[test]
fn a_lone_unrelated_child_is_not_taken_as_this_tools_process() {
    let base = Instant::now();
    let server = Server::with(vec![child(33, 1_100, &["node", "server.js"])]);
    let mut running = tools(&server);
    running.note(&part("prt_lone", "running", "make", 1_000), base, 1_000);
    assert!(running.stopped(41_022, at(base, 3)).is_empty());
    // The unrelated process ends; the tool's own was never there.
    server.set(Some(Vec::new()));
    assert!(running.stopped(41_022, at(base, 4)).is_empty());
    assert_eq!(running.stopped(41_022, at(base, 10)).len(), 1);
}
