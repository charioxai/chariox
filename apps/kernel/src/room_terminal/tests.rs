//! Real PTYs and the user's real shell.

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::events::{FinishBinding, OutputMatcher};
use super::service::enters;
use super::*;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        // Tests start in the same millisecond; the counter keeps their
        // directories apart so one test's cleanup cannot remove another's.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chariox-room-terminal-{}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

#[derive(Default)]
struct FakeEvents(Mutex<Vec<RoomTerminalEvent>>);

impl CompletionEvents for FakeEvents {
    fn deliver(&self, event: RoomTerminalEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl FakeEvents {
    fn fired(&self, wake_id: &str) -> Vec<RoomTerminalEvent> {
        let all = self.0.lock().unwrap();
        all.iter()
            .filter(|event| event.wake_id == wake_id)
            .cloned()
            .collect()
    }
}

fn service(root: &Path) -> RoomTerminalService {
    service_with_events(root, Arc::new(FakeEvents::default()))
}

fn service_with_events(root: &Path, events: Arc<FakeEvents>) -> RoomTerminalService {
    let service = RoomTerminalService::new(root.join("terminals")).unwrap();
    let weak = Arc::downgrade(&events);
    service.bind_events(weak);
    service
}

fn owner(wake_id: &str, agent_id: &str) -> WatchOwner {
    WatchOwner {
        wake_id: wake_id.to_string(),
        agent_id: agent_id.to_string(),
    }
}

fn open(service: &RoomTerminalService, directory: &Path) -> RoomTerminalSummary {
    service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: directory.to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: None,
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

    type_as(
        &service,
        id,
        &TerminalActor::User,
        "pwd; shopt -q login_shell && echo LOGIN-$((6*7))\r",
    )
    .await;
    until("pwd and login shell", || async {
        screen_has(&service, id, "LOGIN-42").await && screen_has(&service, id, "project").await
    })
    .await;
    service.end_session("room-1");
}

/// Restores `HOME` when the test ends, whichever way it ends.
struct RestoreHome(Option<std::ffi::OsString>);

impl Drop for RestoreHome {
    fn drop(&mut self) {
        match self.0.take() {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}

#[test]
fn an_agent_may_type_while_the_login_files_still_run() {
    let _environment = crate::env_lock::lock();
    let root = TempDir::new();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let running = home.join("profile-running");
    std::fs::write(
        home.join(".bash_profile"),
        format!("sleep 0.3\ntouch '{}'\nsleep 2\n", running.display()),
    )
    .unwrap();
    let _home = RestoreHome(std::env::var_os("HOME"));
    std::env::set_var("HOME", &home);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let service = service(root.path());
            let id = open(&service, root.path()).terminal_id;
            until("a login file to hold the foreground", || async {
                running.exists()
            })
            .await;
            // The `touch` has returned and `sleep 2` is starting: a program the
            // login files started holds the foreground, and nobody ran it.
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(service.list("room-1")[0].driver, RoomTerminalDriver::Free);
            let sent = type_as(&service, &id, &agent("a"), "echo STARTED-$((6*7))\r").await;
            assert!(sent.accepted, "input refused: {:?}", sent.refusal);
            until("the line to run once the prompt appears", || async {
                screen_has(&service, &id, "STARTED-42").await
            })
            .await;
            until("the terminal to be free again", || async {
                service.pump_once();
                service.list("room-1")[0].driver == RoomTerminalDriver::Free
            })
            .await;
            service.end_session("room-1");
        });
}

/// zsh installs its finish hook while the first prompt's hooks run, so the
/// first mark it reports is the start of the first command.
#[test]
fn the_first_command_ends_the_startup_window_without_a_prompt_mark() {
    use super::store::TerminalRecord;
    use super::terminal::Terminal;
    let record = TerminalRecord {
        terminal_id: "t".to_string(),
        session_id: "room-1".to_string(),
        title: "zsh".to_string(),
        program: "/bin/zsh".to_string(),
        working_directory: "/".to_string(),
        cols: 100,
        rows: 30,
        opened_by: TerminalActor::User,
        command: None,
        user_interacted: false,
        created_at_ms: 0,
        ended: None,
        scrollback_text: String::new(),
        screen_text: String::new(),
    };
    let mut terminal = Terminal::running(record, "run".to_string(), None, true);
    let now = Instant::now();
    assert!(terminal.starting(now));
    terminal.apply_output(b"\x1b]133;C\x07", now);
    assert!(
        !terminal.starting(now),
        "a started command is not a login file"
    );
    assert!(terminal.is_busy(&crate::pty::PtyManager::new()));
}

#[tokio::test]
async fn a_program_started_right_after_login_refuses_other_agents() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let sent = type_as(&service, &id, &agent("a"), "sleep 3\r").await;
    assert!(sent.accepted, "input refused: {:?}", sent.refusal);
    until("the first program to hold the terminal", || async {
        service.pump_once();
        service.list("room-1")[0].driver
            == RoomTerminalDriver::Busy {
                driven_by: agent("a"),
            }
    })
    .await;
    let other = type_as(&service, &id, &agent("b"), "echo INTRUDER\r").await;
    assert!(
        !other.accepted,
        "a running program took input from another agent"
    );
    service.end_session("room-1");
}

#[test]
fn a_queued_line_is_not_finished_by_the_login_files_helpers() {
    let _environment = crate::env_lock::lock();
    let root = TempDir::new();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let running = home.join("profile-running");
    std::fs::write(
        home.join(".bash_profile"),
        format!("sleep 0.3\ntouch '{}'\nsleep 2\n", running.display()),
    )
    .unwrap();
    let _home = RestoreHome(std::env::var_os("HOME"));
    std::env::set_var("HOME", &home);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let service = service(root.path());
            let id = open(&service, root.path()).terminal_id;
            until("a login file to hold the foreground", || async {
                running.exists()
            })
            .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            let sent = type_as(&service, &id, &agent("a"), "sleep 5; echo DONE-$((6*7))\r").await;
            assert!(sent.accepted, "input refused: {:?}", sent.refusal);
            let offset = service.snapshot("room-1", &id).unwrap().output_offset;
            let pump = service.clone();
            let pumping = tokio::spawn(async move {
                loop {
                    pump.pump_once();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            });
            let waited = service
                .wait(
                    "room-1",
                    &id,
                    "a",
                    vec![RoomTerminalWatch::Finished],
                    Some(offset),
                    Duration::from_secs(25),
                )
                .await
                .unwrap();
            pumping.abort();
            assert!(matches!(
                waited.fired,
                Some(RoomTerminalEventKind::Finished { .. })
            ));
            assert!(
                waited.screen_text.contains("DONE-42"),
                "the wait ended before the queued command: {}",
                waited.screen_text
            );
            service.end_session("room-1");
        });
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
    assert!(
        type_as(&service, &id, &TerminalActor::User, "from-the-user\r")
            .await
            .accepted
    );
    until("the user's text to echo", || async {
        screen_has(&service, &id, "from-the-user").await
    })
    .await;
    assert!(!service
        .snapshot("room-1", &id)
        .unwrap()
        .screen_text
        .contains("intruder"));

    assert!(type_as(&service, &id, &agent("a"), "\u{4}").await.accepted);
    until("cat to exit", || async {
        service.pump_once();
        service.list("room-1")[0].driver == RoomTerminalDriver::Free
    })
    .await;
    assert!(
        type_as(&service, &id, &agent("b"), "echo b-types-now\r")
            .await
            .accepted
    );
    until("b's command", || async {
        screen_has(&service, &id, "b-types-now").await
    })
    .await;
    service.end_session("room-1");
}

#[test]
fn every_enter_at_a_prompt_is_a_submission() {
    assert_eq!(enters(b"cat"), 0, "typing without a line break");
    assert_eq!(enters(b"\r"), 1, "Enter on text typed earlier, or empty");
    assert_eq!(enters(b"\x1b[A\r"), 1, "a recalled line");
    assert_eq!(enters(b"true\r\n"), 1, "a CRLF is one line break");
    assert_eq!(enters(b"a\rb\n\nc"), 3, "the last is still being typed");
    assert_eq!(
        enters(b"\x1b[200~a\nb\r\x1b[201~\r"),
        1,
        "a paste in brackets is one line"
    );
}

/// Types each input in turn (on the login shell, or on `shell` when given),
/// then returns, per pid file the inputs write, whether the kernel may hold
/// that command for room.
async fn who_owns(
    shell: Option<&str>,
    inputs: &[(&TerminalActor, String)],
    pid_files: &[&str],
) -> Vec<bool> {
    let root = TempDir::new();
    let service = service(root.path());
    if let Some(shell) = shell {
        service.use_shell_for_test(shell);
    }
    let id = open(&service, root.path()).terminal_id;
    let run_id = service
        .terminal_for_test(&id)
        .lock_open()
        .unwrap()
        .terminal
        .run_id
        .clone()
        .unwrap();
    // The pid files are named in the inputs through `{dir}`.
    // A shell without marks can run foreground programs from its login files.
    // Prove it has read a command before testing who owns subsequent input.
    assert!(
        type_as(
            &service,
            &id,
            &TerminalActor::User,
            "printf 'READY-%s\\n' 42\r"
        )
        .await
        .accepted
    );
    until("the shell to read input", || {
        screen_has(&service, &id, "READY-42")
    })
    .await;
    // The output appears before the shell reports the command finished; an
    // input in between would find the terminal still driven by the person.
    until("the shell to be back at its prompt", || async {
        service.pump_once();
        service.list("room-1")[0].driver == RoomTerminalDriver::Free
    })
    .await;
    let sampling = crate::runtime::process_resources::pause_sampling_for_key(&run_id);
    let dir = root.path().display().to_string();
    for (actor, input) in inputs {
        let mut input = input.replace("{dir}", &dir);
        // `{after NAME}` holds an input back until the command before it has
        // launched what writes the pid file NAME.
        if let Some(rest) = input.strip_prefix("{after ") {
            let (name, text) = rest.split_once('}').expect("a closing brace");
            let file = root.path().join(name);
            until("the earlier command to be launched", || async {
                file.exists()
            })
            .await;
            input = text.to_string();
        }
        assert!(
            type_as(&service, &id, actor, &input).await.accepted,
            "an input was refused, so the race did not happen"
        );
    }
    let mut pids = Vec::new();
    for name in pid_files {
        let file = root.path().join(name);
        let pid = || {
            std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok())
        };
        until("the command to be launched", || async { pid().is_some() }).await;
        let pid = pid().unwrap();
        assert!(pid > 1 && i32::try_from(pid).is_ok(), "a child PID");
        assert_eq!(
            crate::runtime::process_resources::command_holdable_for_key(&run_id, pid),
            None,
            "the sampler must not see the job before the next submission"
        );
        pids.push(pid);
    }
    drop(sampling);
    let mut owners = Vec::new();
    for &pid in &pids {
        let deadline = Instant::now() + Duration::from_secs(15);
        let owner = loop {
            if let Some(holdable) =
                crate::runtime::process_resources::command_holdable_for_key(&run_id, pid)
            {
                break Some(holdable);
            }
            if Instant::now() > deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        owners.push(owner);
    }
    for pid in pids {
        // SAFETY: the positive PID of a command this test started; never a group.
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    service.end_session("room-1");
    owners
        .into_iter()
        .map(|owner| owner.expect("the sampler never saw the command"))
        .collect()
}

/// The two shells that report their job table: the login shell (bash) and zsh.
fn integrated_shells(root: &Path) -> [Option<String>; 2] {
    let zsh = std::env::var("CHARIOX_TEST_ZSH").unwrap_or_else(|_| "/usr/bin/zsh".into());
    assert!(
        Path::new(&zsh).exists(),
        "zsh is not installed at {zsh}: install it or set CHARIOX_TEST_ZSH"
    );
    // Keep the real integration .zshenv, but give it only our startup files.
    // Global rc files can run compinit and consume typed input with a prompt.
    let startup = root.join("zsh-startup");
    std::fs::create_dir_all(&startup).unwrap();
    std::fs::write(startup.join(".zshenv"), "unsetopt GLOBAL_RCS\n").unwrap();
    let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    let wrapper = root.join("zsh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport CHARIOX_ORIG_ZDOTDIR={}\nexec {} \"$@\"\n",
            quote(startup.to_str().unwrap()),
            quote(&zsh),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    [None, Some(wrapper.to_str().unwrap().to_string())]
}

/// A background job's line, then a second line accepted before the shell
/// launches the first.
async fn who_owns_the_job_two_lines_launch(first: &TerminalActor, second: &TerminalActor) -> bool {
    who_owns(
        None,
        &[
            (first, "sleep 120 & echo $! > '{dir}/job'\r".to_string()),
            (second, "true\r".to_string()),
        ],
        &["job"],
    )
    .await[0]
}

#[tokio::test]
async fn a_hot_reload_hands_over_who_launched_the_terminals_commands() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let run_id = service
        .terminal_for_test(&id)
        .lock_open()
        .unwrap()
        .terminal
        .run_id
        .clone()
        .unwrap();
    let job = root.path().join("job");
    type_as(
        &service,
        &id,
        &TerminalActor::User,
        &format!("sleep 120 & echo $! > '{}'\r", job.display()),
    )
    .await;
    until("the job to be launched", || async { job.exists() }).await;
    let pid: u32 = std::fs::read_to_string(&job)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    until("the sampler to decide the job", || async {
        crate::runtime::process_resources::command_holdable_for_key(&run_id, pid).is_some()
    })
    .await;
    let handovers = service.hand_over().expect("the terminal is handed over");
    service.release_handover();
    // SAFETY: the positive PID of a job this test started; never a group.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    service.end_session("room-1");
    let submissions = serde_json::to_value(&handovers[0].0.pty.submissions).unwrap();
    assert_eq!(submissions["default"], false, "{submissions}");
    assert_eq!(submissions["unreported"], false, "{submissions}");
    assert_eq!(submissions["jobs"][pid.to_string()], false, "{submissions}");
}

/// The PTY a master descriptor refers to, or `None` when the descriptor is not an open PTY master.
fn pty_number(fd: i32) -> Option<libc::c_uint> {
    let mut number: libc::c_uint = 0;
    (unsafe { libc::ioctl(fd, libc::TIOCGPTN, &mut number) } == 0).then_some(number)
}

#[tokio::test]
async fn a_hot_reload_owns_every_descriptor_it_names_while_the_kernel_stops() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let prepared = crate::hot_reload::PreparedForTests::with_terminals(&service);
    let named = prepared.inherited_fds();
    let ptys: Vec<_> = named.iter().map(|fd| pty_number(*fd)).collect();
    assert!(
        ptys.iter().all(Option::is_some),
        "{named:?} are not all PTY masters"
    );
    let mut distinct = named.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        named.len(),
        "{named:?} names a descriptor twice"
    );
    let shell = unsafe { libc::tcgetpgrp(named[0]) };
    // The kernel stops before `exec`: the terminal's owner drops its PTY.
    let owner = service.terminal_for_test(&id);
    drop(std::mem::take(&mut owner.lock_open().unwrap().pty));
    let after: Vec<_> = named.iter().map(|fd| pty_number(*fd)).collect();
    drop(prepared);
    if shell > 1 {
        // SAFETY: the positive PID of the shell this test started; never a group.
        unsafe { libc::kill(shell, libc::SIGKILL) };
        unsafe { libc::waitpid(shell, std::ptr::null_mut(), 0) };
    }
    assert_eq!(
        after, ptys,
        "the swap names {named:?}, which closed or changed while the kernel stopped"
    );
}

/// The stopped kernel's state as `run_on_listener` leaves it: it owns the terminals.
struct StoppedKernel {
    _terminals: RoomTerminalService,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for StoppedKernel {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_hot_reload_keeps_the_stopped_kernel_and_its_terminals_until_exec() {
    use std::sync::atomic::Ordering::SeqCst;
    let root = TempDir::new();
    let service = service(root.path());
    open(&service, root.path());
    let prepared = crate::hot_reload::PreparedForTests::with_terminals(&service);
    let shell = unsafe { libc::tcgetpgrp(prepared.inherited_fds()[0]) };
    assert!(shell > 1, "the terminal has no shell");
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // The kernel's run returns before main's `finish` replaces the program.
    crate::hot_reload::keep_until_exec(Box::new(StoppedKernel {
        _terminals: service,
        dropped: Arc::clone(&dropped),
    }));
    let kept = !dropped.load(SeqCst);
    let running = unsafe { libc::waitpid(shell, std::ptr::null_mut(), libc::WNOHANG) } == 0;
    drop(prepared);
    let released = dropped.load(SeqCst);
    // SAFETY: the positive PID of the shell this test started; never a group.
    unsafe { libc::kill(shell, libc::SIGKILL) };
    unsafe { libc::waitpid(shell, std::ptr::null_mut(), 0) };
    assert!(kept, "the stopped kernel was dropped before exec");
    assert!(
        running,
        "the handed-over terminal's shell ended before exec"
    );
    assert!(released, "the stopped kernel outlived the swap");
}

/// A command that records its pid, then sleeps as that same process.
const SLEEPER: &str = "sh -c 'echo $$ > {dir}/sleeper; exec sleep 120'";

#[tokio::test]
async fn an_agents_background_job_stays_the_agents_when_a_person_submits_before_the_shell_launches_it(
) {
    assert!(
        who_owns_the_job_two_lines_launch(&agent("a"), &TerminalActor::User).await,
        "the person's later line took the agent's job"
    );
}

#[tokio::test]
async fn a_persons_background_job_stays_the_persons_when_an_agent_submits_before_the_shell_launches_it(
) {
    assert!(
        !who_owns_the_job_two_lines_launch(&TerminalActor::User, &agent("a")).await,
        "the agent's later line took the person's job"
    );
}

#[tokio::test]
async fn a_command_belongs_to_whoever_pressed_enter_when_the_text_came_in_separately() {
    let a = agent("a");
    let line = SLEEPER.to_string();
    assert_eq!(
        who_owns(
            None,
            &[(&a, line.clone()), (&a, "\r".to_string())],
            &["sleeper"]
        )
        .await,
        [true],
        "an agent's Enter in a separate input did not submit the agent's command"
    );
    assert_eq!(
        who_owns(
            None,
            &[(&a, line), (&TerminalActor::User, "\r".to_string())],
            &["sleeper"]
        )
        .await,
        [false],
        "the person's Enter did not take the command it submitted"
    );
}

#[tokio::test]
async fn every_job_one_line_starts_belongs_to_that_lines_submitter() {
    let a = agent("a");
    let line = "sleep 120 & echo $! > '{dir}/one'; sleep 121 & echo $! > '{dir}/two'\r";
    let shells = TempDir::new();
    for shell in integrated_shells(shells.path()) {
        assert_eq!(
            who_owns(
                shell.as_deref(),
                &[
                    (&a, line.to_string()),
                    (&TerminalActor::User, "true\r".to_string())
                ],
                &["one", "two"]
            )
            .await,
            [true, true],
            "{shell:?}: the person's later line took a job the agent's line started"
        );
    }
}

#[tokio::test]
async fn a_pipeline_in_the_background_belongs_to_its_line_in_every_member() {
    let a = agent("a");
    let line = "sh -c 'echo $$ > {dir}/head; exec sleep 120' | sh -c 'echo $$ > {dir}/tail; exec sleep 121' &\r";
    let shells = TempDir::new();
    for shell in integrated_shells(shells.path()) {
        assert_eq!(
            who_owns(
                shell.as_deref(),
                &[
                    (&a, line.to_string()),
                    (&TerminalActor::User, "true\r".to_string())
                ],
                &["head", "tail"]
            )
            .await,
            [true, true],
            "{shell:?}: a member of the agent's background pipeline took the person's line"
        );
    }
}

#[tokio::test]
async fn zsh_reports_its_jobs_without_a_word_at_the_prompt() {
    let root = TempDir::new();
    let service = service(root.path());
    service.use_shell_for_test(integrated_shells(root.path())[1].as_deref().unwrap());
    let id = open(&service, root.path()).terminal_id;
    type_as(
        &service,
        &id,
        &agent("a"),
        "sleep 5 & sleep 6 & sleep 7 | cat &\r",
    )
    .await;
    type_as(&service, &id, &TerminalActor::User, "echo DONE-$((2+2))\r").await;
    until("the prompt to return", || {
        screen_has(&service, &id, "DONE-4")
    })
    .await;
    let screen = service.snapshot("room-1", &id).unwrap().screen_text;
    for word in [
        "zsh:",
        "not found",
        "bad ",
        "no such",
        "jobstates",
        "parameter",
    ] {
        assert!(
            !screen.to_lowercase().contains(word),
            "the prompt shows `{word}`: {screen}"
        );
    }
    service.end_session("room-1");
}

#[tokio::test]
async fn a_running_command_is_decided_while_a_later_line_waits_behind_it() {
    let a = agent("a");
    let started = Instant::now();
    assert_eq!(
        who_owns(None, &[(&a, format!("{SLEEPER}\rtrue\r"))], &["sleeper"]).await,
        [true]
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the running command waited for the line queued behind it"
    );
}

#[tokio::test]
async fn an_agents_command_stays_the_agents_in_a_shell_that_reports_no_marks() {
    // Marks exist for bash and zsh; any other login shell (fish) reports none.
    let a = agent("a");
    let started = Instant::now();
    assert_eq!(
        who_owns(
            Some("/bin/dash"),
            &[(&a, format!("{SLEEPER}\r"))],
            &["sleeper"]
        )
        .await,
        [true],
        "an agent's command in a shell without marks fell back to the person's default"
    );
    assert!(started.elapsed() < Duration::from_secs(8));
}

/// Without marks, an agent's job and a person's next Enter, which lands once
/// the job runs and before the paused sampler first sees it.
async fn who_owns_a_job_in_a_shell_without_marks(first: &TerminalActor, second: &TerminalActor) {
    assert_eq!(
        who_owns(
            Some("/bin/dash"),
            &[
                (first, "sleep 120 & echo $! > '{dir}/job'\r".to_string()),
                (second, "{after job}true\r".to_string()),
            ],
            &["job"]
        )
        .await,
        [*first != TerminalActor::User],
        "the job went to the later submitter"
    );
}

#[tokio::test]
async fn an_agents_job_stays_the_agents_when_a_person_submits_next_in_a_shell_without_marks() {
    who_owns_a_job_in_a_shell_without_marks(&agent("a"), &TerminalActor::User).await;
}

#[tokio::test]
async fn a_persons_job_stays_the_persons_when_an_agent_submits_next_in_a_shell_without_marks() {
    who_owns_a_job_in_a_shell_without_marks(&TerminalActor::User, &agent("a")).await;
}

#[tokio::test]
async fn typeahead_keeps_its_owner_while_the_command_before_it_runs_past_the_window() {
    // The first command outlasts the ten seconds an unreported Enter is kept.
    let a = agent("a");
    let shells = TempDir::new();
    for shell in integrated_shells(shells.path()) {
        assert_eq!(
            who_owns(
                shell.as_deref(),
                &[(&a, format!("sleep 11\r{SLEEPER}\r"))],
                &["sleeper"]
            )
            .await,
            [true],
            "{shell:?}: the second command lost its submitter while the first ran"
        );
    }
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
async fn a_typed_key_is_echoed_without_waiting_for_a_slow_tick() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["cat".to_string()]),
        })
        .expect("terminal opens")
        .terminal_id;
    let mut offset = service
        .read_output("room-1", &id, 0, Duration::from_millis(500))
        .await
        .unwrap()
        .next_offset;
    let mut echo_ms = Vec::new();
    for round in 0..20u64 {
        // Land at every phase of the pump interval.
        tokio::time::sleep(Duration::from_millis(7 + (round * 13) % 97)).await;
        let typed = Instant::now();
        type_as(&service, &id, &TerminalActor::User, "k").await;
        let page = service
            .read_output("room-1", &id, offset, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(page.next_offset > offset, "the echo arrives");
        offset = page.next_offset;
        echo_ms.push(typed.elapsed().as_millis());
    }
    echo_ms.sort_unstable();
    let median = echo_ms[echo_ms.len() / 2];
    assert!(
        median < 30,
        "typing waited for the 100 ms pump tick: echo times {echo_ms:?}"
    );
    service.end_session("room-1");
}

#[tokio::test]
async fn shell_exit_ends_the_terminal_and_keeps_its_screen() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    type_as(
        &service,
        &id,
        &TerminalActor::User,
        "echo kept-line; exit 3\r",
    )
    .await;
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
            outcome: RoomTerminalOutcome::ProgramExited {
                exit_code: Some(3),
                signal: None
            },
            description: "ended: exited with code 3".to_string()
        }
    );
    assert!(service
        .snapshot("room-1", &id)
        .unwrap()
        .screen_text
        .contains("kept-line"));
    assert!(service
        .input("room-1", &id, &TerminalActor::User, b"x")
        .await
        .is_err());
}

#[tokio::test]
async fn an_ended_terminal_keeps_no_session_to_sweep() {
    // Once the shell exits its PID, and with it the session id, is free for
    // reuse; archiving the agent hours later must not SIGKILL whichever
    // session leader holds that number by then.
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    type_as(&service, &id, &TerminalActor::User, "exit\r").await;
    until("the shell to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    assert!(
        service.lock().terminals[&id]
            .lock()
            .terminal
            .session_sweep
            .is_none(),
        "an ended terminal still holds the old shell's session for a later sweep"
    );
}

#[tokio::test]
async fn a_restart_ends_terminals_and_keeps_scrollback() {
    let root = TempDir::new();
    let first = service(root.path());
    let id = open(&first, root.path()).terminal_id;
    type_as(
        &first,
        &id,
        &TerminalActor::User,
        "seq 1 80; echo before-restart\r",
    )
    .await;
    until("output", || async {
        screen_has(&first, &id, "before-restart").await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    first.pump_once();

    until("periodic snapshot before simulated restart", || async {
        std::fs::read(root.path().join("terminals").join(format!("{id}.json")))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok())
            .is_some_and(|record| {
                record["screen_text"]
                    .as_str()
                    .is_some_and(|text| text.contains("before-restart"))
            })
    })
    .await;

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
    assert!(
        snapshot.scrollback_text.contains("\n2\n"),
        "{}",
        snapshot.scrollback_text
    );
    assert!(second
        .input("room-1", &id, &TerminalActor::User, b"x")
        .await
        .is_err());
    first.end_session("room-1");
}

#[tokio::test]
async fn closing_and_ending_a_room_stop_the_shell_processes() {
    let root = TempDir::new();
    let service = service(root.path());
    let a = open(&service, root.path()).terminal_id;
    let b = open(&service, root.path()).terminal_id;
    assert_eq!(
        service
            .list("room-1")
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["Terminal 1", "Terminal 2"]
    );
    service.close("room-1", &a, &TerminalActor::User).unwrap();
    assert_eq!(service.list("room-1").len(), 1);
    assert!(service.snapshot("room-1", &a).is_err());
    service.end_session("room-1");
    assert!(service.list("room-1").is_empty());
    assert!(service.snapshot("room-1", &b).is_err());
    assert!(!root
        .path()
        .join("terminals")
        .join(format!("{b}.json"))
        .exists());
}

#[tokio::test]
async fn notifications_fire_for_text_waiting_and_finish() {
    let root = TempDir::new();
    let events = Arc::new(FakeEvents::default());
    let service = service_with_events(root.path(), events.clone());
    let id = open(&service, root.path()).terminal_id;
    until("the initial shell prompt", || async {
        service.pump_once();
        service.terminal_for_test(&id).lock().terminal.mark_tracking
    })
    .await;

    service
        .watch(
            "room-1",
            &id,
            owner("w-text", "agent-a"),
            vec![RoomTerminalWatch::OutputContains {
                text: "NEEDLE-7".into(),
            }],
            None,
        )
        .unwrap();
    service
        .watch(
            "room-1",
            &id,
            owner("w-wait", "agent-a"),
            vec![RoomTerminalWatch::WaitingForInput],
            None,
        )
        .unwrap();
    let sent = type_as(
        &service,
        &id,
        &agent("agent-a"),
        "echo NEEDLE-$((3+4)); cat\r",
    )
    .await;
    assert!(sent.accepted, "the command was refused: {sent:?}");
    until("text and waiting events", || async {
        service.pump_once();
        !events.fired("w-text").is_empty() && !events.fired("w-wait").is_empty()
    })
    .await;
    assert!(matches!(
        &events.fired("w-text")[0].kind,
        RoomTerminalEventKind::OutputMatched { text } if text == "NEEDLE-7"
    ));
    let waiting = &events.fired("w-wait")[0];
    assert_eq!(waiting.kind, RoomTerminalEventKind::WaitingForInput);
    assert_eq!(waiting.armed_by, "agent-a");

    // `cat` returns to the prompt: the shell reports the program's status.
    service
        .watch(
            "room-1",
            &id,
            owner("w-program", "agent-a"),
            vec![RoomTerminalWatch::Finished],
            None,
        )
        .unwrap();
    type_as(&service, &id, &agent("agent-a"), "\u{4}").await;
    until("the program's finish event", || async {
        service.pump_once();
        !events.fired("w-program").is_empty()
    })
    .await;
    assert_eq!(
        events.fired("w-program")[0].kind,
        RoomTerminalEventKind::Finished {
            exit_code: Some(0),
            signal: None
        }
    );

    // The shell itself ends: its exit code is known.
    service
        .watch(
            "room-1",
            &id,
            owner("w-shell", "agent-a"),
            vec![RoomTerminalWatch::Finished],
            None,
        )
        .unwrap();
    type_as(&service, &id, &agent("agent-a"), "exit 5\r").await;
    until("the shell's finish event", || async {
        service.pump_once();
        !events.fired("w-shell").is_empty()
    })
    .await;
    let done = events.fired("w-shell");
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].kind,
        RoomTerminalEventKind::Finished {
            exit_code: Some(5),
            signal: None
        }
    );
    assert!(done[0].screen_text.contains("NEEDLE-7"));
}

#[tokio::test]
async fn a_blocking_wait_returns_when_the_program_finishes() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let sent = type_as(&service, &id, &agent("a"), "sleep 1; echo DONE-$((6*7))\r").await;
    assert!(sent.accepted);
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    let pump = service.clone();
    let pumping = tokio::spawn(async move {
        loop {
            pump.pump_once();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    let waited = service
        .wait(
            "room-1",
            &id,
            "a",
            vec![RoomTerminalWatch::Finished],
            Some(offset),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    assert!(matches!(
        waited.fired,
        Some(RoomTerminalEventKind::Finished { .. })
    ));
    assert!(
        waited.screen_text.contains("DONE-42"),
        "{}",
        waited.screen_text
    );
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn a_wait_that_times_out_loses_nothing() {
    let root = TempDir::new();
    let events = Arc::new(FakeEvents::default());
    let service = service_with_events(root.path(), events.clone());
    let id = open(&service, root.path()).terminal_id;
    let sent = type_as(&service, &id, &agent("a"), "sleep 4\r").await;
    assert!(sent.accepted, "input refused: {:?}", sent.refusal);
    until("sleep in the foreground", || async {
        service.pump_once();
        service.list("room-1")[0].driver
            == RoomTerminalDriver::Busy {
                driven_by: agent("a"),
            }
    })
    .await;
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    let waited = service
        .wait(
            "room-1",
            &id,
            "a",
            vec![RoomTerminalWatch::Finished],
            Some(offset),
            Duration::from_millis(300),
        )
        .await
        .unwrap();
    assert_eq!(waited.fired, None);

    service
        .watch(
            "room-1",
            &id,
            owner("later", "a"),
            vec![RoomTerminalWatch::Finished],
            Some(offset),
        )
        .unwrap();
    until("the notification armed after the timeout", || async {
        service.pump_once();
        !events.fired("later").is_empty()
    })
    .await;
    assert!(events.fired("wait:1").is_empty());
    service.end_session("room-1");
}

#[tokio::test]
async fn a_condition_that_held_before_arming_fires_at_once() {
    let root = TempDir::new();
    let events = Arc::new(FakeEvents::default());
    let service = service_with_events(root.path(), events.clone());
    let id = open(&service, root.path()).terminal_id;
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    type_as(&service, &id, &agent("a"), "echo EARLY-$((1+1))\r").await;
    until("the output", || async {
        screen_has(&service, &id, "EARLY-2").await
    })
    .await;
    let waited = service
        .wait(
            "room-1",
            &id,
            "a",
            vec![RoomTerminalWatch::OutputContains {
                text: "EARLY-2".into(),
            }],
            Some(offset),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(matches!(
        waited.fired,
        Some(RoomTerminalEventKind::OutputMatched { .. })
    ));
    service.end_session("room-1");
}

#[tokio::test]
async fn once_the_user_used_a_tab_only_the_user_can_close_it() {
    let root = TempDir::new();
    let service = service(root.path());
    let spec = |opened_by| OpenRoomTerminal {
        project_environment: Default::default(),
        session_id: "room-1".to_string(),
        working_directory: root.path().to_path_buf(),
        cols: 80,
        rows: 24,
        title: None,
        opened_by,
        command: None,
    };
    let id = service.open(spec(agent("a"))).unwrap().terminal_id;
    assert!(!service.list("room-1")[0].user_interacted);
    type_as(&service, &id, &agent("a"), "echo hi\r").await;
    type_as(&service, &id, &TerminalActor::User, "echo mine\r").await;
    assert!(service.list("room-1")[0].user_interacted);

    let refused = service.close("room-1", &id, &agent("a")).unwrap_err();
    assert!(
        refused.to_string().contains("only the user can close it"),
        "{refused}"
    );
    assert_eq!(service.list("room-1").len(), 1);
    service.close("room-1", &id, &TerminalActor::User).unwrap();
    assert!(service.list("room-1").is_empty());

    let resized = service.open(spec(agent("a"))).unwrap().terminal_id;
    // A viewing client sizes the terminal to its window; that is not use of the terminal.
    service.resize("room-1", &resized, 90, 30).unwrap();
    assert!(!service.list("room-1")[0].user_interacted);
    service.close("room-1", &resized, &agent("b")).unwrap();
    let untouched = service.open(spec(agent("a"))).unwrap().terminal_id;
    service.close("room-1", &untouched, &agent("b")).unwrap();
    service.end_session("room-1");
}

#[tokio::test]
async fn a_terminal_opened_with_a_command_belongs_to_its_opener() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 80,
            rows: 24,
            title: Some("build".to_string()),
            opened_by: agent("a"),
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "echo BUILT; sleep 30".into(),
            ]),
        })
        .unwrap()
        .terminal_id;
    until("the command output", || async {
        screen_has(&service, &id, "BUILT").await
    })
    .await;
    let summary = &service.list("room-1")[0];
    assert_eq!(summary.title, "build");
    assert_eq!(
        summary.driver,
        RoomTerminalDriver::Busy {
            driven_by: agent("a")
        }
    );
    assert!(!type_as(&service, &id, &agent("b"), "x").await.accepted);
    service.end_session("room-1");
}

#[tokio::test]
async fn a_graceful_shutdown_keeps_output_newer_than_the_last_save() {
    let root = TempDir::new();
    let first = service(root.path());
    let id = open(&first, root.path()).terminal_id;
    type_as(
        &first,
        &id,
        &TerminalActor::User,
        "echo LAST-WORDS-$((8*8))\r",
    )
    .await;
    until("output", || async {
        screen_has(&first, &id, "LAST-WORDS-64").await
    })
    .await;
    first.shutdown();

    let second = service(root.path());
    let snapshot = second.snapshot("room-1", &id).unwrap();
    assert!(snapshot.screen_text.contains("LAST-WORDS-64"));
    assert_eq!(
        snapshot.terminal.status,
        RoomTerminalStatus::Ended {
            outcome: RoomTerminalOutcome::KernelRestarted,
            description: "ended: kernel restarted".to_string()
        }
    );
    first.end_session("room-1");
}

#[tokio::test]
async fn a_graceful_shutdown_stops_disowned_jobs_and_preserves_the_terminal() {
    let root = TempDir::new();
    let first = service(root.path());
    let id = open(&first, root.path()).terminal_id;
    let background = root.path().join("shutdown-job.pid");
    type_as(
        &first,
        &id,
        &TerminalActor::User,
        &format!(
            "nohup sleep 8884 >/dev/null 2>&1 & echo $! > '{}'; disown; echo SHUTDOWN-SCREEN\r",
            background.display()
        ),
    )
    .await;
    until("the disowned job and its output", || async {
        job_running(&background) && screen_has(&first, &id, "SHUTDOWN-SCREEN").await
    })
    .await;
    let shell_pid = first
        .terminal_for_test(&id)
        .lock()
        .terminal
        .shell_pid
        .unwrap();
    let shell = root.path().join("shell.pid");
    std::fs::write(&shell, shell_pid.to_string()).unwrap();
    first.shutdown();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stopped = !job_running(&background) && !job_running(&shell);
    let second = service(root.path());
    let recovered = second.snapshot("room-1", &id);
    // Always clean up, including on the unfixed implementation.
    first.end_session("room-1");
    assert!(
        stopped,
        "shutdown left the shell or its disowned job running"
    );
    let snapshot = recovered.unwrap();
    assert!(snapshot.screen_text.contains("SHUTDOWN-SCREEN"));
    assert!(matches!(
        snapshot.terminal.status,
        RoomTerminalStatus::Ended {
            outcome: RoomTerminalOutcome::KernelRestarted,
            ..
        }
    ));
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

#[tokio::test]
async fn closing_a_terminal_also_stops_background_jobs() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let background = root.path().join("background.pid");
    let foreground = root.path().join("foreground.pid");
    type_as(&service, &id, &TerminalActor::User, &format!(
        "nohup sleep 8881 >/dev/null 2>&1 & echo $! > '{}'; disown; sh -c 'echo $$ > \"{}\"; exec sleep 8882'\r",
        background.display(), foreground.display(),
    )).await;
    until("both jobs", || async {
        job_running(&background) && job_running(&foreground)
    })
    .await;
    service.close("room-1", &id, &TerminalActor::User).unwrap();
    until("both jobs to stop", || async {
        !job_running(&background) && !job_running(&foreground)
    })
    .await;
}

#[tokio::test]
async fn a_shell_that_exits_leaves_no_background_jobs() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let background = root.path().join("background.pid");
    type_as(
        &service,
        &id,
        &TerminalActor::User,
        &format!(
            "nohup sleep 8883 >/dev/null 2>&1 & echo $! > '{}'; disown; exit\r",
            background.display()
        ),
    )
    .await;
    until("the shell to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    assert!(background.is_file());
    until("the background job to stop", || async {
        !job_running(&background)
    })
    .await;
}

#[tokio::test]
async fn a_job_that_left_the_session_outlives_its_command() {
    let root = TempDir::new();
    let service = service(root.path());
    let detached = root.path().join("detached.pid");
    let attached = root.path().join("attached.pid");
    // The pause lets the resource watch observe both jobs before the command ends.
    let script = format!(
        "{} </dev/null >/dev/null 2>&1 & \
         sh -c 'echo $$ > \"{}\"; exec sleep 8886' </dev/null >/dev/null 2>&1 & sleep 1",
        crate::test_support::session_job(&format!("'{}'", detached.display()), 8885),
        attached.display(),
    );
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        })
        .expect("terminal opens")
        .terminal_id;
    until("the command to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    until("the job still in the session to stop", || async {
        !job_running(&attached)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let running = job_running(&detached);
    let pid = std::fs::read_to_string(&detached)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
        .expect("the detached job started");
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert!(
        running,
        "a job that left the terminal's session ended with it"
    );
    service.close("room-1", &id, &TerminalActor::User).unwrap();
}

#[tokio::test]
async fn a_command_ends_while_its_background_child_holds_the_terminal_output() {
    let root = TempDir::new();
    let service = service(root.path());
    let child = root.path().join("child.pid");
    // The child keeps the command's stdout and stderr: the PTY stays open.
    let script = format!(
        "sh -c 'echo $$ > \"{}\"; exec sleep 8899' &",
        child.display()
    );
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        })
        .expect("terminal opens")
        .terminal_id;
    until("the command to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended {
                outcome: RoomTerminalOutcome::ProgramExited {
                    exit_code: Some(0),
                    ..
                },
                ..
            }
        )
    })
    .await;
    until("the background child to stop", || async {
        !job_running(&child)
    })
    .await;
    service.close("room-1", &id, &TerminalActor::User).ok();
}

#[tokio::test]
async fn a_job_detaching_as_its_command_returns_keeps_running() {
    let root = TempDir::new();
    let service = service(root.path());
    let detached = root.path().join("late.pid");
    // The command returns at once: the job is still detaching when it ends.
    let script = format!(
        "nohup {} </dev/null >/dev/null 2>&1 &",
        crate::test_support::detached_job(&format!("'{}'", detached.display()), 8887),
    );
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        })
        .expect("terminal opens")
        .terminal_id;
    until("the command to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let running = job_running(&detached);
    if let Some(pid) = std::fs::read_to_string(&detached)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    {
        // SAFETY: the positive PID of the job this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(
        running,
        "a job detaching as its command returned was killed"
    );
    service.close("room-1", &id, &TerminalActor::User).unwrap();
}

#[tokio::test]
async fn a_detached_job_stays_listed_counted_and_stoppable_under_its_terminal() {
    let root = TempDir::new();
    let service = service(root.path());
    let detached = root.path().join("owned.pid");
    let script = format!(
        "nohup {} </dev/null >/dev/null 2>&1 &",
        crate::test_support::detached_job(&format!("'{}'", detached.display()), 8893),
    );
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        })
        .expect("terminal opens")
        .terminal_id;
    until("the command to end", || async {
        service.pump_once();
        matches!(
            service.list("room-1")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    // The Room's process list (Health) reads the same projection.
    let providers = crate::provider::ProviderProcessServiceStore::new(
        crate::provider::ProviderProcessService::new(),
    );
    let owner = crate::runtime::process_resources::SessionProcessOwner::Terminal(id.clone());
    let listed = || {
        let pid = std::fs::read_to_string(&detached)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        crate::runtime::process_resources::for_session("room-1", &providers, &service)
            .into_iter()
            .find(|row| Some(row.process.pid) == pid && row.owner == owner)
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while listed().is_none() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Some(row) = listed() else {
        if let Some(pid) = std::fs::read_to_string(&detached)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
        {
            // SAFETY: the positive PID of the job this test started; never a group.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        panic!("the detached job is not listed under its terminal");
    };
    assert!(
        row.process.resident_set_bytes > 0,
        "the job's memory is not counted"
    );
    let key = crate::runtime::process_resources::keys_for_session("room-1", &providers, &service)
        .into_iter()
        .find(|(current, _)| *current == owner)
        .map(|(_, key)| key)
        .expect("the owner keeps a stop handle");
    let stopped =
        crate::runtime::process_resources::stop_for_key(&key, Some(row.process.process_id.clone()));
    let running = job_running(&detached);
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(row.process.pid as i32, libc::SIGKILL) };
    assert!(stopped.is_ok(), "stop from the list failed: {stopped:?}");
    assert!(!running, "the stopped job still runs");
    service.close("room-1", &id, &TerminalActor::User).unwrap();
}

#[tokio::test]
async fn a_detached_job_ends_with_its_room() {
    let root = TempDir::new();
    let service = service(root.path());
    let detached = root.path().join("room.pid");
    let script = format!(
        "nohup {} </dev/null >/dev/null 2>&1 &",
        crate::test_support::detached_job(&format!("'{}'", detached.display()), 8894),
    );
    service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-2".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        })
        .expect("terminal opens");
    until("the command to end", || async {
        service.pump_once();
        matches!(
            service.list("room-2")[0].status,
            RoomTerminalStatus::Ended { .. }
        )
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let started = job_running(&detached);
    service.end_session("room-2");
    crate::runtime::process_resources::stop_released(Some("room-2"), None);
    let running = job_running(&detached);
    if let Some(pid) = std::fs::read_to_string(&detached)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    {
        // SAFETY: the positive PID of the job this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(started, "the detached job did not start");
    assert!(!running, "a detached job outlived its Room");
}

#[test]
fn a_command_argument_over_the_limit_is_refused_with_that_limit() {
    let root = TempDir::new();
    let service = service(root.path());
    let error = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/sh".into(), "-c".into(), "x".repeat(4097)]),
        })
        .expect_err("a 4097-byte argument is refused");
    assert!(
        error.to_string().contains("each at most 4096 bytes"),
        "{error}"
    );
}

#[test]
fn the_shell_keeps_the_users_ssh_agent_but_not_kernel_tokens() {
    let _environment = crate::env_lock::lock();
    std::env::set_var("SSH_AUTH_SOCK", "users-ssh-agent");
    std::env::set_var("CHARIOX_RELAY_TOKEN", "kernel-private-token");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = TempDir::new();
            let service = service(root.path());
            let id = open(&service, root.path()).terminal_id;
            type_as(
                &service,
                &id,
                &TerminalActor::User,
                "echo AGENT=$SSH_AUTH_SOCK TOKEN=${CHARIOX_RELAY_TOKEN:-none}\r",
            )
            .await;
            until("the environment", || async {
                screen_has(&service, &id, "AGENT=users-ssh-agent TOKEN=none").await
            })
            .await;
            service.end_session("room-1");
        });
    std::env::remove_var("SSH_AUTH_SOCK");
    std::env::remove_var("CHARIOX_RELAY_TOKEN");
}

/// Types `line` and waits for the shell to report it finished, without any
/// sleep in the command: the finish must not depend on sampling.
async fn finish_of(
    service: &RoomTerminalService,
    id: &str,
    line: &str,
) -> Option<RoomTerminalEventKind> {
    let offset = service.snapshot("room-1", id).unwrap().output_offset;
    assert!(type_as(service, id, &agent("a"), line).await.accepted);
    service
        .wait(
            "room-1",
            id,
            "a",
            vec![RoomTerminalWatch::Finished],
            Some(offset),
            Duration::from_secs(10),
        )
        .await
        .unwrap()
        .fired
}

#[tokio::test]
async fn typed_commands_report_finish_and_exit_code_even_when_instant() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let pump = service.clone();
    let pumping = tokio::spawn(async move {
        loop {
            pump.pump_once();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    for (line, expected) in [
        ("echo builtin-done\r", 0),
        ("false\r", 1),
        ("/bin/true\r", 0),
        ("(exit 7)\r", 7),
        ("(:)&\r", 0),
    ] {
        assert_eq!(
            finish_of(&service, &id, line).await,
            Some(RoomTerminalEventKind::Finished {
                exit_code: Some(expected),
                signal: None
            }),
            "{line}"
        );
    }
    // A background subshell must not report a late start after the prompt.
    tokio::time::sleep(Duration::from_millis(100)).await;
    service.pump_once();
    assert_eq!(service.list("room-1")[0].driver, RoomTerminalDriver::Free);
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn a_finished_wait_belongs_to_the_command_that_was_submitted() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let pump = service.clone();
    let pumping = tokio::spawn(async move {
        loop {
            pump.pump_once();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    // The agent reads its offset at the prompt; before its line is typed the
    // user runs a command that ends after that offset.
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    assert!(
        type_as(&service, &id, &TerminalActor::User, "(exit 1)\r")
            .await
            .accepted
    );
    // The agent may only type once the user's command is over; on a slow
    // machine it can still be running when the next line is typed.
    until("the user's command to end", || async {
        let terminal_owner = service.terminal_for_test(&id);
        let state = terminal_owner.lock();
        let terminal = &state.terminal;
        terminal.commands_started == 1 && !terminal.command_in_flight
    })
    .await;
    assert!(
        type_as(&service, &id, &agent("a"), "(exit 5)\r")
            .await
            .accepted
    );
    let fired = service
        .wait(
            "room-1",
            &id,
            "a",
            vec![RoomTerminalWatch::Finished],
            Some(offset),
            Duration::from_secs(10),
        )
        .await
        .unwrap()
        .fired;
    assert_eq!(
        fired,
        Some(RoomTerminalEventKind::Finished {
            exit_code: Some(5),
            signal: None
        })
    );
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn an_agent_is_admitted_after_a_finish_the_shell_already_reported() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    until("the first prompt", || async {
        service.terminal_for_test(&id).lock().terminal.mark_tracking
    })
    .await;
    let release = root.path().join("release");
    let ended = root.path().join("ended");
    let line = format!(
        "sh -c 'until [ -e {} ]; do sleep 0.01; done; touch {}; exit 1'\r",
        release.display(),
        ended.display()
    );
    assert!(
        type_as(&service, &id, &TerminalActor::User, &line)
            .await
            .accepted
    );
    until("the user's command to run", || async {
        service
            .terminal_for_test(&id)
            .lock()
            .terminal
            .command_in_flight
    })
    .await;
    // This test runs on one thread and does not yield from here on, so the
    // pump cannot read the shell's report that the command is over.
    std::fs::write(&release, "").unwrap();
    wait_blocking("the command to end", || ended.exists());
    // The shell reports the end right after the command exits.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        service
            .terminal_for_test(&id)
            .lock()
            .terminal
            .command_in_flight,
        "the pump has not read the finish report yet"
    );
    assert!(
        type_as(&service, &id, &agent("a"), "(exit 5)\r")
            .await
            .accepted
    );
    service.end_session("room-1");
}

fn wait_blocking(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn finished_with(code: u32) -> Option<RoomTerminalEventKind> {
    Some(RoomTerminalEventKind::Finished {
        exit_code: Some(code),
        signal: None,
    })
}

async fn wait_finished_after(
    service: &RoomTerminalService,
    id: &str,
    offset: u64,
) -> Option<RoomTerminalEventKind> {
    service
        .wait(
            "room-1",
            id,
            "a",
            vec![RoomTerminalWatch::Finished],
            Some(offset),
            Duration::from_secs(10),
        )
        .await
        .unwrap()
        .fired
}

fn pump_in_background(service: &RoomTerminalService) -> tokio::task::JoinHandle<()> {
    let pump = service.clone();
    tokio::spawn(async move {
        loop {
            pump.pump_once();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

#[tokio::test]
async fn a_second_send_by_the_same_agent_waits_for_its_own_command() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let pumping = pump_in_background(&service);
    assert!(
        type_as(&service, &id, &agent("a"), "(sleep 1; exit 3)\r")
            .await
            .accepted
    );
    until("the first command to run", || async {
        service
            .terminal_for_test(&id)
            .lock()
            .terminal
            .command_in_flight
    })
    .await;
    // The second line waits in the terminal's input until the first command
    // returns to the prompt.
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    assert!(
        type_as(&service, &id, &agent("a"), "(exit 5)\r")
            .await
            .accepted
    );
    assert_eq!(
        wait_finished_after(&service, &id, offset).await,
        finished_with(5)
    );
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn two_sends_before_the_next_pump_wait_for_their_own_commands() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    assert!(
        type_as(&service, &id, &agent("a"), "(sleep 1; exit 3)\r")
            .await
            .accepted
    );
    let second = service.snapshot("room-1", &id).unwrap().output_offset;
    assert!(
        type_as(&service, &id, &agent("a"), "(exit 5)\r")
            .await
            .accepted
    );
    let pumping = pump_in_background(&service);
    assert_eq!(
        wait_finished_after(&service, &id, second).await,
        finished_with(5)
    );
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn input_a_running_command_reads_does_not_hide_its_finish() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let pumping = pump_in_background(&service);
    assert!(
        type_as(
            &service,
            &id,
            &agent("a"),
            "read -r answer; exit_code=4; (exit $exit_code)\r"
        )
        .await
        .accepted
    );
    until("the command to run", || async {
        service
            .terminal_for_test(&id)
            .lock()
            .terminal
            .command_in_flight
    })
    .await;
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    // The running command takes this line, so no other command follows it.
    assert!(
        type_as(&service, &id, &agent("a"), "answer\r")
            .await
            .accepted
    );
    assert_eq!(
        wait_finished_after(&service, &id, offset).await,
        finished_with(4)
    );
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn a_shell_started_inside_the_terminal_does_not_repeat_the_marks() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let pump = service.clone();
    let pumping = tokio::spawn(async move {
        loop {
            pump.pump_once();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    // Input typed while the shell is still starting is refused as busy (or lands
    // before the marks are on). Wait for the first prompt.
    until("the shell's first prompt", || async {
        service.pump_once();
        let terminal_owner = service.terminal_for_test(&id);
        let state = terminal_owner.lock();
        state.terminal.mark_tracking && !state.terminal.command_in_flight
    })
    .await;
    // The nested shell prints no marks of its own, so the outer command is
    // reported finished only when the nested shell exits.
    let fired = finish_of(&service, &id, "bash -c 'echo $PROMPT_COMMAND-[$PS0]'\r").await;
    assert!(matches!(
        fired,
        Some(RoomTerminalEventKind::Finished {
            exit_code: Some(0),
            ..
        })
    ));
    let screen = service.snapshot("room-1", &id).unwrap().screen_text;
    assert!(screen.contains("-[]"), "{screen}");
    pumping.abort();
    service.end_session("room-1");
}

#[tokio::test]
async fn builtin_read_keeps_its_driver_until_the_command_finishes() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    for driver in [agent("a"), TerminalActor::User] {
        assert!(
            type_as(&service, &id, &driver, "read -r answer\r")
                .await
                .accepted
        );
        until("the shell command start", || async {
            service.pump_once();
            service
                .terminal_for_test(&id)
                .lock()
                .terminal
                .command_in_flight
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let still_waiting = service
            .wait(
                "room-1",
                &id,
                "b",
                vec![RoomTerminalWatch::Finished],
                None,
                Duration::from_millis(150),
            )
            .await
            .unwrap();
        assert!(
            still_waiting.fired.is_none(),
            "builtin read finished before receiving input"
        );
        assert_eq!(still_waiting.terminal.status, RoomTerminalStatus::Running);
        let refused = type_as(&service, &id, &agent("b"), "intruder\r").await;
        let owned = service.list("room-1")[0].driver.clone();
        // Clean up even on the RED path so a failed assertion leaves no PTY.
        assert!(
            type_as(&service, &id, &TerminalActor::User, "owner-answer\r")
                .await
                .accepted
        );
        until("read to finish", || async {
            service.pump_once();
            !service
                .terminal_for_test(&id)
                .lock()
                .terminal
                .command_in_flight
        })
        .await;
        if refused.accepted {
            service.end_session("room-1");
        }
        assert!(!refused.accepted, "another agent answered the builtin read");
        assert_eq!(
            owned,
            RoomTerminalDriver::Busy {
                driven_by: driver.clone()
            }
        );
        assert!(
            type_as(&service, &id, &driver, "read -r another\r")
                .await
                .accepted
        );
        until("another read to start", || async {
            service.pump_once();
            service
                .terminal_for_test(&id)
                .lock()
                .terminal
                .command_in_flight
        })
        .await;
        assert!(
            type_as(&service, &id, &driver, "driver-answer\r")
                .await
                .accepted
        );
        until("the driver's read to finish", || async {
            service.pump_once();
            !service
                .terminal_for_test(&id)
                .lock()
                .terminal
                .command_in_flight
        })
        .await;
    }
    service.end_session("room-1");
}

#[tokio::test]
async fn an_output_marker_survives_a_large_burst_in_one_pump_batch() {
    let root = TempDir::new();
    let service = service(root.path());
    // Apply bytes directly so the test is independent of PTY reader chunking.
    let id = open(&service, root.path()).terminal_id;
    let terminal_owner = service.terminal_for_test(&id);
    let mut state = terminal_owner.lock();
    let terminal = &mut state.terminal;
    terminal.watches.arm(
        owner("burst", "a"),
        RoomTerminalWatch::OutputContains {
            text: "ready-marker".into(),
        },
        OutputMatcher::new(&terminal.plain),
        false,
        FinishBinding::default(),
    );
    let now = Instant::now();
    let mut fired = terminal.apply_output(b"ready-", now);
    let mut burst = b"marker".to_vec();
    burst.extend(vec![b'x'; 32 * 1024]);
    fired.extend(terminal.apply_output(&burst, now));
    fired.extend(terminal.watches.on_output(&terminal.plain));
    let long_needle = format!("{}tail", "λ".repeat(10_000));
    terminal.watches.arm(
        owner("long-burst", "a"),
        RoomTerminalWatch::OutputContains {
            text: long_needle.clone(),
        },
        OutputMatcher::new(&terminal.plain),
        false,
        FinishBinding::default(),
    );
    for part in long_needle.as_bytes().chunks(1023) {
        fired.extend(terminal.apply_output(part, now));
    }
    let long_matched = fired.iter().any(|(owner, kind)| {
        owner.wake_id == "long-burst"
            && matches!(kind, RoomTerminalEventKind::OutputMatched { text } if text == &long_needle)
    });
    drop(state);
    service.end_session("room-1");
    assert!(
        long_matched,
        "the matcher lost a needle larger than its text window"
    );
    assert!(fired.iter().any(|(owner, kind)| owner.wake_id == "burst"
        && matches!(kind,
        RoomTerminalEventKind::OutputMatched { text } if text == "ready-marker")));
}

#[tokio::test]
async fn a_late_output_wait_matches_a_marker_before_a_large_burst() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let offset = service.snapshot("room-1", &id).unwrap().output_offset;
    type_as(
        &service,
        &id,
        &agent("a"),
        "printf 'late-\\033[32mready\\033[0m'; printf '%32768s' x\r",
    )
    .await;
    until("all output before registering the wait", || async {
        service.pump_once();
        service
            .terminal_for_test(&id)
            .lock()
            .terminal
            .log
            .end_offset()
            > offset + 32768
    })
    .await;
    let waited = service
        .wait(
            "room-1",
            &id,
            "a",
            vec![RoomTerminalWatch::OutputContains {
                text: "late-ready".into(),
            }],
            Some(offset),
            Duration::from_millis(100),
        )
        .await
        .unwrap();
    service.end_session("room-1");
    assert_eq!(
        waited.fired,
        Some(RoomTerminalEventKind::OutputMatched {
            text: "late-ready".into()
        })
    );
}

#[tokio::test]
async fn a_start_mark_alone_keeps_foreground_completion_available() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = open(&service, root.path()).terminal_id;
    let terminal_owner = service.terminal_for_test(&id);
    let mut state = terminal_owner.lock();
    let terminal = &mut state.terminal;
    terminal.mark_tracking = false;
    terminal.apply_output(b"\x1b]133;C\x07", Instant::now());
    let tracking = terminal.mark_tracking;
    drop(state);
    service.end_session("room-1");
    assert!(
        !tracking,
        "a start-only hook disabled the foreground fallback"
    );
}

/// Runs `call` on its own thread so a hang fails the test instead of
/// freezing it.
fn finishes_within<T: Send + 'static>(
    wait: Duration,
    call: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (done, result) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(call());
    });
    result.recv_timeout(wait).ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_that_stops_reading_does_not_lock_the_others() {
    let root = TempDir::new();
    let service = service(root.path());
    let blocked = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "stty -icanon -echo; echo NOT-READING; exec sleep 600".into(),
            ]),
        })
        .unwrap()
        .terminal_id;
    let other = open(&service, root.path()).terminal_id;
    until("the program to stop reading", || async {
        screen_has(&service, &blocked, "NOT-READING").await
    })
    .await;

    // Far more than the tty input buffer holds.
    let flood = {
        let service = service.clone();
        let blocked = blocked.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            runtime.block_on(service.input(
                "room-1",
                &blocked,
                &TerminalActor::User,
                &vec![b'x'; 64 * 1024],
            ))
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    let others_work = {
        let service = service.clone();
        let other = other.clone();
        finishes_within(Duration::from_secs(5), move || {
            service.pump_once();
            service.snapshot("room-1", &other).is_ok() && service.list("room-1").len() == 2
        })
    };
    assert_eq!(
        others_work,
        Some(true),
        "a stuck write locked every terminal"
    );
    let closed = {
        let service = service.clone();
        let blocked = blocked.clone();
        finishes_within(Duration::from_secs(5), move || {
            service.close("room-1", &blocked, &TerminalActor::User)
        })
    };
    assert!(
        matches!(closed, Some(Ok(()))),
        "the stuck terminal could not be closed"
    );
    assert!(flood.join().is_ok());
    service.end_session("room-1");
}

/// What the user pasted before Ctrl-C must not run later: the interrupt
/// discards it, the shell that resumes reading gets only what is typed
/// afterwards, and the terminal says how much was dropped.
#[tokio::test]
async fn ctrl_c_discards_the_paste_it_overtakes() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "stty -icanon -echo; trap : INT; echo NOT-READING; sleep 600; \
                 stty sane; echo SHELL-IS-BACK; exec sh"
                    .into(),
            ]),
        })
        .unwrap()
        .terminal_id;
    until("the program to stop reading", || async {
        screen_has(&service, &id, "NOT-READING").await
    })
    .await;
    // Junk the program never reads, then a command a shell would run.
    let junk = vec![b'x'; 60 * 1024];
    let command = b"\necho STALE-$((6*7))\n";
    for paste in [&junk[..], &junk[..], &command[..]] {
        let queued = service
            .input("room-1", &id, &TerminalActor::User, paste)
            .await
            .unwrap();
        assert!(queued.accepted);
    }
    let paste_len = (2 * junk.len() + command.len()) as u64;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let interrupt = service
        .input("room-1", &id, &TerminalActor::User, b"\x03")
        .await
        .unwrap();
    assert!(interrupt.accepted);
    until("the shell to come back", || async {
        screen_has(&service, &id, "SHELL-IS-BACK").await
    })
    .await;
    // A fresh command works, and runs before anything stale could.
    let fresh = service
        .input(
            "room-1",
            &id,
            &TerminalActor::User,
            b"echo FRESH-$((6*7))\n",
        )
        .await
        .unwrap();
    assert!(fresh.accepted);
    until("the fresh command to run", || async {
        screen_has(&service, &id, "FRESH-42").await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    service.pump_once();
    let snapshot = service.snapshot("room-1", &id).unwrap();
    assert!(
        !snapshot.screen_text.contains("STALE-42")
            && !snapshot.scrollback_text.contains("STALE-42"),
        "the discarded paste ran: {}{}",
        snapshot.scrollback_text,
        snapshot.screen_text
    );
    let dropped = snapshot.terminal.dropped_input_bytes;
    assert!(
        dropped > 0 && dropped < paste_len,
        "the terminal reports what the interrupt dropped, got {dropped} of {paste_len}"
    );
    service.end_session("room-1");
}

#[tokio::test]
async fn a_needle_split_across_the_wait_registration_still_matches() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: agent("a"),
            command: Some(vec!["sleep".into(), "30".into()]),
        })
        .unwrap()
        .terminal_id;
    let terminal_owner = service.terminal_for_test(&id);
    let mut state = terminal_owner.lock();
    let terminal = &mut state.terminal;
    let now = Instant::now();
    let offset = terminal.log.end_offset();
    terminal.apply_output(b"ready-", now);
    let at_once = terminal.arm_watch(
        owner("split", "a"),
        RoomTerminalWatch::OutputContains {
            text: "ready-marker".into(),
        },
        Some(offset),
        now,
        true,
    );
    let fired = terminal.apply_output(b"marker", now);
    drop(state);
    service.end_session("room-1");
    assert_eq!(at_once, None);
    assert!(
        fired.iter().any(|(owner, kind)| owner.wake_id == "split"
            && matches!(kind, RoomTerminalEventKind::OutputMatched { text } if text == "ready-marker")),
        "the half of the text that came before the wait was forgotten"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_stops_the_program_behind_a_blocked_paste() {
    let root = TempDir::new();
    let service = service(root.path());
    let id = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".to_string(),
            working_directory: root.path().to_path_buf(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "stty -icanon -echo; trap : INT; echo NOT-READING; sleep 600; \
                 echo SHELL-STILL-ALIVE; exec sleep 600"
                    .into(),
            ]),
        })
        .unwrap()
        .terminal_id;
    until("the program to stop reading", || async {
        screen_has(&service, &id, "NOT-READING").await
    })
    .await;
    let flood = {
        let service = service.clone();
        let id = id.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            runtime.block_on(service.input(
                "room-1",
                &id,
                &TerminalActor::User,
                &vec![b'x'; 64 * 1024],
            ))
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    let interrupt = service
        .input("room-1", &id, &TerminalActor::User, b"\x03")
        .await
        .unwrap();
    assert!(interrupt.accepted);
    until(
        "Ctrl-C to stop the program and leave the shell running",
        || async { screen_has(&service, &id, "SHELL-STILL-ALIVE").await },
    )
    .await;
    assert!(flood.join().is_ok());
    service.end_session("room-1");
}

#[test]
fn an_output_read_never_waits_past_the_service_cap() {
    use super::service::{output_wait, MAX_WAIT};
    assert_eq!(output_wait(Duration::from_secs(600)), MAX_WAIT);
    assert_eq!(output_wait(Duration::from_secs(5)), Duration::from_secs(5));
}

#[test]
fn memory_limit_wakes_all_watch_kinds_with_the_limit() {
    let plain = super::plain_text::PlainText::new();
    let mut watches = super::events::WatchSet::default();
    for (number, watch) in [
        RoomTerminalWatch::Finished,
        RoomTerminalWatch::OutputContains {
            text: "never".into(),
        },
        RoomTerminalWatch::WaitingForInput,
    ]
    .into_iter()
    .enumerate()
    {
        watches.arm(
            WatchOwner {
                wake_id: format!("memory-{number}"),
                agent_id: "agent".into(),
            },
            watch,
            OutputMatcher::new(&plain),
            false,
            FinishBinding::default(),
        );
    }
    let fired = watches.on_memory_limit(32 * 1024 * 1024);
    assert_eq!(fired.len(), 3);
    assert!(fired.iter().all(|(_, kind)| matches!(
        kind,
        RoomTerminalEventKind::MemoryLimit {
            limit_bytes: 33554432
        }
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_output_flood_preserves_its_tail_and_leaves_other_terminals_responsive() {
    let root = TempDir::new();
    let service = service(root.path());
    let open = |command| OpenRoomTerminal {
        project_environment: Default::default(),
        session_id: "room-1".into(),
        working_directory: root.path().to_path_buf(),
        cols: 80,
        rows: 24,
        title: None,
        opened_by: TerminalActor::User,
        command: Some(command),
    };
    let flood = service
        .open(open(vec![
            "python3".into(),
            "-c".into(),
            "import sys; sys.stdout.buffer.write(b'x'*8_000_000+b'FLOOD_END'); sys.stdout.flush()"
                .into(),
        ]))
        .unwrap()
        .terminal_id;
    let other = service
        .open(open(vec!["/bin/cat".into()]))
        .unwrap()
        .terminal_id;
    let reply = service
        .input("room-1", &other, &TerminalActor::User, b"OTHER_OK\n")
        .await
        .unwrap();
    assert!(reply.accepted);
    until("other terminal echo", || async {
        screen_has(&service, &other, "OTHER_OK").await
    })
    .await;
    until("flood tail before exit", || async {
        screen_has(&service, &flood, "FLOOD_END").await
    })
    .await;
    until("flood exits after output EOF", || async {
        service.list("room-1").iter().any(|terminal| {
            terminal.terminal_id == flood
                && matches!(
                    terminal.status,
                    super::types::RoomTerminalStatus::Ended { .. }
                )
        })
    })
    .await;
    service.end_session("room-1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_waits_for_its_commit_without_holding_the_terminal_registry() {
    let root = TempDir::new();
    let service = service(root.path());
    let existing = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".into(),
            working_directory: root.path().into(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/cat".into()]),
        })
        .unwrap()
        .terminal_id;
    let (entered, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    struct Release(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let release = Release(Some(release));
    let barrier = service.pause_writer_for_test(entered, released);
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let opener = service.clone();
    let directory = root.path().to_path_buf();
    let (done, completion) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = opener.open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".into(),
            working_directory: directory,
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/cat".into()]),
        });
        let _ = done.send(result);
    });
    until("new terminal admitted while commit waits", || async {
        service.list("room-1").len() == 2
    })
    .await;
    assert!(
        matches!(
            completion.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "open acknowledged before its commit"
    );
    service
        .input(
            "room-1",
            &existing,
            &TerminalActor::User,
            b"DURING_COMMIT\n",
        )
        .await
        .unwrap();
    until("unrelated echo during commit", || async {
        screen_has(&service, &existing, "DURING_COMMIT").await
    })
    .await;
    drop(release);
    barrier.wait().unwrap();
    completion
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    service.end_session("room-1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_parser_does_not_delay_another_terminals_echo() {
    unrelated_echo_while_screen_paused(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_renderer_does_not_delay_another_terminals_echo() {
    unrelated_echo_while_screen_paused(true).await;
}

async fn unrelated_echo_while_screen_paused(rendering: bool) {
    use base64::Engine as _;
    let root = TempDir::new();
    let service = service(root.path());
    let cat = || {
        service
            .open(OpenRoomTerminal {
                project_environment: Default::default(),
                session_id: "room-1".into(),
                working_directory: root.path().into(),
                cols: 80,
                rows: 24,
                title: None,
                opened_by: TerminalActor::User,
                command: Some(vec!["/bin/cat".into()]),
            })
            .unwrap()
            .terminal_id
    };
    let busy = cat();
    let other = cat();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    {
        let terminal_owner = service.terminal_for_test(&busy);
        let mut state = terminal_owner.lock();
        let hook = Some((entered, released));
        if rendering {
            state.terminal.screen.pause_text = hook;
        } else {
            state.terminal.screen.pause_feed = hook;
        }
    }
    let busy_writer = service.clone();
    let busy_runtime = tokio::runtime::Handle::current();
    let busy_thread = std::thread::spawn(move || {
        if rendering {
            busy_writer.snapshot("room-1", &busy).unwrap();
        } else {
            busy_runtime
                .block_on(busy_writer.input("room-1", &busy, &TerminalActor::User, b"BUSY\n"))
                .unwrap();
        }
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let writer = service.clone();
    let reader = service.clone();
    let runtime = tokio::runtime::Handle::current();
    let (done, completion) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = runtime.block_on(async {
            writer.resize("room-1", &other, 90, 30).unwrap();
            writer.snapshot("room-1", &other).unwrap();
            writer
                .input("room-1", &other, &TerminalActor::User, b"INDEPENDENT\n")
                .await
                .unwrap();
            let output = reader
                .read_output("room-1", &other, 0, Duration::from_millis(500))
                .await
                .unwrap();
            base64::engine::general_purpose::STANDARD
                .decode(output.data_base64)
                .unwrap()
        });
        let _ = done.send(result);
    });
    let echoed = completion.recv_timeout(Duration::from_millis(750));
    release.send(()).unwrap();
    busy_thread.join().unwrap();
    thread.join().unwrap();
    service.end_session("room-1");
    assert!(
        echoed.is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains("INDEPENDENT")),
        "another terminal's echo waited for paused screen work"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exited_terminal_does_not_become_work_when_its_state_is_locked() {
    let root = TempDir::new();
    let service = service(root.path());
    let terminal = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".into(),
            working_directory: root.path().into(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/true".into()]),
        })
        .unwrap();
    let owner = service.terminal_for_test(&terminal.terminal_id);
    until("terminal exit", || async {
        service.pump_once();
        !owner.lock().terminal.is_running()
    })
    .await;
    assert!(!service.has_running_work());
    // Rendering or persistence of an exited terminal cannot invent a new work cycle.
    let state = owner.lock();
    assert!(
        !service.has_running_work(),
        "an exited PTY became busy solely because its state was locked"
    );
    drop(state);
    service.end_session("room-1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_terminal_state_contention_remains_managed_work() {
    let root = TempDir::new();
    let service = service(root.path());
    let terminal = service
        .open(OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: "room-1".into(),
            working_directory: root.path().into(),
            cols: 80,
            rows: 24,
            title: None,
            opened_by: TerminalActor::User,
            command: Some(vec!["/bin/cat".into()]),
        })
        .unwrap();
    let owner = service.terminal_for_test(&terminal.terminal_id);
    let state = owner.lock();
    assert!(state.terminal.is_running());
    assert!(
        service.has_running_work(),
        "a busy live owner must still fence managed Stop"
    );
    drop(state);
    service.end_session("room-1");
}
