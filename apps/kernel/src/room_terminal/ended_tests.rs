//! No terminal limit; a one-command terminal goes when its command is over and read; any other
//! ended terminal goes after its retention. Real PTYs; the clock jumps over the waiting.

use std::path::Path;
use std::time::{Duration, Instant};

use super::service::{AGENT_ENDED_RETENTION, ONE_COMMAND_UNREAD_RETENTION};
use super::*;
use crate::test_support::TestWorktree;

fn service(root: &Path) -> RoomTerminalService {
    RoomTerminalService::new(root.join("terminals")).unwrap()
}

fn open_command(
    service: &RoomTerminalService,
    scope: TerminalScope,
    root: &Path,
    opened_by: TerminalActor,
    script: &str,
) -> String {
    service
        .open(OpenRoomTerminal {
            stdin: None,
            project_environment: Default::default(),
            scope,
            working_directory: root.to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
        })
        .expect("terminal opens")
        .terminal_id
}

fn stored(root: &Path, id: &str) -> bool {
    root.join("terminals").join(format!("{id}.json")).exists()
}

async fn until_ended(service: &RoomTerminalService, scope: &TerminalScope, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        service.pump_once();
        let ended = service
            .list(scope.clone())
            .iter()
            .any(|t| t.terminal_id == id && matches!(t.status, RoomTerminalStatus::Ended { .. }));
        if ended {
            return;
        }
        assert!(Instant::now() < deadline, "terminal {id} never ended");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn ids(service: &RoomTerminalService, scope: &TerminalScope) -> Vec<String> {
    service
        .list(scope.clone())
        .into_iter()
        .map(|t| t.terminal_id)
        .collect()
}

#[tokio::test]
async fn a_room_holds_twenty_terminals() {
    let root = TestWorktree::new("twenty-terminals");
    let service = service(root.path());
    let room = TerminalScope::room("room-1");
    let opened: Vec<_> = (0..20)
        .map(|_| {
            open_command(
                &service,
                room.clone(),
                root.path(),
                TerminalActor::Agent("agent-1".into()),
                "sleep 600",
            )
        })
        .collect();
    assert_eq!(ids(&service, &room).len(), 20);
    for id in &opened {
        assert!(service.snapshot(room.clone(), id).is_ok());
    }
    service.end_scope(room);
}

#[tokio::test]
async fn a_one_command_kernel_terminal_goes_when_its_output_was_read_to_the_end() {
    let root = TestWorktree::new("one-command-read");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    let id = open_command(
        &service,
        kernel.clone(),
        root.path(),
        TerminalActor::User,
        "echo hi",
    );
    until_ended(&service, &kernel, &id).await;
    // Not read yet: still there, with its output.
    assert_eq!(ids(&service, &kernel), vec![id.clone()]);
    let output = service
        .read_output(kernel.clone(), &id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert!(!output.data_base64.is_empty());
    assert!(
        ids(&service, &kernel).is_empty(),
        "read to the end, still listed"
    );
    assert!(!stored(root.path(), &id), "its record stays in the store");
    // The client's own close, after the kernel removed it: a refusal that names the terminal.
    let closed = service.close(kernel, &id, &TerminalActor::User);
    assert!(closed.is_err());
}

#[tokio::test]
async fn a_page_that_stops_short_of_the_end_leaves_the_one_command_terminal_until_the_last_page() {
    let root = TestWorktree::new("one-command-partial");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    // More than one page of output (a page is at most 256 KiB).
    let id = open_command(
        &service,
        kernel.clone(),
        root.path(),
        TerminalActor::User,
        "seq 1 120000",
    );
    until_ended(&service, &kernel, &id).await;
    let first = service
        .read_output(kernel.clone(), &id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert!(
        first.next_offset < first.terminal.output_offset,
        "the output fits one page"
    );
    assert_eq!(
        ids(&service, &kernel),
        vec![id.clone()],
        "removed before the last page"
    );
    let mut offset = first.next_offset;
    while offset < first.terminal.output_offset {
        offset = service
            .read_output(kernel.clone(), &id, offset, Duration::ZERO)
            .await
            .unwrap()
            .next_offset;
    }
    assert!(
        ids(&service, &kernel).is_empty(),
        "still listed after the last page"
    );
}

#[tokio::test]
async fn an_unread_one_command_terminal_goes_a_minute_after_it_ended() {
    let root = TestWorktree::new("one-command-unread");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    let id = open_command(
        &service,
        kernel.clone(),
        root.path(),
        TerminalActor::User,
        "echo hi",
    );
    until_ended(&service, &kernel, &id).await;
    service.advance_clock_for_test(ONE_COMMAND_UNREAD_RETENTION - Duration::from_secs(10));
    assert_eq!(
        ids(&service, &kernel),
        vec![id.clone()],
        "gone before its minute"
    );
    service.advance_clock_for_test(Duration::from_secs(15));
    assert!(
        ids(&service, &kernel).is_empty(),
        "still listed after its minute"
    );
    assert!(!stored(root.path(), &id));
}

#[tokio::test]
async fn a_running_one_command_terminal_is_never_cleaned_up() {
    let root = TestWorktree::new("one-command-running");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    let id = open_command(
        &service,
        kernel.clone(),
        root.path(),
        TerminalActor::User,
        "sleep 600",
    );
    service.advance_clock_for_test(AGENT_ENDED_RETENTION * 2);
    assert_eq!(ids(&service, &kernel), vec![id]);
    service.end_scope(kernel);
}

fn agent() -> TerminalActor {
    TerminalActor::Agent("agent-1".into())
}

#[tokio::test]
async fn an_ended_agent_terminal_nobody_used_is_listed_then_removed_after_its_hour() {
    let root = TestWorktree::new("ended-agent");
    let service = service(root.path());
    let room = TerminalScope::room("room-1");
    let id = open_command(&service, room.clone(), root.path(), agent(), "exit 0");
    until_ended(&service, &room, &id).await;
    // Reading it all does not remove a Room terminal: its way of ending stays to be seen.
    service
        .read_output(room.clone(), &id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(ids(&service, &room), vec![id.clone()]);
    service.advance_clock_for_test(AGENT_ENDED_RETENTION - Duration::from_secs(60));
    assert_eq!(
        ids(&service, &room),
        vec![id.clone()],
        "gone before its hour"
    );
    service.advance_clock_for_test(Duration::from_secs(120));
    assert!(
        ids(&service, &room).is_empty(),
        "still listed after its hour"
    );
    assert!(!stored(root.path(), &id));
}

#[tokio::test]
async fn an_ended_terminal_the_user_opened_is_never_removed_by_the_kernel() {
    let root = TestWorktree::new("ended-user-opened");
    let service = service(root.path());
    let room = TerminalScope::room("room-1");
    let id = open_command(
        &service,
        room.clone(),
        root.path(),
        TerminalActor::User,
        "exit 0",
    );
    until_ended(&service, &room, &id).await;
    service.advance_clock_for_test(AGENT_ENDED_RETENTION * 3);
    assert_eq!(
        ids(&service, &room),
        vec![id.clone()],
        "removed behind the user's back"
    );
    assert!(stored(root.path(), &id));
    // Only the user closes it.
    service
        .close(room.clone(), &id, &TerminalActor::User)
        .unwrap();
    assert!(ids(&service, &room).is_empty());
}

#[tokio::test]
async fn an_agent_terminal_a_human_typed_into_survives_its_end_and_only_the_human_closes_it() {
    let root = TestWorktree::new("ended-human-used");
    let service = service(root.path());
    let room = TerminalScope::room("room-1");
    let running = open_command(&service, room.clone(), root.path(), agent(), "sleep 600");
    let used = open_command(
        &service,
        room.clone(),
        root.path(),
        agent(),
        "sleep 1; exit 0",
    );
    for id in [&running, &used] {
        service
            .input(room.clone(), id, &TerminalActor::User, b"x")
            .await
            .unwrap();
    }
    // A running tab the user used: an agent cannot close it.
    let refused = service.close(room.clone(), &running, &agent()).unwrap_err();
    assert!(
        refused.to_string().contains("only the user can close it"),
        "{refused}"
    );
    until_ended(&service, &room, &used).await;
    service.advance_clock_for_test(AGENT_ENDED_RETENTION * 3);
    let left = ids(&service, &room);
    assert!(left.contains(&used) && left.contains(&running), "{left:?}");
    assert!(service.close(room.clone(), &used, &agent()).is_err());
    service
        .close(room.clone(), &used, &TerminalActor::User)
        .unwrap();
    service.end_scope(room);
}

#[tokio::test]
async fn a_terminal_that_ended_before_a_restart_is_cleaned_up_on_time_when_the_kernel_tidies_it() {
    let root = TestWorktree::new("restart-cleanup");
    let room = TerminalScope::room("room-1");
    let first = service(root.path());
    let agent_ended = open_command(&first, room.clone(), root.path(), agent(), "exit 0");
    let user_ended = open_command(
        &first,
        room.clone(),
        root.path(),
        TerminalActor::User,
        "exit 0",
    );
    let agent_running = open_command(&first, room.clone(), root.path(), agent(), "sleep 600");
    until_ended(&first, &room, &agent_ended).await;
    until_ended(&first, &room, &user_ended).await;
    // Let the periodic save write the ended records.
    let deadline = Instant::now() + Duration::from_secs(10);
    while ![&agent_ended, &user_ended].iter().all(|id| {
        std::fs::read_to_string(root.path().join("terminals").join(format!("{id}.json"))).is_ok_and(
            |text| text.contains("\"ended_at_ms\":") && !text.contains("\"ended_at_ms\":null"),
        )
    }) {
        assert!(
            Instant::now() < deadline,
            "the ended records were not saved"
        );
        first.pump_once();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    first.shutdown();
    drop(first);

    // The restart ends the running one as well; all three come back ended, each on its own clock.
    let second = service(root.path());
    let listed = ids(&second, &room);
    assert_eq!(listed.len(), 3, "{listed:?}");
    second.advance_clock_for_test(AGENT_ENDED_RETENTION - Duration::from_secs(60));
    assert_eq!(ids(&second, &room).len(), 3, "gone before its hour");
    second.advance_clock_for_test(Duration::from_secs(120));
    assert_eq!(
        ids(&second, &room),
        vec![user_ended.clone()],
        "agent terminals stay after their hour"
    );
    assert!(stored(root.path(), &user_ended));
    assert!(!stored(root.path(), &agent_ended) && !stored(root.path(), &agent_running));
}

#[tokio::test]
async fn an_agent_terminal_already_past_its_hour_at_startup_is_gone_before_the_first_list() {
    let root = TestWorktree::new("startup-due");
    let room = TerminalScope::room("room-1");
    let first = service(root.path());
    let id = open_command(&first, room.clone(), root.path(), agent(), "exit 0");
    until_ended(&first, &room, &id).await;
    let path = root.path().join("terminals").join(format!("{id}.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(&path).is_ok_and(|text| text.contains("\"ended\":{")) {
        assert!(Instant::now() < deadline, "the ended record was not saved");
        first.pump_once();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    first.shutdown();
    drop(first);
    // Rewrite the end time to two hours ago.
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["ended_at_ms"] = serde_json::json!(crate::session::unix_epoch_ms() - 2 * 3_600_000);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let second = service(root.path());
    assert!(
        second.lock().terminals.is_empty(),
        "due at startup, still held"
    );
    assert!(!path.exists());
}

fn job_running(pid_file: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|pid| pid.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    else {
        return false;
    };
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()
        .is_some_and(|output| {
            let status = String::from_utf8_lossy(&output.stdout);
            !status.trim().is_empty() && !status.trim().starts_with('Z')
        })
}

fn kill_job(pid_file: &Path) {
    if let Some(pid) = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    {
        // SAFETY: the positive PID of the job this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// A command that starts a detached job and exits: the terminal ends, the job keeps running.
async fn open_with_detached_job(
    service: &RoomTerminalService,
    scope: &TerminalScope,
    root: &Path,
    opened_by: TerminalActor,
    pid_file: &Path,
) -> String {
    let script = format!(
        "nohup {} </dev/null >/dev/null 2>&1 &",
        crate::test_support::detached_job(&format!("'{}'", pid_file.display()), 8895),
    );
    let id = open_command(service, scope.clone(), root, opened_by, &script);
    until_ended(service, scope, &id).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !job_running(pid_file) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(job_running(pid_file), "the detached job did not start");
    id
}

async fn job_ends_after(pid_file: &Path, stop: impl FnOnce()) -> bool {
    stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while job_running(pid_file) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    !job_running(pid_file)
}

#[tokio::test]
async fn an_unread_one_command_terminal_going_away_leaves_the_job_it_left_running() {
    let root = TestWorktree::new("expiry-keeps-job-kernel");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    let pid_file = root.path().join("kernel-expiry.pid");
    let id = open_with_detached_job(
        &service,
        &kernel,
        root.path(),
        TerminalActor::User,
        &pid_file,
    )
    .await;
    service.advance_clock_for_test(ONE_COMMAND_UNREAD_RETENTION + Duration::from_secs(1));
    assert!(
        ids(&service, &kernel).is_empty(),
        "the terminal is still listed"
    );
    assert!(!stored(root.path(), &id));
    let alive = job_running(&pid_file);
    // Still stoppable: the kernel's own stop path for what its terminals left running.
    let stopped = job_ends_after(&pid_file, || {
        crate::runtime::process_resources::end_owner(Some(kernel.process_key()), None)
    })
    .await;
    kill_job(&pid_file);
    assert!(
        alive,
        "removing the ended terminal ended the job it left running"
    );
    assert!(stopped, "the job left behind cannot be stopped");
}

#[tokio::test]
async fn reading_a_one_command_terminal_to_the_end_leaves_the_job_it_left_running() {
    let root = TestWorktree::new("read-keeps-job-kernel");
    let service = service(root.path());
    let kernel = TerminalScope::Kernel;
    let pid_file = root.path().join("kernel-read.pid");
    let id = open_with_detached_job(
        &service,
        &kernel,
        root.path(),
        TerminalActor::User,
        &pid_file,
    )
    .await;
    service
        .read_output(kernel.clone(), &id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert!(
        ids(&service, &kernel).is_empty(),
        "read to the end, still listed"
    );
    let alive = job_running(&pid_file);
    kill_job(&pid_file);
    assert!(
        alive,
        "removing the read terminal ended the job it left running"
    );
}

#[tokio::test]
async fn an_agent_terminal_going_away_after_its_hour_leaves_the_job_it_left_running() {
    let root = TestWorktree::new("expiry-keeps-job-room");
    let service = service(root.path());
    let room = TerminalScope::room("room-1");
    let pid_file = root.path().join("room-expiry.pid");
    open_with_detached_job(&service, &room, root.path(), agent(), &pid_file).await;
    service.advance_clock_for_test(AGENT_ENDED_RETENTION + Duration::from_secs(1));
    assert!(
        ids(&service, &room).is_empty(),
        "the terminal is still listed"
    );
    let alive = job_running(&pid_file);
    // The Room's end still stops what its terminals left running.
    let stopped = job_ends_after(&pid_file, || {
        crate::runtime::process_resources::end_owner(Some("room-1"), None)
    })
    .await;
    kill_job(&pid_file);
    assert!(
        alive,
        "removing the ended terminal ended the job it left running"
    );
    assert!(
        stopped,
        "the job left behind cannot be stopped with its Room"
    );
}
