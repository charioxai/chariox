//! EA2-86: a planned in-place upgrade (hot reload, SIGUSR2) keeps a running
//! terminal command alive, and its output continues at the same offsets; a
//! running agent turn continues in the same provider process.
#![cfg(unix)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Tests run in parallel threads: a program file being written must not be
/// open in a child another thread forks meanwhile, or exec fails with ETXTBSY.
static PROGRAM_FILES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Installs `program` at `installed` the way an upgrade does: write aside, rename over.
fn install(write: impl FnOnce(&Path), installed: &Path) {
    let _files = PROGRAM_FILES.lock().unwrap_or_else(|e| e.into_inner());
    let staged = installed.with_extension("new");
    write(&staged);
    std::fs::set_permissions(&staged, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, installed).unwrap();
}

struct Kernel {
    child: Child,
    scratch: PathBuf,
    port: u16,
    token: String,
}

impl Drop for Kernel {
    fn drop(&mut self) {
        let pid = self.child.id() as i32;
        if pid > 1 {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::env::var_os("CHARIOX_HOT_RELOAD_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.scratch);
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_kernel(scratch: &Path) -> Kernel {
    start_kernel_with(scratch, &[])
}

fn start_kernel_with(scratch: &Path, env: &[(&str, PathBuf)]) -> Kernel {
    start_kernel_inheriting(scratch, env, &[])
}

/// Starts the kernel with `inherited` descriptors open in it (and only in it),
/// as a supervisor hands them over.
fn start_kernel_inheriting(scratch: &Path, env: &[(&str, PathBuf)], inherited: &[i32]) -> Kernel {
    // The drill may point at another build, e.g. the base binary for the red run.
    let binary = std::env::var_os("CHARIOX_HOT_RELOAD_KERNEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_chariox-kernel")));
    std::fs::create_dir_all(scratch.join("project")).unwrap();
    std::fs::create_dir_all(scratch.join("bin")).unwrap();
    // Run an installed copy, so the upgrade can replace what its path names.
    let installed = scratch.join("bin/chariox-kernel");
    install(
        |staged| {
            std::fs::copy(&binary, staged).unwrap();
        },
        &installed,
    );
    let port = free_port();
    let token = format!("{:032x}", rand::random::<u128>());
    let log = std::fs::File::create(scratch.join("kernel.log")).unwrap();
    // A managed kernel reads its token from a file its supervisor wrote.
    let token_file = env
        .iter()
        .find(|(name, _)| *name == "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE");
    if let Some((_, path)) = token_file {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        std::io::Write::write_all(&mut file, token.as_bytes()).unwrap();
    }
    let files = PROGRAM_FILES.lock().unwrap_or_else(|e| e.into_inner());
    let mut command = Command::new(&installed);
    command.env_clear();
    let inherited = inherited.to_vec();
    // SAFETY: fcntl is async-signal-safe; it runs in the child between fork and exec.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, move || {
            for fd in &inherited {
                if libc::fcntl(*fd, libc::F_SETFD, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    if token_file.is_none() {
        command.env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", &token);
    }
    let child = command
        .env("HOME", scratch.join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .env("CHARIOX_HOME", scratch.join("home/.chariox"))
        .env("CHARIOX_LOG_DIR", scratch.join("logs"))
        .env("CHARIOX_KERNEL_HOST", "127.0.0.1")
        .env("CHARIOX_KERNEL_PORT", port.to_string())
        .env("CHARIOX_MCP_PORT", free_port().to_string())
        .env("CHARIOX_DAEMON_SOCKET", scratch.join("k.sock"))
        .envs(env.iter().map(|(name, value)| (*name, value)))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    drop(files);
    Kernel {
        child,
        scratch: scratch.to_path_buf(),
        port,
        token,
    }
}

async fn connect(kernel: &Kernel, within: Duration) -> Socket {
    let deadline = Instant::now() + within;
    loop {
        let mut request = format!("ws://127.0.0.1:{}/kernel", kernel.port)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", kernel.token).parse().unwrap(),
        );
        if let Ok((socket, _)) = connect_async(request).await {
            return socket;
        }
        assert!(
            Instant::now() < deadline,
            "the kernel did not accept a connection"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn request(socket: &mut Socket, body: Value) -> Value {
    let id = format!("hot-reload-{:016x}", rand::random::<u64>());
    let frame = json!({"type": "request", "request_id": id, "command_id": id, "request": body});
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "response" && frame["request_id"] == id.as_str() {
                        assert!(frame["error"].is_null(), "{frame}");
                        return frame["response"].clone();
                    }
                }
                Some(Ok(Message::Ping(data))) => socket.send(Message::Pong(data)).await.unwrap(),
                Some(Ok(_)) => {}
                other => panic!("the kernel connection ended: {other:?}"),
            }
        }
    })
    .await
    .expect("the kernel answered in time")
}

async fn closed_by_the_old_program(mut socket: Socket) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(Message::Ping(data))) => {
                    let _ = socket.send(Message::Pong(data)).await;
                }
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("the old kernel program ended its connections");
}

async fn terminal(socket: &mut Socket, session_id: &str, operation: Value) -> Value {
    let response = request(
        socket,
        json!({"RoomTerminal": {"session_id": session_id, "operation": operation}}),
    )
    .await;
    response["RoomTerminal"]["result"].clone()
}

#[test]
fn a_hot_reload_keeps_a_running_terminal_command_and_its_output() {
    let scratch = std::env::temp_dir().join(format!("hr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let mut kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        // Prints before, during and after the swap: it waits after tick-9
        // until the new program runs, however long the swap takes.
        let go = scratch.join("go");
        let script = format!(
            "i=0; while [ $i -lt 80 ]; do echo tick-$i; i=$((i+1)); sleep 0.1; \
             if [ $i -eq 10 ]; then while [ ! -e {} ]; do sleep 0.1; done; fi; done; echo done",
            go.display()
        );
        let opened = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "open", "cols": 80, "rows": 24,
                "command": ["/bin/sh", "-c", script]}),
        )
        .await;
        let terminal_id = opened["terminal"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_string();
        let mut output = Vec::new();
        let mut offset = 0;
        while !String::from_utf8_lossy(&output).contains("tick-5") {
            let page = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "read_output", "terminal_id": terminal_id,
                    "after_offset": offset, "wait_ms": 500}),
            )
            .await;
            output.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(page["output"]["data_base64"].as_str().unwrap())
                    .unwrap(),
            );
            offset = page["output"]["next_offset"].as_u64().unwrap();
        }

        // The upgrade installs the new program over the old one, then asks
        // the running kernel to reload.
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        // The old program's connections end with it; a refused reload would
        // keep this one open.
        closed_by_the_old_program(socket).await;
        #[cfg(target_os = "linux")]
        assert_eq!(
            std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
            installed,
            "the kernel runs the newly installed program"
        );
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        std::fs::write(&go, "").unwrap();

        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            let page = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "read_output", "terminal_id": terminal_id,
                    "after_offset": offset, "wait_ms": 500}),
            )
            .await;
            assert_eq!(page["output"]["gap"], false, "{page}");
            output.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(page["output"]["data_base64"].as_str().unwrap())
                    .unwrap(),
            );
            offset = page["output"]["next_offset"].as_u64().unwrap();
            let status = page["output"]["terminal"]["status"].clone();
            if status["state"] == "ended" && page["output"]["data_base64"] == "" {
                break status;
            }
            assert!(Instant::now() < deadline, "the command did not finish");
        };
        assert_eq!(
            status["outcome"],
            json!({"reason": "program_exited", "exit_code": 0, "signal": null}),
            "{status}"
        );
        let text = String::from_utf8_lossy(&output);
        for tick in 0..80 {
            assert_eq!(
                text.matches(&format!("tick-{tick}\r\n")).count(),
                1,
                "line tick-{tick} is missing or repeated: {text}"
            );
        }
        assert!(text.contains("done"), "{text}");
    });
    assert!(
        kernel.child.try_wait().unwrap().is_none(),
        "the kernel process kept its PID through the reload"
    );
}

#[test]
fn a_reload_to_a_program_that_cannot_take_the_handover_changes_nothing() {
    let scratch = std::env::temp_dir().join(format!("hr-refused-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let opened = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "open", "cols": 80, "rows": 24,
                "command": ["/bin/sh", "-c", "sleep 3; echo still-here"]}),
        )
        .await;
        let terminal_id = opened["terminal"]["terminal_id"].as_str().unwrap();
        // An older program that knows no handover format.
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| std::fs::write(staged, "#!/bin/sh\nexit 2\n").unwrap(),
            &installed,
        );
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        // The kernel kept running and its connection open: the output arrives here.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let page = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "read_output", "terminal_id": terminal_id,
                    "after_offset": 0, "wait_ms": 500}),
            )
            .await;
            let output = base64::engine::general_purpose::STANDARD
                .decode(page["output"]["data_base64"].as_str().unwrap())
                .unwrap();
            if String::from_utf8_lossy(&output).contains("still-here") {
                break;
            }
            assert!(Instant::now() < deadline, "{page}");
        }
    });
}

/// A managed kernel reads its token from a file its supervisor wrote and
/// deleted it; the new program must still accept the same token.
#[test]
fn a_kernel_whose_token_came_from_a_file_keeps_it_across_a_hot_reload() {
    let scratch = std::env::temp_dir().join(format!("hr-token-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let token_file = scratch.join("kernel.token");
    let kernel = start_kernel_with(
        &scratch,
        &[("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", token_file.clone())],
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let socket = connect(&kernel, Duration::from_secs(60)).await;
        assert!(!token_file.exists(), "the kernel consumes its token file");
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        connect(&kernel, Duration::from_secs(60)).await;
    });
    assert!(!token_file.exists(), "no token is left on disk");
    #[cfg(target_os = "linux")]
    assert_eq!(
        std::fs::read_link(format!("/proc/{}/exe", kernel.child.id())).unwrap(),
        scratch.join("bin/chariox-kernel")
    );
}

/// A client resumes its subscription on the attachment it had: a browser whose
/// attachment is gone waits for its Room to retry (seconds) before it shows the
/// output again.
#[test]
fn a_client_keeps_its_attachment_across_a_hot_reload() {
    let scratch = std::env::temp_dir().join(format!("hr-attach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let attached = request(
            &mut socket,
            json!({"AttachToSession": {"session_id": session_id,
                "client_id": "hot-reload-test", "capability_level": "FullTerminal"}}),
        )
        .await;
        let attachment_id = attached["SessionAttached"]["attachment"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{attached}"))
            .to_string();
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        // `request` fails on an error answer such as `attachment_not_found`.
        let pumped = request(
            &mut socket,
            json!({"PumpTerminalOutput": {"session_id": session_id, "attachment_id": attachment_id}}),
        )
        .await;
        assert!(pumped["TerminalOutput"].is_object(), "{pumped}");
    });
}

/// A local browser picks a kernel only while its presence record says it is
/// on the relay, so the new program must say so as soon as it is back on the
/// relay, not at its next presence heartbeat (10 s later in the web drill).
#[test]
fn a_reloaded_kernel_publishes_that_it_is_back_on_the_relay() {
    let scratch = std::env::temp_dir().join(format!("hr-presence-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    // A relay that accepts the kernel and reads what it sends.
    let relay = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let relay_url = format!("ws://{}", relay.local_addr().unwrap());
    runtime.spawn(async move {
        while let Ok((stream, _)) = relay.accept().await {
            tokio::spawn(async move {
                if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                    while let Some(Ok(_)) = socket.next().await {}
                }
            });
        }
    });
    let kernel = start_kernel_with(
        &scratch,
        &[
            ("CHARIOX_RELAY_URL", PathBuf::from(&relay_url)),
            ("CHARIOX_RELAY_TOKEN", PathBuf::from("hot-reload-presence")),
            ("CHARIOX_DAEMON_ID", PathBuf::from("hr-presence")),
        ],
    );
    let record_path = scratch.join("home/.chariox/kernels/active/hr-presence.json");
    let on_relay_since = |started_after_ms: u64, within: Duration| {
        let deadline = Instant::now() + within;
        loop {
            let record = std::fs::read(&record_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if record.as_ref().is_some_and(|record| {
                record["relay_connected"] == true
                    && record["started_at_ms"].as_u64() > Some(started_after_ms)
            }) {
                return epoch_ms();
            }
            assert!(
                Instant::now() < deadline,
                "the presence record never said the kernel is on the relay: {record:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    on_relay_since(0, Duration::from_secs(60));
    let installed = scratch.join("bin/chariox-kernel");
    install(
        |staged| {
            std::fs::copy(&installed, staged).unwrap();
        },
        &installed,
    );
    let reload_at = epoch_ms();
    let pid = kernel.child.id() as i32;
    assert!(pid > 1);
    unsafe { libc::kill(pid, libc::SIGUSR2) };
    let published_at = on_relay_since(reload_at, Duration::from_secs(30));
    let registered_at = logged_after(&scratch, "relay register sent", reload_at);
    assert!(
        published_at <= registered_at + 2_000,
        "the new program was back on the relay {} ms after the reload, but its presence record said so only {} ms after the reload",
        registered_at - reload_at,
        published_at - reload_at
    );
}

/// A managed kernel reaches its slice Docker broker over a connection its
/// supervisor handed it (`CHARIOX_SLICE_DOCKER_BROKER_FD`, with no socket path
/// to reconnect to): the new program keeps that connection.
#[test]
fn a_hot_reload_keeps_the_managed_slice_broker_connection() {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let scratch = std::env::temp_dir().join(format!("hr-broker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let (kernel_end, mut broker) = std::os::unix::net::UnixStream::pair().unwrap();
    let fd = kernel_end.as_raw_fd();
    let kernel = start_kernel_inheriting(
        &scratch,
        &[("CHARIOX_SLICE_DOCKER_BROKER_FD", fd.to_string().into())],
        &[fd],
    );
    #[cfg(target_os = "linux")]
    let socket_name = std::fs::read_link(format!("/proc/self/fd/{fd}")).unwrap();
    // Only the kernel holds its end now: the broker reads EOF once it closes.
    drop(kernel_end);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let socket = connect(&kernel, Duration::from_secs(60)).await;
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        connect(&kernel, Duration::from_secs(60)).await;
    });
    // The new program holds the connection: no EOF, and requests still reach
    // the broker (it sends none by itself).
    broker.set_nonblocking(true).unwrap();
    let mut byte = [0; 1];
    match broker.read(&mut byte) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("the broker connection ended at the hot reload: {other:?}"),
    }
    #[cfg(target_os = "linux")]
    {
        let held = std::fs::read_link(format!("/proc/{}/fd/{fd}", kernel.child.id()));
        assert_eq!(
            held.ok(),
            Some(socket_name),
            "the new program holds the connection"
        );
    }
}

#[test]
fn work_that_is_not_handed_over_stops_as_for_a_restart() {
    let scratch = std::env::temp_dir().join(format!("hr-exec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let pid_file = scratch.join("exec.pid");
    let exec_pid = runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        // A Room exec has no PTY to hand over.
        let script = format!("echo $$ > {}; exec sleep 120", pid_file.display());
        terminal(
            &mut socket,
            &session_id,
            json!({"operation": "exec_open", "command": ["/bin/sh", "-c", script]}),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(20);
        let exec_pid = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "the exec did not start");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        connect(&kernel, Duration::from_secs(60)).await;
        exec_pid
    });
    // Not left running unmanaged: ended (and reaped) as on a planned restart.
    let deadline = Instant::now() + Duration::from_secs(20);
    // Signal 0 only checks that the process still exists; ended and reaped
    // it does not.
    assert!(exec_pid > 1);
    while unsafe { libc::kill(exec_pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "the exec kept running after the hot reload"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_terminal_takes_input_after_a_hot_reload() {
    let scratch = std::env::temp_dir().join(format!("hr-input-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let opened = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "open", "cols": 80, "rows": 24, "command": ["/bin/sh"]}),
        )
        .await;
        let terminal_id = opened["terminal"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_string();
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let before = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "read_output", "terminal_id": terminal_id,
                "after_offset": 0, "wait_ms": 0}),
        )
        .await["output"]["terminal"]["output_offset"]
            .as_u64()
            .unwrap();
        let typed = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "input", "terminal_id": terminal_id,
                "data_base64": base64::engine::general_purpose::STANDARD
                    .encode("echo typed-$((40+2))\n")}),
        )
        .await;
        assert_eq!(typed["input"]["accepted"], true, "{typed}");
        // Only the shell's own answer prints 42; the echoed keys do not.
        let waited = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "wait", "terminal_id": terminal_id,
                "watches": [{"kind": "output_contains", "text": "typed-42"}],
                "after_offset": before, "timeout_ms": 20000}),
        )
        .await;
        assert_eq!(
            waited["waited"]["fired"]["kind"], "output_matched",
            "{waited}"
        );
    });
}

/// Memory domains are cgroups, which only Linux has.
#[cfg(target_os = "linux")]
#[test]
fn an_adopted_terminal_keeps_its_memory_domain() {
    let scratch = std::env::temp_dir().join(format!("hr-domain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let joined = scratch.join("cgroup");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let domain = runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let script = format!("cat /proc/self/cgroup > {}; sleep 3", joined.display());
        terminal(
            &mut socket,
            &session_id,
            json!({"operation": "open", "cols": 80, "rows": 24,
                "command": ["/bin/sh", "-c", script]}),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(20);
        let domain = loop {
            if let Some(path) = std::fs::read_to_string(&joined).ok().and_then(|text| {
                text.lines()
                    .find_map(|l| l.strip_prefix("0::"))
                    .map(str::to_string)
            }) {
                break Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
            }
            assert!(Instant::now() < deadline, "the command did not start");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        connect(&kernel, Duration::from_secs(60)).await;
        domain
    });
    // Only where the kernel can create memory domains (cgroup v2 delegation).
    if !domain
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("process-"))
    {
        return;
    }
    // The new program owns the domain again: it ends with the process.
    let deadline = Instant::now() + Duration::from_secs(20);
    while domain.exists() {
        assert!(
            Instant::now() < deadline,
            "the adopted process's memory domain was left behind: {}",
            domain.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn connections_during_a_hot_reload_are_not_refused() {
    let scratch = std::env::temp_dir().join(format!("hr-listen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel(&scratch);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let socket = connect(&kernel, Duration::from_secs(60)).await;
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        // Connect without pause through the swap (it takes well under 4 s).
        let swap = tokio::spawn(closed_by_the_old_program(socket));
        let mut refused = 0;
        let mut attempts = 0;
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            attempts += 1;
            if let Err(error) = TcpStream::connect(("127.0.0.1", kernel.port)).await {
                assert_eq!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused,
                    "{error}"
                );
                refused += 1;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        swap.await.unwrap();
        connect(&kernel, Duration::from_secs(60)).await;
        assert_eq!(
            refused, 0,
            "{refused} of {attempts} connections were refused"
        );
    });
}

/// Claude Code as the kernel drives it (stream-json on stdin/stdout): each
/// prompt is a turn that streams 400 parts over 4 s, waiting halfway until
/// the test has upgraded the kernel. It records every start
/// and every prompt it receives.
const FAKE_CLAUDE: &str = r#"#!/bin/bash
if [[ "$1" == "--version" ]]; then printf 'Claude Code 2.1.207\n'; exit 0; fi
if [[ "$1" == "auth" && "$2" == "status" ]]; then
    printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"fixture@example.test","subscriptionType":"pro"}'
    exit 0
fi
if [[ "$*" == *CHARIOX_AUTH_OK* ]]; then
    printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"CHARIOX_AUTH_OK","usage":{"output_tokens":3}}'
    exit 0
fi
[[ " $* " == *" stream-json "* ]] || exit 0
record="$CHARIOX_FAKE_CLAUDE_RECORD"
echo "start $$ token=${CLAUDE_CODE_OAUTH_TOKEN:+set}" >> "$record/starts"
session=11111111-2222-4333-8444-555555555555
turn=0
while IFS= read -r line; do
    [[ "$line" == *'"type":"user"'* ]] || continue
    turn=$((turn + 1))
    echo "prompt $turn" >> "$record/prompts"
    printf '%s\n' "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"$session\",\"model\":\"claude-haiku-5-5\"}"
    # One message whose text block streams as deltas, as with --include-partial-messages.
    printf '%s\n' "{\"type\":\"stream_event\",\"session_id\":\"$session\",\"event\":{\"type\":\"message_start\",\"message\":{\"id\":\"msg-$turn\",\"role\":\"assistant\",\"model\":\"claude-haiku-5-5\"}}}"
    printf '%s\n' "{\"type\":\"stream_event\",\"session_id\":\"$session\",\"event\":{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}}"
    text=""
    for part in $(seq 1 400); do
        printf '%s\n' "{\"type\":\"stream_event\",\"session_id\":\"$session\",\"event\":{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"turn-$turn-part-$part \"}}}"
        text="${text}turn-$turn-part-$part "
        echo "part $part $EPOCHREALTIME" >> "$record/parts"
        # Halfway, wait for the upgrade: the turn is running when it happens.
        if [[ $part == 200 ]]; then
            for _ in $(seq 1 600); do [[ -e "$record/go" ]] && break; sleep 0.1; done
        fi
        sleep 0.01
    done
    printf '%s\n' "{\"type\":\"stream_event\",\"session_id\":\"$session\",\"event\":{\"type\":\"content_block_stop\",\"index\":0}}"
    printf '%s\n' "{\"type\":\"assistant\",\"session_id\":\"$session\",\"message\":{\"id\":\"msg-$turn\",\"role\":\"assistant\",\"model\":\"claude-haiku-5-5\",\"content\":[{\"type\":\"text\",\"text\":\"$text\"}]}}"
    printf '%s\n' "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"session_id\":\"$session\",\"result\":\"turn-$turn-done\"}"
    echo "result $turn" >> "$record/parts"
done
"#;

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn a_hot_reload_keeps_a_running_claude_turn() {
    claude_turn_across_a_hot_reload(Login::Native);
}

/// The new program's Vault may be locked or, here, empty: the turn keeps the
/// login it runs with.
#[test]
fn a_hot_reload_keeps_a_claude_turn_whose_login_is_in_the_vault() {
    claude_turn_across_a_hot_reload(Login::Vault);
}

/// Launch credentials larger than a pipe holds still reach the next program.
#[test]
fn a_hot_reload_keeps_a_claude_turn_with_launch_credentials_larger_than_a_pipe() {
    claude_turn_across_a_hot_reload(Login::LargeVault);
}

#[derive(Debug, PartialEq)]
enum Login {
    Native,
    Vault,
    LargeVault,
}

impl Login {
    fn vault_token(&self) -> Option<String> {
        match self {
            Login::Native => None,
            Login::Vault => Some("fixture-token".into()),
            // More than a pipe holds (64 KiB), less than one exec string (128 KiB).
            Login::LargeVault => Some("t".repeat(100 << 10)),
        }
    }
}

fn claude_turn_across_a_hot_reload(login: Login) {
    let scratch = std::env::temp_dir().join(format!("hr-claude-{}-{login:?}", std::process::id(),));
    let _ = std::fs::remove_dir_all(&scratch);
    let record = scratch.join("record");
    std::fs::create_dir_all(&record).unwrap();
    let claude = scratch.join("claude");
    std::fs::write(&claude, FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::create_dir_all(scratch.join("claude-config")).unwrap();
    let mut env = vec![
        ("CHARIOX_CLAUDE_BIN", claude.clone()),
        ("CHARIOX_FAKE_CLAUDE_RECORD", record.clone()),
        ("CLAUDE_CONFIG_DIR", scratch.join("claude-config")),
        ("XDG_CONFIG_HOME", scratch.join("config")),
        ("XDG_STATE_HOME", scratch.join("state")),
        ("XDG_CACHE_HOME", scratch.join("cache")),
    ];
    match login {
        // A signed-in default account, as the kernel checks it before a launch.
        Login::Native => std::fs::write(
            scratch.join("claude-config/.credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"fixture","refreshToken":"fixture"}}"#,
        )
        .unwrap(),
        // A Vault that lives in the kernel's memory only.
        Login::Vault | Login::LargeVault => {
            std::fs::create_dir_all(scratch.join("home/.chariox")).unwrap();
            std::fs::write(
                scratch.join("home/.chariox/config.toml"),
                "[credential_vault]\nbackend = \"process_memory\"\n",
            )
            .unwrap();
            env.push(("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1".into()));
        }
    }
    let kernel = start_kernel_with(&scratch, &env);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        if let Some(token) = login.vault_token() {
            request(
                &mut socket,
                json!({"SetProviderAccountCredential": {"provider": "claude",
                    "account_profile": "default", "value": token,
                    "run": false, "overwrite": true}}),
            )
            .await;
        }
        let spawned = request(
            &mut socket,
            json!({"SpawnAgent": {"session_id": session_id, "alias": "writer",
                "provider": "claude", "model": "haiku", "effort": null,
                "worktree_id": null, "no_subscribe": true}}),
        )
        .await;
        let agent_id = spawned["AgentSpawned"]["agent"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{spawned}"))
            .to_string();
        let attached = request(
            &mut socket,
            json!({"AttachToSession": {"session_id": session_id,
                "client_id": "hot-reload-test", "capability_level": "FullTerminal"}}),
        )
        .await;
        let attachment_id = attached["SessionAttached"]["attachment"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{attached}"))
            .to_string();
        request(
            &mut socket,
            json!({"SubmitPrompt": {"session_id": session_id, "attachment_id": attachment_id,
                "target_agent_id": agent_id, "prompt": "write sixteen parts", "attachments": []}}),
        )
        .await;

        // Upgrade while the turn streams.
        let deadline = Instant::now() + Duration::from_secs(60);
        while lines(&record.join("parts")).len() < 40 {
            assert!(
                Instant::now() < deadline,
                "the turn did not start streaming"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;

        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        std::fs::write(record.join("go"), "").unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let state = request(
                &mut socket,
                json!({"GetSessionState": {"session_id": session_id}}),
            )
            .await;
            let settled = state["SessionState"]["session"]["prompt_states"][&agent_id]
                ["active_prompt"]
                .is_null();
            if settled
                && lines(&record.join("parts")).last().map(String::as_str) == Some("result 1")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the turn did not complete: {state}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let history = request(
            &mut socket,
            json!({"GetSessionHistoryOutline": {"session_id": session_id,
                "agent_ids": [agent_id], "include_prefetched_content": true}}),
        )
        .await
        .to_string();
        std::fs::write(scratch.join("history.json"), &history).unwrap();
        // Every part once: none lost or repeated across the swap.
        for part in 1..=400 {
            assert_eq!(
                history.matches(&format!("turn-1-part-{part} ")).count(),
                1,
                "part {part} is not in history once: {history}"
            );
        }
        assert!(!history.contains("interrupted"), "{history}");
    });
    // One process ran the whole turn, and it got the prompt once.
    assert_eq!(
        lines(&record.join("starts")).len(),
        1,
        "Claude Code was restarted"
    );
    assert_eq!(lines(&record.join("prompts")), ["prompt 1"]);
    if login.vault_token().is_some() {
        assert!(lines(&record.join("starts"))[0].ends_with("token=set"));
    }
    #[cfg(target_os = "linux")]
    assert_eq!(
        std::fs::read_link(format!("/proc/{}/exe", kernel.child.id())).unwrap(),
        scratch.join("bin/chariox-kernel")
    );
}

/// A Codex app-server shaped like the measured one (codex-cli 0.159.3).
const FAKE_CODEX: &str = include_str!("hot_reload/fake_codex.py");

/// The item streaming at the swap keeps streaming: its deltas show up with no
/// stall longer than the swap itself, each once and in order.
#[test]
fn a_codex_item_keeps_streaming_across_a_hot_reload() {
    codex_turn_across_a_hot_reload(false, false);
}

/// Two turns are handed over at once: the Codex item keeps streaming beside a
/// Claude turn that is still running.
#[test]
fn a_codex_turn_continues_beside_a_running_claude_turn() {
    codex_turn_across_a_hot_reload(false, true);
}

/// The turn may end while the kernel program is being replaced.
#[test]
fn a_hot_reload_keeps_a_codex_turn_that_ends_during_the_swap() {
    codex_turn_across_a_hot_reload(true, false);
}

fn codex_turn_across_a_hot_reload(finish_in_gap: bool, beside_claude: bool) {
    let scratch = std::env::temp_dir().join(format!(
        "hr-codex-{}-{finish_in_gap}-{beside_claude}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let record = scratch.join("record");
    std::fs::create_dir_all(&record).unwrap();
    std::fs::write(scratch.join("fake_codex.py"), FAKE_CODEX).unwrap();
    let codex = scratch.join("codex");
    {
        let _files = PROGRAM_FILES.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::write(
            &codex,
            format!(
                "#!/bin/sh\nexec python3 {} \"$@\"\n",
                scratch.join("fake_codex.py").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&codex, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
    std::fs::create_dir_all(scratch.join("codex-home")).unwrap();
    let mut env = vec![
        ("CHARIOX_CODEX_BIN", codex.clone()),
        ("CHARIOX_FAKE_CODEX_RECORD", record.clone()),
        ("CODEX_HOME", scratch.join("codex-home")),
    ];
    if finish_in_gap {
        env.push(("CHARIOX_FAKE_CODEX_FINISH_IN_GAP", "1".into()));
    }
    // A signed-in Claude Code whose turn holds at part 200 until `go`.
    let claude_record = scratch.join("claude-record");
    if beside_claude {
        std::fs::create_dir_all(&claude_record).unwrap();
        std::fs::create_dir_all(scratch.join("claude-config")).unwrap();
        std::fs::write(
            scratch.join("claude-config/.credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"fixture","refreshToken":"fixture"}}"#,
        )
        .unwrap();
        let claude = scratch.join("claude");
        let _files = PROGRAM_FILES.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::write(&claude, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        env.extend([
            ("CHARIOX_CLAUDE_BIN", claude),
            ("CHARIOX_FAKE_CLAUDE_RECORD", claude_record.clone()),
            ("CLAUDE_CONFIG_DIR", scratch.join("claude-config")),
            ("XDG_CONFIG_HOME", scratch.join("config")),
            ("XDG_STATE_HOME", scratch.join("state")),
            ("XDG_CACHE_HOME", scratch.join("cache")),
        ]);
    }
    let kernel = start_kernel_with(&scratch, &env);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        let project = scratch.join("project").display().to_string();
        let created = request(
            &mut socket,
            json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                "alias": null, "slice_ref": null, "no_subscribe": true}}),
        )
        .await;
        let session_id = created["SessionCreated"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let spawned = request(
            &mut socket,
            json!({"SpawnAgent": {"session_id": session_id, "alias": "writer",
                "provider": "codex", "model": "gpt-6-luna", "effort": "low",
                "worktree_id": null, "no_subscribe": true}}),
        )
        .await;
        let agent_id = spawned["AgentSpawned"]["agent"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{spawned}"))
            .to_string();
        let attached = request(
            &mut socket,
            json!({"AttachToSession": {"session_id": session_id,
                "client_id": "hot-reload-test", "capability_level": "FullTerminal"}}),
        )
        .await;
        let attachment_id = attached["SessionAttached"]["attachment"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{attached}"))
            .to_string();
        if beside_claude {
            let spawned = request(
                &mut socket,
                json!({"SpawnAgent": {"session_id": session_id, "alias": "counter",
                    "provider": "claude", "model": "haiku", "effort": null,
                    "worktree_id": null, "no_subscribe": true}}),
            )
            .await;
            let claude_id = spawned["AgentSpawned"]["agent"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{spawned}"))
                .to_string();
            request(
                &mut socket,
                json!({"SubmitPrompt": {"session_id": session_id, "attachment_id": attachment_id,
                    "target_agent_id": claude_id, "prompt": "write sixteen parts", "attachments": []}}),
            )
            .await;
            let deadline = Instant::now() + Duration::from_secs(60);
            while lines(&claude_record.join("parts")).len() < 200 {
                assert!(Instant::now() < deadline, "the Claude turn did not start");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        request(
            &mut socket,
            json!({"SubmitPrompt": {"session_id": session_id, "attachment_id": attachment_id,
                "target_agent_id": agent_id, "prompt": "write four hundred parts", "attachments": []}}),
        )
        .await;

        // Install the new program first: the upgrade then only signals.
        let installed = scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        // Upgrade while the item streams; watch what the session shows of it.
        let started = Instant::now();
        let mut shown = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while lines(&record.join("parts")).len() < 100 {
            assert!(
                Instant::now() < deadline,
                "the turn did not start streaming"
            );
            note_shown_parts(&mut socket, &session_id, &agent_id, started, &mut shown).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        std::fs::write(record.join("swap"), "").unwrap();
        let signalled = epoch_ms();
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;

        let mut socket = connect(&kernel, Duration::from_secs(60)).await;
        // A viewer that attaches after the swap, as a browser does.
        let late = request(
            &mut socket,
            json!({"AttachToSession": {"session_id": session_id,
                "client_id": "late-viewer", "capability_level": "FullTerminal"}}),
        )
        .await;
        let late_id = late["SessionAttached"]["attachment"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{late}"))
            .to_string();
        let mut live_parts = 0;
        let mut live_before_result = 0;
        std::fs::write(record.join("go"), "").unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let pumped = request(
                &mut socket,
                json!({"PumpTerminalOutput": {"session_id": session_id, "attachment_id": late_id}}),
            )
            .await;
            let bytes: Vec<u8> = pumped["TerminalOutput"]["records"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|record| record["bytes"].as_array().cloned().unwrap_or_default())
                .filter_map(|byte| byte.as_u64().map(|byte| byte as u8))
                .collect();
            live_parts += String::from_utf8_lossy(&bytes).matches("turn-1-part-").count();
            if lines(&record.join("parts")).last().map(String::as_str) != Some("result 1") {
                live_before_result = live_parts;
            }
            note_shown_parts(&mut socket, &session_id, &agent_id, started, &mut shown).await;
            let state = request(
                &mut socket,
                json!({"GetSessionState": {"session_id": session_id}}),
            )
            .await;
            let settled = state["SessionState"]["session"]["prompt_states"][&agent_id]
                ["active_prompt"]
                .is_null();
            if settled
                && lines(&record.join("parts")).last().map(String::as_str) == Some("result 1")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the turn did not complete: {state}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !finish_in_gap {
            // The late viewer gets the parts live, not only once the turn ends.
            assert!(
                live_before_result >= 50,
                "a viewer attached after the swap got {live_before_result} parts live ({live_parts} in all)"
            );
            // The swap: from the signal until the new program continues the
            // turn (the old program stops, the new one starts and adopts it).
            let continued = logged_at(&scratch, "continuing a handed-over Codex turn");
            let swap = Duration::from_millis(continued.saturating_sub(signalled));
            // From the last part shown before the swap to the first after it,
            // and between any two later ones, no longer than the swap (plus
            // this test's 100 ms polling and the output pump's cadence).
            let stall = shown
                .windows(2)
                .map(|pair| pair[1].0 - pair[0].0)
                .max()
                .unwrap();
            assert!(
                stall <= swap + Duration::from_secs(1),
                "the item stood still for {stall:?}; the swap took {swap:?}: {shown:?}"
            );
        }
        let history = request(
            &mut socket,
            json!({"GetSessionHistoryOutline": {"session_id": session_id,
                "agent_ids": [agent_id], "include_prefetched_content": true}}),
        )
        .await
        .to_string();
        std::fs::write(scratch.join("history.json"), &history).unwrap();
        // Every part once and in order, including those streamed during the swap.
        let mut previous = 0;
        for part in 1..=450 {
            let needle = format!("turn-1-part-{part} ");
            assert_eq!(
                history.matches(&needle).count(),
                1,
                "part {part} is not in history once: {history}"
            );
            let at = history.find(&needle).unwrap();
            assert!(at > previous, "part {part} is out of order: {history}");
            previous = at;
        }
        assert!(!history.contains("interrupted"), "{history}");
    });
    // One app-server ran the whole turn, and it got the prompt once.
    assert_eq!(lines(&record.join("prompts")), ["prompt 1"]);
    // On one connection: the thread was started once and never resumed.
    let threads = lines(&record.join("threads"));
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert!(threads[0].starts_with("thread/start "), "{threads:?}");
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// When the kernel first logged `message`.
fn logged_at(scratch: &Path, message: &str) -> u64 {
    logged_after(scratch, message, 0)
}

/// When the kernel first logged `message` after `after_ms`.
fn logged_after(scratch: &Path, message: &str, after_ms: u64) -> u64 {
    std::fs::read_dir(scratch.join("logs"))
        .unwrap()
        .flat_map(|file| {
            std::fs::read_to_string(file.unwrap().path())
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|line| line["message"] == message)
                .filter_map(|line| line["timestamp_ms"].as_u64())
                .filter(|at| *at > after_ms)
                .collect::<Vec<_>>()
        })
        .min()
        .unwrap_or_else(|| panic!("the kernel never logged {message:?}"))
}

/// Notes when the session shows more parts of the streaming item.
async fn note_shown_parts(
    socket: &mut Socket,
    session_id: &str,
    agent_id: &str,
    started: Instant,
    shown: &mut Vec<(Duration, usize)>,
) {
    let history = request(
        socket,
        json!({"GetSessionHistoryOutline": {"session_id": session_id,
            "agent_ids": [agent_id], "include_prefetched_content": true}}),
    )
    .await
    .to_string();
    let parts = history.matches("turn-1-part-").count();
    // From the first part on: the stream's start is not part of the swap.
    if parts > 0 && shown.last().is_none_or(|(_, last)| parts > *last) {
        shown.push((started.elapsed(), parts));
    }
}
