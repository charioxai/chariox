use std::env;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arroba_daemon::attachment::ClientCapabilityLevel;
use arroba_daemon::local::{
    AttachToSessionRequest, CancelActivePromptRequest, DetachFromSessionRequest,
    GetSessionStateRequest, LaunchProviderRunRequest, ListSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse, LocalIpcClient, PumpTerminalOutputRequest, ResizeTerminalRequest,
    SubmitPromptRequest,
};
use arroba_daemon::session::{CreateSessionRequest, RuntimeSession, SessionStatus};
use arroba_daemon::{DaemonConfig, DaemonError};

fn main() -> Result<(), DaemonError> {
    let options = CliOptions::parse(env::args().skip(1))?;
    let config = DaemonConfig::load_from_env();
    let socket_path = options
        .socket_path
        .clone()
        .unwrap_or_else(|| config.local_socket_path.clone());
    let client = LocalIpcClient::new(socket_path);
    let workspace = options.workspace.unwrap_or_else(default_working_directory);
    let worktree = options.worktree.unwrap_or_else(|| workspace.clone());

    let (session_id, created_session) = if let Some(session_id) = options.session_id.clone() {
        (session_id, false)
    } else if let Some(session) = find_attachable_session(&client, &workspace, &worktree)? {
        (session.id().to_string(), false)
    } else {
        let session = match client.send(&LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace.display().to_string(),
                worktree.display().to_string(),
            ),
        ))? {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            other => unexpected_response("create session", &other),
        };
        (session.id().to_string(), true)
    };

    let attachment = match client.send(&LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: options.client_id.clone(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        },
    ))? {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => unexpected_response("attach to session", &other),
    };

    let session_state = get_session_state(&client, &session_id)?;
    if session_state.active_provider_run_id().is_none() {
        let _ = client.send(&LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: None,
                adapter_key: "opencode".to_string(),
                provider: "opencode".to_string(),
                account_profile: options.account_profile.clone(),
                model: options.model.clone(),
                variant: None,
            },
        ))?;
    }

    if let Some((cols, rows)) = current_terminal_size() {
        let _ = client.send(&LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols,
            rows,
        }));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let output_activity = Arc::new(Mutex::new(OutputActivity::default()));
    let output_thread = spawn_output_poller(
        client.clone(),
        session_id.clone(),
        attachment.id().to_string(),
        stop.clone(),
        output_activity.clone(),
    );
    let resize_thread = spawn_resize_poller(client.clone(), session_id.clone(), stop.clone());

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();
    let mut should_drain_after_eof = false;

    while !stop.load(Ordering::Relaxed) {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "read cli input",
                message: error.to_string(),
            })?;
        if bytes == 0 {
            should_drain_after_eof = lock_output_activity(&output_activity)
                .last_prompt_at
                .is_some();
            break;
        }
        if line.trim() == "/exit" {
            break;
        }
        if line.trim() == "/stop" {
            let _ = client.send(&LocalDaemonRequest::CancelActivePrompt(
                CancelActivePromptRequest {
                    session_id: session_id.clone(),
                    attachment_id: attachment.id().to_string(),
                },
            ))?;
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }

        let _ = client.send(&LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            prompt: line.clone(),
            attachments: Vec::new(),
        }))?;
        lock_output_activity(&output_activity).note_prompt_submission();
    }

    if should_drain_after_eof {
        wait_for_provider_drain(&stop, &output_activity);
    }
    stop.store(true, Ordering::Relaxed);
    let _ = output_thread.join();
    let _ = resize_thread.join();

    let _ = created_session;
    let _ = client.send(&LocalDaemonRequest::DetachFromSession(
        DetachFromSessionRequest {
            attachment_id: attachment.id().to_string(),
        },
    ));

    Ok(())
}

#[derive(Debug, Clone)]
struct CliOptions {
    socket_path: Option<PathBuf>,
    session_id: Option<String>,
    client_id: String,
    model: String,
    account_profile: String,
    workspace: Option<PathBuf>,
    worktree: Option<PathBuf>,
}

impl CliOptions {
    fn parse<I>(args: I) -> Result<Self, DaemonError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut options = Self {
            socket_path: None,
            session_id: None,
            client_id: format!("arroba-cli-{}", std::process::id()),
            model: "default".to_string(),
            account_profile: "default".to_string(),
            workspace: None,
            worktree: None,
        };

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => options.socket_path = Some(PathBuf::from(next_arg(&mut args, &arg)?)),
                "--session" => options.session_id = Some(next_arg(&mut args, &arg)?),
                "--client-id" => options.client_id = next_arg(&mut args, &arg)?,
                "--model" => options.model = next_arg(&mut args, &arg)?,
                "--account-profile" => options.account_profile = next_arg(&mut args, &arg)?,
                "--workspace" => {
                    options.workspace = Some(PathBuf::from(next_arg(&mut args, &arg)?))
                }
                "--worktree" => options.worktree = Some(PathBuf::from(next_arg(&mut args, &arg)?)),
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                _ => {
                    return Err(DaemonError::LocalTransport {
                        operation: "parse cli arguments",
                        message: format!("unknown argument `{arg}`"),
                    });
                }
            }
        }

        Ok(options)
    }
}

fn next_arg<I>(args: &mut I, flag: &str) -> Result<String, DaemonError>
where
    I: Iterator<Item = String>,
{
    args.next().ok_or_else(|| DaemonError::LocalTransport {
        operation: "parse cli arguments",
        message: format!("missing value for `{flag}`"),
    })
}

fn print_usage() {
    println!(
        "usage: arroba-cli [--socket PATH] [--session ID] [--client-id ID] [--model MODEL] [--account-profile PROFILE] [--workspace PATH] [--worktree PATH]\n\ncommands:\n  /stop   request cancellation of the active provider turn\n  /exit   exit the CLI"
    );
}

fn spawn_output_poller(
    client: LocalIpcClient,
    session_id: String,
    attachment_id: String,
    stop: Arc<AtomicBool>,
    output_activity: Arc<Mutex<OutputActivity>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let stdout = io::stdout();
        let mut lock = stdout.lock();

        while !stop.load(Ordering::Relaxed) {
            match client.send(&LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                },
            )) {
                Ok(LocalDaemonResponse::TerminalOutput { records }) => {
                    for record in records {
                        lock_output_activity(&output_activity).note_output();
                        if lock.write_all(&record.bytes).is_err() {
                            stop.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                    let _ = lock.flush();
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = writeln!(io::stderr(), "output polling failed: {error}");
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
            }

            thread::sleep(Duration::from_millis(50));
        }
    })
}

fn spawn_resize_poller(
    client: LocalIpcClient,
    session_id: String,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_size = current_terminal_size();

        while !stop.load(Ordering::Relaxed) {
            let current_size = current_terminal_size();
            if let Some((cols, rows)) = current_size {
                if current_size != last_size {
                    let _ =
                        client.send(&LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
                            session_id: session_id.clone(),
                            cols,
                            rows,
                        }));
                    last_size = current_size;
                }
            }

            thread::sleep(Duration::from_millis(250));
        }
    })
}

fn get_session_state(
    client: &LocalIpcClient,
    session_id: &str,
) -> Result<arroba_daemon::session::RuntimeSession, DaemonError> {
    match client.send(&LocalDaemonRequest::GetSessionState(
        GetSessionStateRequest {
            session_id: session_id.to_string(),
        },
    ))? {
        LocalDaemonResponse::SessionState { session } => Ok(session),
        other => unexpected_response("get session state", &other),
    }
}

fn find_attachable_session(
    client: &LocalIpcClient,
    workspace: &std::path::Path,
    worktree: &std::path::Path,
) -> Result<Option<RuntimeSession>, DaemonError> {
    let sessions = match client.send(&LocalDaemonRequest::ListSessions(ListSessionsRequest))? {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        other => unexpected_response("list sessions", &other),
    };

    let workspace = workspace.display().to_string();
    let worktree = worktree.display().to_string();

    Ok(sessions
        .into_iter()
        .filter(|session| {
            session.workspace_id() == workspace
                && session.worktree_id() == worktree
                && session.status() != SessionStatus::Ended
        })
        .max_by_key(session_sort_key))
}

fn session_sort_key(session: &RuntimeSession) -> u64 {
    session
        .id()
        .strip_prefix("session-")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn default_working_directory() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn unexpected_response<T>(operation: &str, response: &LocalDaemonResponse) -> T {
    panic!("unexpected response for {operation}: {response:?}")
}

fn current_terminal_size() -> Option<(u16, u16)> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };

    if rc == 0 && size.ws_col > 0 && size.ws_row > 0 {
        Some((size.ws_col, size.ws_row))
    } else {
        None
    }
}

#[derive(Debug, Default)]
struct OutputActivity {
    last_prompt_at: Option<Instant>,
    last_output_at: Option<Instant>,
    output_count: u64,
}

impl OutputActivity {
    fn note_prompt_submission(&mut self) {
        self.last_prompt_at = Some(Instant::now());
    }

    fn note_output(&mut self) {
        self.last_output_at = Some(Instant::now());
        self.output_count += 1;
    }
}

fn wait_for_provider_drain(stop: &Arc<AtomicBool>, output_activity: &Arc<Mutex<OutputActivity>>) {
    let first_response_grace = Duration::from_millis(
        env::var("ARROBA_CLI_EOF_FIRST_RESPONSE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30_000),
    );
    let idle_window = Duration::from_millis(
        env::var("ARROBA_CLI_EOF_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(2_000),
    );
    let eof_started_at = Instant::now();
    let starting_output_count = lock_output_activity(output_activity).output_count;

    while !stop.load(Ordering::Relaxed) {
        let state = lock_output_activity(output_activity);
        let saw_new_output = state.output_count > starting_output_count;
        let last_output_at = state.last_output_at;
        drop(state);

        if saw_new_output {
            if let Some(last_output_at) = last_output_at {
                if last_output_at.elapsed() >= idle_window {
                    break;
                }
            }
        } else if eof_started_at.elapsed() >= first_response_grace {
            break;
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn lock_output_activity<'a>(
    output_activity: &'a Arc<Mutex<OutputActivity>>,
) -> std::sync::MutexGuard<'a, OutputActivity> {
    output_activity
        .lock()
        .expect("output activity mutex should not be poisoned")
}
