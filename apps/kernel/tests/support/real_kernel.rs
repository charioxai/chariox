//! A real kernel process that requires its host's token, as a supervisor starts
//! a managed worker: the token is left in a private file the kernel reads once
//! and removes. Included by the integration tests that drive it over its socket.
#![allow(dead_code)]

use std::ffi::OsString;
use std::net::TcpListener;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct Kernel {
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

pub fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Starts the kernel under `scratch`; `open_files` lowers its descriptor limit.
pub fn start(scratch: &Path, env: &[(&str, OsString)], open_files: Option<u64>) -> Kernel {
    std::fs::create_dir_all(scratch.join("project")).unwrap();
    std::fs::create_dir_all(scratch.join("tmp")).unwrap();
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
    // Run the same entry-point drill against retained control builds too.
    let binary = std::env::var_os("CHARIOX_TEST_KERNEL_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_chariox-kernel")));
    let mut command = Command::new(binary);
    if let Some(limit) = open_files {
        // SAFETY: setrlimit is async-signal-safe; it runs between fork and exec.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, move || {
                let limit = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = command
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
        .envs(env.iter().map(|(name, value)| (*name, value)))
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

pub async fn connect(kernel: &Kernel) -> Socket {
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
pub async fn request(socket: &mut Socket, body: Value) -> Result<Value, Value> {
    let id = format!("real-kernel-{:016x}", rand::random::<u64>());
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

pub async fn create_session(socket: &mut Socket, scratch: &Path) -> String {
    let project = scratch.join("project").display().to_string();
    let created = request(
        socket,
        json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
            "alias": null, "slice_ref": null, "no_subscribe": true}}),
    )
    .await
    .unwrap();
    created["SessionCreated"]["session"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A Room terminal operation: its result, or the error the kernel refused it with.
pub async fn terminal(
    socket: &mut Socket,
    session_id: &str,
    operation: Value,
) -> Result<Value, Value> {
    request(
        socket,
        json!({"RoomTerminal": {"session_id": session_id, "operation": operation}}),
    )
    .await
    .map(|response| response["RoomTerminal"]["result"].clone())
}
