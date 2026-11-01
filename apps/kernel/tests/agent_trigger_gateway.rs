//! A kernel that requires its host's token (managed and hosted workers) still
//! starts the local HTTP gateway of an agent trigger: the gateway reads the
//! credential the way its real client does and the kernel admits it.
#![cfg(unix)]

use std::net::TcpListener;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const STUB_CLI: &str = include_str!("agent_trigger_gateway/stub_cli.py");
const FAKE_CODEX: &str = include_str!("hot_reload/fake_codex.py");

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
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)).unwrap();
}

/// A kernel started as a managed worker is: its supervisor leaves the host
/// token in a private file the kernel reads once and removes.
fn start_kernel_requiring_a_host_token(scratch: &Path) -> Kernel {
    for directory in ["project", "bin", "tmp", "codex-home", "record"] {
        std::fs::create_dir_all(scratch.join(directory)).unwrap();
    }
    write_executable(&scratch.join("bin/chariox-cli"), STUB_CLI);
    std::fs::write(scratch.join("fake_codex.py"), FAKE_CODEX).unwrap();
    write_executable(
        &scratch.join("bin/codex"),
        &format!(
            "#!/bin/sh\nexec python3 {} \"$@\"\n",
            scratch.join("fake_codex.py").display()
        ),
    );
    let token = format!("{:032x}", rand::random::<u128>());
    let token_file = scratch.join("kernel.token");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&token_file)
        .unwrap();
    std::io::Write::write_all(&mut file, token.as_bytes()).unwrap();
    let port = free_port();
    let log = std::fs::File::create(scratch.join("kernel.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_chariox-kernel"))
        .env_clear()
        .env("HOME", scratch.join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .env("TMPDIR", scratch.join("tmp"))
        .env("CHARIOX_HOME", scratch.join("home/.chariox"))
        .env("CHARIOX_LOG_DIR", scratch.join("logs"))
        .env("CHARIOX_KERNEL_HOST", "127.0.0.1")
        .env("CHARIOX_KERNEL_PORT", port.to_string())
        .env("CHARIOX_MCP_PORT", free_port().to_string())
        .env("CHARIOX_DAEMON_SOCKET", scratch.join("k.sock"))
        .env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", &token_file)
        .env("CHARIOX_CLI_BIN", scratch.join("bin/chariox-cli"))
        .env("CHARIOX_CODEX_BIN", scratch.join("bin/codex"))
        .env("CHARIOX_FAKE_CODEX_RECORD", scratch.join("record"))
        .env("CODEX_HOME", scratch.join("codex-home"))
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

async fn connect(kernel: &Kernel) -> Socket {
    let deadline = Instant::now() + Duration::from_secs(60);
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

/// The response frame of `body`, or the error frame the kernel answered with.
async fn request(socket: &mut Socket, body: Value) -> Result<Value, Value> {
    let id = format!("trigger-gateway-{:016x}", rand::random::<u64>());
    let frame = json!({"type": "request", "request_id": id, "command_id": id, "request": body});
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "response" && frame["request_id"] == id.as_str() {
                        return if frame["error"].is_null() {
                            Ok(frame["response"].clone())
                        } else {
                            Err(frame["error"].clone())
                        };
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

#[test]
fn a_kernel_that_requires_a_host_token_starts_the_http_trigger_gateway() {
    let scratch = std::env::temp_dir().join(format!("atg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = start_kernel_requiring_a_host_token(&scratch);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut socket = connect(&kernel).await;
            let project = scratch.join("project").display().to_string();
            let created = request(
                &mut socket,
                json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
                    "alias": null, "slice_ref": null, "no_subscribe": true}}),
            )
            .await
            .unwrap();
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
            .await
            .unwrap();
            let agent_id = spawned["AgentSpawned"]["agent"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{spawned}"))
                .to_string();

            let trigger = request(
                &mut socket,
                json!({"CreateAgentTrigger": {"session_id": session_id, "agent_id": agent_id,
                    "source": {"kind": "http"}, "busy_policy": "queue",
                    "prompt": "Decide what to do"}}),
            )
            .await
            .unwrap_or_else(|error| panic!("the gateway did not start: {error}"));
            let trigger_id = trigger["AgentTriggerCreated"]["trigger"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{trigger}"))
                .to_string();

            let invoked = request(
                &mut socket,
                json!({"InvokeAgentTrigger": {"session_id": session_id,
                    "trigger_id": trigger_id, "payload": {"event": "pushed"}}}),
            )
            .await
            .unwrap();
            assert_eq!(
                invoked["AgentTriggerInvoked"]["invocation"]["status"],
                "accepted",
                "{invoked}"
            );
        });
    let leftovers: Vec<_> = std::fs::read_dir(scratch.join("tmp"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "no credential file is left behind: {leftovers:?}"
    );
}
