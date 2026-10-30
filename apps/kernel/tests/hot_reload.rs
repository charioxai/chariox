//! EA2-86: a planned in-place upgrade (hot reload, SIGUSR2) keeps a running
//! terminal command alive, and its output continues at the same offsets.
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
    // The drill may point at another build, e.g. the base binary for the red run.
    let binary = std::env::var_os("CHARIOX_HOT_RELOAD_KERNEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_chariox-kernel")));
    std::fs::create_dir_all(scratch.join("project")).unwrap();
    std::fs::create_dir_all(scratch.join("bin")).unwrap();
    // Run an installed copy, so the upgrade can replace what its path names.
    let installed = scratch.join("bin/chariox-kernel");
    std::fs::copy(&binary, &installed).unwrap();
    let port = free_port();
    let token = format!("{:032x}", rand::random::<u128>());
    let log = std::fs::File::create(scratch.join("kernel.log")).unwrap();
    let child = Command::new(&installed)
        .env_clear()
        .env("HOME", scratch.join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .env("CHARIOX_HOME", scratch.join("home/.chariox"))
        .env("CHARIOX_LOG_DIR", scratch.join("logs"))
        .env("CHARIOX_KERNEL_HOST", "127.0.0.1")
        .env("CHARIOX_KERNEL_PORT", port.to_string())
        .env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", &token)
        .env("CHARIOX_MCP_PORT", free_port().to_string())
        .env("CHARIOX_DAEMON_SOCKET", scratch.join("k.sock"))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
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
        // Prints for about eight seconds, so lines are produced before, during
        // and after the swap.
        let script =
            "i=0; while [ $i -lt 80 ]; do echo tick-$i; i=$((i+1)); sleep 0.1; done; echo done";
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
        let staged = scratch.join("bin/chariox-kernel.new");
        std::fs::copy(&installed, &staged).unwrap();
        std::fs::rename(&staged, &installed).unwrap();
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
        let staged = scratch.join("bin/chariox-kernel.new");
        std::fs::write(&staged, "#!/bin/sh\nexit 2\n").unwrap();
        std::fs::set_permissions(&staged, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        std::fs::rename(&staged, &installed).unwrap();
        let pid = kernel.child.id() as i32;
        assert!(pid > 1);
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        let waited = terminal(
            &mut socket,
            &session_id,
            json!({"operation": "wait", "terminal_id": terminal_id,
                "watches": [{"kind": "output_contains", "text": "still-here"}],
                "timeout_ms": 20000}),
        )
        .await;
        // The kernel kept running and its connection open: the command finished here.
        assert_eq!(
            waited["waited"]["fired"]["kind"], "output_matched",
            "{waited}"
        );
    });
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
    while Path::new(&format!("/proc/{exec_pid}")).exists() {
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
