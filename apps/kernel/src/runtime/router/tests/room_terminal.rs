//! Room terminals through the real router: the owner-only gate and the
//! one-driver rule between the user and two Room agents.

use super::*;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult};
use crate::room_terminal::{RoomTerminalDriver, RoomTerminalInputResult, RoomTerminalSummary};
use base64::Engine as _;

fn run<F: std::future::Future<Output = ()> + 'static>(name: &str, test: fn() -> F) {
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(test());
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}

async fn terminal_request(
    router: &CommandRouter,
    user_id: &str,
    session_id: &str,
    acting_agent: Option<&str>,
    operation: RoomTerminalOperation,
) -> Result<RoomTerminalResult, DaemonError> {
    terminal_request_hosted(router, user_id, session_id, acting_agent, None, operation).await
}

async fn terminal_request_hosted(
    router: &CommandRouter,
    user_id: &str,
    session_id: &str,
    acting_agent: Option<&str>,
    host_agent: Option<&str>,
    operation: RoomTerminalOperation,
) -> Result<RoomTerminalResult, DaemonError> {
    let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
        session_id: session_id.to_string(),
        acting_agent: acting_agent.map(str::to_string),
        host_agent: host_agent.map(str::to_string),
        operation,
    });
    dispatch_terminal_request(
        router,
        remote_command_for_request(&request, Some(user_id)),
        request,
    )
    .await
}

async fn dispatch_terminal_request(
    router: &CommandRouter,
    command: KernelCommand,
    request: LocalDaemonRequest,
) -> Result<RoomTerminalResult, DaemonError> {
    match router.dispatch(command, request).await? {
        LocalDaemonResponse::RoomTerminal { result } => Ok(result),
        other => panic!("unexpected response: {other:?}"),
    }
}

async fn type_text(
    router: &CommandRouter,
    session_id: &str,
    terminal_id: &str,
    acting_agent: Option<&str>,
    text: &str,
) -> RoomTerminalInputResult {
    match terminal_request(
        router,
        DEFAULT_LOCAL_USER_ID,
        session_id,
        acting_agent,
        RoomTerminalOperation::Input {
            terminal_id: terminal_id.to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(text),
        },
    )
    .await
    .expect("input is admitted or refused")
    {
        RoomTerminalResult::Input { input } => input,
        other => panic!("unexpected result: {other:?}"),
    }
}

async fn list(router: &CommandRouter, session_id: &str) -> Vec<RoomTerminalSummary> {
    match terminal_request(
        router,
        DEFAULT_LOCAL_USER_ID,
        session_id,
        None,
        RoomTerminalOperation::List,
    )
    .await
    .expect("list")
    {
        RoomTerminalResult::Listed { terminals } => terminals,
        other => panic!("unexpected result: {other:?}"),
    }
}

async fn until_driver(router: &CommandRouter, session_id: &str, expected: RoomTerminalDriver) {
    for _ in 0..200 {
        if list(router, session_id).await[0].driver == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("driver never became {expected:?}");
}

#[test]
fn only_the_room_owner_reaches_terminals_and_drivers_do_not_collide() {
    run("room-terminal-router", || async {
        let worktree = crate::test_support::TestWorktree::new("room_terminal_router");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .expect("session");
        let session_id = session.id().to_string();
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_id,
                "invite-member".to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
                crate::session::CollaborationLevel::Private,
            )
            .expect("invite");
        app.sessions_mut()
            .join_session_invite(&session_id, invite.invite_id(), "member".to_string(), 1)
            .expect("member joins");
        let agent_a = spawn_test_agent(&mut app, &session_id, "agent-a", "dev-stub");
        let agent_b = spawn_test_agent(&mut app, &session_id, "agent-b", "dev-stub");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

        let opened = terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            None,
            RoomTerminalOperation::Open {
                cols: 100,
                rows: 30,
                title: None,
                command: None,
            },
        )
        .await
        .expect("the owner opens a terminal");
        let RoomTerminalResult::Opened { terminal } = opened else {
            panic!("unexpected result");
        };
        let id = terminal.terminal_id;
        assert_eq!(
            terminal.working_directory,
            worktree.path().display().to_string()
        );

        // TERM-NO-OTHER-MEMBERS: an invited member is refused by the protocol.
        for operation in [
            RoomTerminalOperation::List,
            RoomTerminalOperation::Snapshot {
                terminal_id: id.clone(),
            },
            RoomTerminalOperation::Input {
                terminal_id: id.clone(),
                data_base64: base64::engine::general_purpose::STANDARD.encode("id\r"),
            },
            RoomTerminalOperation::Close {
                terminal_id: id.clone(),
            },
        ] {
            let refused = terminal_request(&router, "member", &session_id, None, operation)
                .await
                .expect_err("a member is refused");
            assert!(
                matches!(refused, DaemonError::OwnershipAccessDenied { .. }),
                "{refused:?}"
            );
        }
        // A stranger never gets past Room membership.
        let stranger = terminal_request(
            &router,
            "stranger",
            &session_id,
            None,
            RoomTerminalOperation::List,
        )
        .await
        .expect_err("a stranger is refused");
        assert!(
            matches!(stranger, DaemonError::SessionAccessDenied { .. }),
            "{stranger:?}"
        );
        assert_eq!(list(&router, &session_id).await.len(), 1);

        // Only Room agents can be typed as.
        let unknown = terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            Some("no-such-agent"),
            RoomTerminalOperation::List,
        )
        .await
        .expect_err("unknown agent");
        assert!(unknown.to_string().contains("no agent"), "{unknown}");

        // TERM-DRIVER-LOCK: A starts a program; B is refused, the user is not.
        // A login shell that is still running its profile owns the foreground.
        until_driver(&router, &session_id, RoomTerminalDriver::Free).await;
        let a = Some(agent_a.id());
        let b = Some(agent_b.id());
        assert!(
            type_text(&router, &session_id, &id, a, "cat\r")
                .await
                .accepted
        );
        until_driver(
            &router,
            &session_id,
            RoomTerminalDriver::Busy {
                driven_by: crate::room_terminal::TerminalActor::Agent(agent_a.id().to_string()),
            },
        )
        .await;
        let refused = type_text(&router, &session_id, &id, b, "echo intruder\r").await;
        assert!(!refused.accepted);
        assert_eq!(
            refused.refusal,
            Some(format!("busy (driven by {})", agent_a.id()))
        );
        assert!(
            type_text(&router, &session_id, &id, None, "from-the-user\r")
                .await
                .accepted
        );
        assert!(
            type_text(&router, &session_id, &id, a, "\u{4}")
                .await
                .accepted
        );
        until_driver(&router, &session_id, RoomTerminalDriver::Free).await;
        assert!(
            type_text(&router, &session_id, &id, b, "echo b-now\r")
                .await
                .accepted
        );

        // The user used the tab: an agent cannot close it, the user can.
        let refused_close = terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            b,
            RoomTerminalOperation::Close {
                terminal_id: id.clone(),
            },
        )
        .await
        .expect_err("an agent cannot close a used tab");
        assert!(refused_close
            .to_string()
            .contains("only the user can close it"));
        terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            None,
            RoomTerminalOperation::Close {
                terminal_id: id.clone(),
            },
        )
        .await
        .expect("the user closes it");
        assert!(list(&router, &session_id).await.is_empty());
    });
}

#[test]
fn a_host_agent_must_belong_to_the_authorized_room() {
    run("room-terminal-host-agent", || async {
        let room_a = crate::test_support::TestWorktree::new("room_terminal_host_a");
        let room_b = crate::test_support::TestWorktree::new("room_terminal_host_b");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
        let session_a = app
            .sessions_mut()
            .create_session(room_a.session_request())
            .expect("session a")
            .id()
            .to_string();
        let session_b = app
            .sessions_mut()
            .create_session(room_b.session_request())
            .expect("session b")
            .id()
            .to_string();
        let agent_b = spawn_test_agent(&mut app, &session_b, "agent-b", "dev-stub");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

        let refused = terminal_request_hosted(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_a,
            None,
            Some(agent_b.id()),
            RoomTerminalOperation::List,
        )
        .await
        .expect_err("a host agent from another Room is refused");
        assert!(refused.to_string().contains("no agent"), "{refused}");
    });
}

#[test]
fn an_agent_made_by_another_member_is_not_the_owners_acting_or_host_agent() {
    run("room-terminal-member-agent", || async {
        let worktree = crate::test_support::TestWorktree::new("room_terminal_member_agent");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
        let session_id = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .expect("session")
            .id()
            .to_string();
        let member_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, "dev-stub").with_owner_user_id("room-member"),
            )
            .expect("member agent");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

        for (acting, host) in [
            (Some(member_agent.id()), None),
            (None, Some(member_agent.id())),
        ] {
            let refused = terminal_request_hosted(
                &router,
                DEFAULT_LOCAL_USER_ID,
                &session_id,
                acting,
                host,
                RoomTerminalOperation::List,
            )
            .await
            .expect_err("another member's agent is not the owner's");
            assert!(
                refused
                    .to_string()
                    .contains("does not act for the Room owner"),
                "{refused}"
            );
        }
    });
}

#[test]
fn the_enrolled_owner_of_a_legacy_local_room_keeps_its_terminals() {
    run("room-terminal-enrolled-owner", || async {
        let worktree = crate::test_support::TestWorktree::new("room_terminal_enrolled");
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "enrolled-user".to_string(),
            ..Default::default()
        });
        let app = DaemonApp::bootstrap(config).expect("daemon boots");
        let session_id = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .expect("session")
            .id()
            .to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

        let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
            session_id: session_id.clone(),
            acting_agent: None,
            host_agent: None,
            operation: RoomTerminalOperation::List,
        });
        let caller = KernelCaller {
            caller_id: "terminal".to_string(),
            caller_kind: KernelCallerKind::LocalClient,
            user_id: Some("enrolled-user".to_string()),
            connection_class: Some(crate::local::KernelConnectionClass::Terminal),
            ..Default::default()
        };
        let command = KernelCommand::from_local_request_with_caller(
            "enrolled-terminal",
            KernelCommandSource::LocalIpc,
            caller.clone(),
            None,
            None,
            &request,
        );
        let result = dispatch_terminal_request(&router, command, request)
            .await
            .expect("the enrolled owner of its local Room uses terminals");
        assert!(matches!(result, RoomTerminalResult::Listed { .. }));
        let request =
            LocalDaemonRequest::StopSessionProcesses(crate::local::StopSessionProcessesRequest {
                session_id: session_id.clone(),
                target: crate::runtime::process_resources::ProcessControlTarget::Process {
                    process_id: "ended-process".into(),
                },
            });
        let command = KernelCommand::from_local_request_with_caller(
            "enrolled-process-stop",
            KernelCommandSource::LocalIpc,
            caller,
            None,
            None,
            &request,
        );
        let error = router
            .dispatch(command, request)
            .await
            .expect_err("an ended target is refused after ownership authorization");
        assert!(
            matches!(
                error,
                DaemonError::LocalTransport {
                    operation: "stop session process",
                    ..
                }
            ),
            "the enrolled owner must reach target validation: {error}"
        );
    });
}

#[test]
fn room_exec_uses_exactly_the_room_terminal_owner_gate() {
    run("room-exec-router", || async {
        let worktree = crate::test_support::TestWorktree::new("room_exec_router");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let room = session.id().to_string();
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &room,
                "invite-member".to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
                crate::session::CollaborationLevel::Private,
            )
            .unwrap();
        app.sessions_mut()
            .join_session_invite(&room, invite.invite_id(), "member".to_string(), 1)
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let operation = RoomTerminalOperation::ExecOpen {
            command: vec!["/bin/cat".to_string()],
        };
        assert!(
            terminal_request(&router, "member", &room, None, operation.clone())
                .await
                .is_err(),
            "membership alone grants neither terminal nor exec access"
        );
        let result = terminal_request(&router, DEFAULT_LOCAL_USER_ID, &room, None, operation)
            .await
            .unwrap();
        let RoomTerminalResult::ExecOpened { exec_id } = result else {
            panic!("expected exec ID");
        };
        let listed = terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &room,
            None,
            RoomTerminalOperation::ExecList,
        )
        .await
        .unwrap();
        let RoomTerminalResult::ExecListed { execs } = listed else {
            panic!("expected exec list");
        };
        assert_eq!(execs.len(), 1);
        assert_eq!(execs[0].exec_id, exec_id);
        for operation in [
            RoomTerminalOperation::ExecList,
            RoomTerminalOperation::ExecWrite {
                exec_id: exec_id.clone(),
                offset: 0,
                data_base64: "YQ==".to_string(),
            },
            RoomTerminalOperation::ExecStdinClose {
                exec_id: exec_id.clone(),
                offset: 0,
            },
            RoomTerminalOperation::ExecRead {
                exec_id: exec_id.clone(),
                after_cursor: 0,
                wait_ms: 0,
            },
            RoomTerminalOperation::ExecCancel {
                exec_id: exec_id.clone(),
            },
            RoomTerminalOperation::ExecClose {
                exec_id: exec_id.clone(),
            },
        ] {
            assert!(terminal_request(&router, "member", &room, None, operation)
                .await
                .is_err());
        }
        terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &room,
            None,
            RoomTerminalOperation::ExecClose { exec_id },
        )
        .await
        .unwrap();
    });
}

// A one-shot `GetSessionState` carries the process rows the live feed shows.
#[test]
fn a_one_shot_session_state_lists_the_rooms_processes() {
    run("room-state-processes", || async {
        let worktree = crate::test_support::TestWorktree::new("room_state_processes");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
        let session_id = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .expect("session")
            .id()
            .to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let opened = terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            None,
            RoomTerminalOperation::Open {
                cols: 80,
                rows: 24,
                title: None,
                command: Some(vec!["/bin/sleep".into(), "8897".into()]),
            },
        )
        .await
        .expect("the owner opens a terminal");
        let RoomTerminalResult::Opened { terminal } = opened else {
            panic!("unexpected result");
        };
        let rows = || async {
            let request =
                LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
                    session_id: session_id.clone(),
                });
            let response = router
                .dispatch(
                    remote_command_for_request(&request, Some(DEFAULT_LOCAL_USER_ID)),
                    request,
                )
                .await
                .expect("one-shot session state");
            serde_json::to_value(response).unwrap()["SessionState"]["processes"].clone()
        };
        let mut found = serde_json::Value::Null;
        for _ in 0..100 {
            found = rows().await;
            if found.as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["command"].as_str().is_some_and(|c| c.contains("sleep")))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            None,
            RoomTerminalOperation::Close {
                terminal_id: terminal.terminal_id.clone(),
            },
        )
        .await
        .expect("the owner closes it");
        let row = found
            .as_array()
            .and_then(|rows| rows.first())
            .unwrap_or_else(|| panic!("the one-shot state lists no process: {found}"));
        assert_eq!(row["owner"]["kind"], "terminal");
        assert_eq!(row["owner"]["id"], terminal.terminal_id.as_str());
    });
}

/// How the agent whose Room terminal detached a job goes away.
#[derive(Clone, Copy, Debug)]
enum AgentEnd {
    Archive,
    Delete,
}

/// MP-08 / MP-09 / MP-10 / MP-11: a job an agent's Room terminal detached
/// before its command returned ends with the agent's archive or deletion,
/// whether the ended terminal is still listed or the agent already closed it.
async fn agent_end_ends_a_detached_terminal_job(end: AgentEnd, close_first: bool, name: &str) {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
    let session = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "terminal-agent", "dev-stub");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let pid_file = worktree.path().join("detached.pid");
    let script = format!(
        "{} </dev/null >/dev/null 2>&1 &",
        crate::test_support::detached_job(&format!("'{}'", pid_file.display()), 8897)
    );
    let RoomTerminalResult::Opened { terminal } = terminal_request(
        &router,
        DEFAULT_LOCAL_USER_ID,
        &session_id,
        Some(agent.id()),
        RoomTerminalOperation::Open {
            cols: 100,
            rows: 30,
            title: None,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        },
    )
    .await
    .expect("the agent opens a terminal") else {
        panic!("unexpected result");
    };
    let read_job = || {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
    };
    let mut ended = false;
    for _ in 0..200 {
        ended = list(&router, &session_id).await.iter().any(|row| {
            matches!(
                row.status,
                crate::room_terminal::RoomTerminalStatus::Ended { .. }
            )
        });
        if ended && read_job().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let job = read_job();
    let cleanup = || {
        if let Some(pid) = job {
            // SAFETY: the positive PID of the job this test started; never a group.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    };
    if !ended || job.is_none() {
        cleanup();
        panic!("the terminal did not end with a detached job (ended: {ended})");
    }
    // Past the grace in which a job still detaching may leave the session.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let job = job.unwrap();
    if !crate::test_support::process_alive(job) {
        panic!("the detached job did not outlive its command");
    }
    if close_first {
        terminal_request(
            &router,
            DEFAULT_LOCAL_USER_ID,
            &session_id,
            Some(agent.id()),
            RoomTerminalOperation::Close {
                terminal_id: terminal.terminal_id.clone(),
            },
        )
        .await
        .expect("the agent closes its ended terminal");
    }
    let (label, request) = match end {
        AgentEnd::Archive => (
            "agent-archive",
            LocalDaemonRequest::ArchiveAgent(crate::local::ArchiveAgentRequest {
                session_id: session_id.clone(),
                agent_id: agent.id().to_string(),
                archived: true,
            }),
        ),
        AgentEnd::Delete => (
            "agent-destroy",
            LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
                session_id: session_id.clone(),
                agent_id: agent.id().to_string(),
            }),
        ),
    };
    let command = KernelCommand::from_local_request(label, None, None, &request);
    let ended_agent = router.dispatch(command, request).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while crate::test_support::process_alive(job) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let survived = crate::test_support::process_alive(job);
    cleanup();
    ended_agent.expect("the agent goes away");
    assert!(
        !survived,
        "the agent's detached terminal job kept running after {end:?} (terminal closed: {close_first})"
    );
}

#[test]
fn archiving_an_agent_ends_the_detached_job_of_its_ended_terminal() {
    run("archive-terminal-job-listed", || {
        agent_end_ends_a_detached_terminal_job(
            AgentEnd::Archive,
            false,
            "archive_terminal_job_listed",
        )
    });
}

#[test]
fn archiving_an_agent_ends_the_detached_job_of_its_closed_terminal() {
    run("archive-terminal-job-closed", || {
        agent_end_ends_a_detached_terminal_job(
            AgentEnd::Archive,
            true,
            "archive_terminal_job_closed",
        )
    });
}

#[test]
fn deleting_an_agent_ends_the_detached_job_of_its_ended_terminal() {
    run("delete-terminal-job-listed", || {
        agent_end_ends_a_detached_terminal_job(
            AgentEnd::Delete,
            false,
            "delete_terminal_job_listed",
        )
    });
}

#[test]
fn deleting_an_agent_ends_the_detached_job_of_its_closed_terminal() {
    run("delete-terminal-job-closed", || {
        agent_end_ends_a_detached_terminal_job(AgentEnd::Delete, true, "delete_terminal_job_closed")
    });
}
