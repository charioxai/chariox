//! Real PTYs and the user's real shell.

use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use super::*;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-room-terminal-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agent(id: &str) -> TerminalActor {
    TerminalActor::Agent(id.to_string())
}

fn service(root: &Path) -> RoomTerminalService {
    RoomTerminalService::new(root.join("terminals"))
}

fn open(service: &RoomTerminalService, directory: &Path) -> RoomTerminalSummary {
    service
        .open(OpenRoomTerminal {
            session_id: "room-1".to_string(),
            working_directory: directory.to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
        })
        .expect("terminal opens")
}

async fn until<F: Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn screen_has(service: &RoomTerminalService, terminal_id: &str, text: &str) -> bool {
    service.pump_once();
    let snapshot = service.snapshot("room-1", terminal_id).unwrap();
    snapshot.screen_text.contains(text) || snapshot.scrollback_text.contains(text)
}

async fn type_as(
    service: &RoomTerminalService,
    terminal_id: &str,
    actor: &TerminalActor,
    input: &str,
) -> RoomTerminalInputResult {
    service
        .input("room-1", terminal_id, actor, input.as_bytes())
        .await
        .expect("input is admitted or refused")
}

#[tokio::test]
async fn opens_the_login_shell_in_the_project_folder() {
    let root = TempDir::new();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let service = service(root.path());
    let terminal = open(&service, &project);
    let id = &terminal.terminal_id;
    assert_eq!(terminal.working_directory, project.display().to_string());

    type_as(&service, id, &TerminalActor::User, "pwd; shopt -q login_shell && echo LOGIN-$((6*7))\r").await;
    until("pwd and login shell", || async {
        screen_has(&service, id, "LOGIN-42").await && screen_has(&service, id, "project").await
    })
    .await;
    service.end_session("room-1");
}

#[tokio::test]
async fn one_driver_rule_protects_a_foreground_program() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;

    assert!(type_as(&service, &id, &agent("a"), "cat\r").await.accepted);
    until("cat to own the foreground", || async {
        service.pump_once();
        service.list("room-1")[0].driver
            == RoomTerminalDriver::Busy {
                driven_by: agent("a"),
            }
    })
    .await;

    let refused = type_as(&service, &id, &agent("b"), "echo intruder\r").await;
    assert!(!refused.accepted);
    assert_eq!(refused.refusal.as_deref(), Some("busy (driven by a)"));
    assert!(type_as(&service, &id, &TerminalActor::User, "from-the-user\r").await.accepted);
    until("the user's text to echo", || async {
        screen_has(&service, &id, "from-the-user").await
    })
    .await;
    assert!(!service.snapshot("room-1", &id).unwrap().screen_text.contains("intruder"));

    assert!(type_as(&service, &id, &agent("a"), "\u{4}").await.accepted);
    until("cat to exit", || async {
        service.pump_once();
        service.list("room-1")[0].driver == RoomTerminalDriver::Free
    })
    .await;
    assert!(type_as(&service, &id, &agent("b"), "echo b-types-now\r").await.accepted);
    until("b's command", || async { screen_has(&service, &id, "b-types-now").await }).await;
    service.end_session("room-1");
}

#[tokio::test]
async fn a_program_the_user_started_refuses_agents() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    type_as(&service, &id, &TerminalActor::User, "cat\r").await;
    until("cat to run", || async {
        service.pump_once();
        service.list("room-1")[0].driver
            == RoomTerminalDriver::Busy {
                driven_by: TerminalActor::User,
            }
    })
    .await;
    let refused = type_as(&service, &id, &agent("a"), "x").await;
    assert_eq!(refused.refusal.as_deref(), Some("busy (driven by user)"));
    service.end_session("room-1");
}

#[tokio::test]
async fn output_continues_from_an_offset_without_a_gap() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let first = service
        .read_output("room-1", &id, 0, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(!first.gap && first.next_offset > 0);

    type_as(&service, &id, &TerminalActor::User, "echo MARK-$((2+2))\r").await;
    let mut offset = first.next_offset;
    let mut seen = Vec::new();
    while !String::from_utf8_lossy(&seen).contains("MARK-4\r\n") {
        let page = service
            .read_output("room-1", &id, offset, Duration::from_secs(10))
            .await
            .unwrap();
        assert!(!page.gap);
        seen.extend(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, page.data_base64)
                .unwrap(),
        );
        offset = page.next_offset;
    }
    let lost = service
        .read_output("room-1", &id, offset + 10_000, Duration::ZERO)
        .await
        .unwrap();
    assert!(lost.gap);
    service.end_session("room-1");
}

#[tokio::test]
async fn shell_exit_ends_the_terminal_and_keeps_its_screen() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    type_as(&service, &id, &TerminalActor::User, "echo kept-line; exit 3\r").await;
    until("the shell to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    let summary = &service.list("room-1")[0];
    assert_eq!(
        summary.status,
        RoomTerminalStatus::Ended {
            outcome: RoomTerminalOutcome::ShellExited {
                exit_code: Some(3),
                signal: None
            },
            description: "ended: shell exited with code 3".to_string()
        }
    );
    assert!(service.snapshot("room-1", &id).unwrap().screen_text.contains("kept-line"));
    assert!(service
        .input("room-1", &id, &TerminalActor::User, b"x")
        .await
        .is_err());
}

#[tokio::test]
async fn a_restart_ends_terminals_and_keeps_scrollback() {
    let root = TempDir::new();
    let first = service(root.path());
    let id = open(&first, root.path()).terminal_id;
    type_as(&first, &id, &TerminalActor::User, "seq 1 80; echo before-restart\r").await;
    until("output", || async { screen_has(&first, &id, "before-restart").await }).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    first.pump_once();

    let second = service(root.path());
    let summary = &second.list("room-1")[0];
    assert_eq!(summary.terminal_id, id);
    assert_eq!(
        summary.status,
        RoomTerminalStatus::Ended {
            outcome: RoomTerminalOutcome::KernelRestarted,
            description: "ended: kernel restarted".to_string()
        }
    );
    let snapshot = second.snapshot("room-1", &id).unwrap();
    assert!(snapshot.screen_text.contains("before-restart"));
    assert!(snapshot.scrollback_text.contains("\n2\n"), "{}", snapshot.scrollback_text);
    assert!(second.input("room-1", &id, &TerminalActor::User, b"x").await.is_err());
    first.end_session("room-1");
}

#[tokio::test]
async fn closing_and_ending_a_room_stop_the_shell_processes() {
    let root = TempDir::new();
    let service = service(root.path());
    let a = open(&service, root.path()).terminal_id;
    let b = open(&service, root.path()).terminal_id;
    assert_eq!(
        service.list("room-1").iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        ["Terminal 1", "Terminal 2"]
    );
    service.close("room-1", &a).unwrap();
    assert_eq!(service.list("room-1").len(), 1);
    assert!(service.snapshot("room-1", &a).is_err());
    service.end_session("room-1");
    assert!(service.list("room-1").is_empty());
    assert!(service.snapshot("room-1", &b).is_err());
    assert!(!root.path().join("terminals").join(format!("{b}.json")).exists());
}

#[tokio::test]
async fn watches_fire_for_text_finish_and_waiting_for_input() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let mut events = service.subscribe();

    service
        .watch("room-1", &id, "agent-a", RoomTerminalWatch::OutputContains("NEEDLE-7".into()))
        .unwrap();
    service
        .watch("room-1", &id, "agent-a", RoomTerminalWatch::WaitingForInput)
        .unwrap();
    type_as(&service, &id, &agent("agent-a"), "echo NEEDLE-$((3+4)); cat\r").await;
    let mut kinds = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while kinds.len() < 2 {
        service.pump_once();
        if let Ok(event) = events.try_recv() {
            kinds.push(event.kind);
        } else {
            assert!(Instant::now() < deadline, "events so far: {kinds:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    assert!(matches!(&kinds[0], RoomTerminalEventKind::OutputMatched { text } if text == "NEEDLE-7"));
    assert_eq!(kinds[1], RoomTerminalEventKind::WaitingForInput);

    service
        .watch("room-1", &id, "agent-a", RoomTerminalWatch::Finished)
        .unwrap();
    type_as(&service, &id, &agent("agent-a"), "\u{4}exit 5\r").await;
    let event = loop {
        service.pump_once();
        if let Ok(event) = events.try_recv() {
            break event;
        }
        assert!(Instant::now() < deadline, "no finish event");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        event.kind,
        RoomTerminalEventKind::Finished {
            exit_code: Some(5),
            signal: None
        }
    );
    assert!(event.screen_text.contains("NEEDLE-7"));
}
