//! Real separate processes on the real Unix and TCP listeners. The holder and
//! kernel are siblings, just as an external agent and a laptop kernel are.
use super::*;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use tokio_tungstenite::{client_async, WebSocketStream};

const PASSKEY: &str = "Access TEST Passkey";
const AUTH: &str = "test-terminal";
const SESSION: &str = "access-session";

mod credential_authority;

#[test]
#[ignore = "subprocess entry point"]
fn kernel_access_child_server() {
    let Ok(root) = std::env::var("CHARIOX_ACCESS_TEST_ROOT") else {
        return;
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .unwrap()
        .block_on(async {
            let root = PathBuf::from(root);
            let worktree = root.join("worktree");
            std::fs::create_dir(&worktree).unwrap();
            let tcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
            let addr = tcp.local_addr().unwrap();
            let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
            let mut config = daemon_config_for_runtime_mcp_listener(&mcp);
            config.local_socket_path = root.join("run/k.sock");
            // Sudo exercises an ordinary home kernel. A retained publication
            // kernel has a separate preparation barrier before prompt admission.
            config.publication_control_state_root =
                (std::env::var("CHARIOX_ACCESS_TEST_SUDO").as_deref() != Ok("1"))
                    .then(|| root.join("control"));
            config.user_config_path = root.join("private/config.toml");
            config.user_config.credential_vault.backend =
                crate::config::CredentialVaultBackend::CharioxEncrypted;
            let vault = root.join("vault.json");
            config.user_config.credential_vault.path = vault.to_string_lossy().into_owned();
            config.user_config.credential_vault.unlock_policy =
                crate::config::CredentialVaultUnlockPolicy::KernelInit;
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY).unwrap();
            let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
            for id in [SESSION, "other-session"] {
                let mut session = crate::session::RuntimeSession::new(
                    id,
                    // A valid alias can collide with another session's ID.
                    // Scoping must use the same ID-first resolver as dispatch.
                    Some(
                        if id == SESSION {
                            "other-session"
                        } else {
                            "other-alias"
                        }
                        .into(),
                    ),
                    "workspace",
                    worktree.to_str().unwrap(),
                    "machine",
                    "kernel",
                );
                session.add_member(
                    "guest",
                    Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
                    crate::session::CollaborationLevel::Full,
                );
                app.sessions_mut().restore_session(session);
            }
            for id in ["access-vault-agent", "access-second-agent"] {
                app.agents_mut()
                    .restore_agent(crate::agent::AgentInstance::new(
                        id,
                        id,
                        SESSION,
                        None,
                        "claude",
                        None,
                        None,
                        None,
                        crate::agent::GridPosition::new(0, 0, 1, 1),
                    ));
            }
            let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
                Arc::new(Mutex::new(app)),
                32,
            ));
            let runtime = router.runtime_state();
            let _critical = runtime
                .raise_critical_approval_for_test(SESSION, "critical-test")
                .await;
            let (stop, stopped) = oneshot::channel();
            let control_root = root.clone();
            tokio::spawn(async move {
                loop {
                    sleep(Duration::from_millis(20)).await;
                    if let Ok(action) = std::fs::read_to_string(control_root.join("command")) {
                        std::fs::remove_file(control_root.join("command")).unwrap();
                        if action == "stop" {
                            let _ = stop.send(());
                            break;
                        }
                        if action == "noinherit" || action == "kernelchild" {
                            let previous = (action == "noinherit")
                                .then(|| runtime.use_kernel_ancestor_holder_for_test());
                            let root = control_root.clone();
                            let status = tokio::task::spawn_blocking(move || {
                                Command::new(std::env::current_exe().unwrap())
                                    .args([
                                        "kernel_access_descendant_child",
                                        "--ignored",
                                        "--nocapture",
                                    ])
                                    .env("CHARIOX_ACCESS_TEST_CLIENT", root)
                                    .env("CHARIOX_ACCESS_TEST_KERNEL_CHILD", "1")
                                    .stdout(Stdio::null())
                                    .status()
                                    .unwrap()
                            })
                            .await
                            .unwrap();
                            if let Some(previous) = previous {
                                runtime.restore_access_holder_for_test(previous);
                            }
                            assert!(status.success());
                        } else {
                            runtime.control_access_for_test(&action, &vault).await;
                        }
                        std::fs::write(control_root.join("ack"), &action).unwrap();
                    }
                }
            });
            std::fs::write(root.join("ready"), addr.to_string()).unwrap();
            run_kernel_websocket_server_with_bound_listeners(
                router,
                adopt_std_listener(tcp, "tcp").unwrap(),
                adopt_std_listener(mcp, "mcp").unwrap(),
                KernelLocalAuth::LocalToken(Arc::new(local_auth::LocalTokenAuth::new(AUTH.into()))),
                async {
                    let _ = stopped.await;
                },
            )
            .await
            .unwrap();
        });
}

#[test]
#[ignore = "subprocess entry point"]
fn kernel_access_client_child() {
    let Ok(root) = std::env::var("CHARIOX_ACCESS_TEST_CLIENT") else {
        return;
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (mut socket, _) = client_async(
                "ws://localhost/kernel",
                tokio::net::UnixStream::connect(Path::new(&root).join("run/k.sock"))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            println!("ACCESS_READY");
            std::io::stdout().flush().unwrap();
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                if line == "exit" {
                    break;
                }
                if line == "subscriber" {
                    let child = Command::new(std::env::current_exe().unwrap())
                        .args(["kernel_access_idle_descendant", "--ignored", "--nocapture"])
                        .env("CHARIOX_ACCESS_TEST_CLIENT", &root)
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit())
                        .spawn()
                        .unwrap();
                    let identity = crate::runtime::kernel_access::process::inspect(child.id())
                        .unwrap()
                        .0;
                    std::fs::write(
                        Path::new(&root).join("descendant-pid"),
                        format!("{} {}", identity.pid, identity.start),
                    )
                    .unwrap();
                    timeout(Duration::from_secs(5), async {
                        while !Path::new(&root).join("descendant-ready").exists() {
                            sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap();
                    println!("ACCESS {{\"subscriber\":true}}");
                } else if line == "child" {
                    // A fresh OS descendant opens its own connection and presents nothing.
                    let child = Command::new(std::env::current_exe().unwrap())
                        .args(["kernel_access_descendant_child", "--ignored", "--nocapture"])
                        .env("CHARIOX_ACCESS_TEST_CLIENT", &root)
                        .output()
                        .unwrap();
                    assert!(
                        child.status.success(),
                        "{}",
                        String::from_utf8_lossy(&child.stderr)
                    );
                    println!("ACCESS {}", serde_json::json!({"descendant": true}));
                } else if line == "second-session" {
                    let (mut second, _) = client_async("ws://localhost/kernel",
                        tokio::net::UnixStream::connect(Path::new(&root).join("run/k.sock")).await.unwrap()).await.unwrap();
                    second.send(Message::Text(frame(serde_json::json!({"GetSessionState":{"session_id":"other-session"}})).to_string().into())).await.unwrap();
                    let state = response(&mut second, "request").await;
                    assert!(state["error"].is_null(), "{state}");
                    second.send(Message::Text(frame(serde_json::json!({"ListSessions":null})).to_string().into())).await.unwrap();
                    let list = response(&mut second, "request").await;
                    let sessions = list["response"]["SessionsListed"]["sessions"].as_array().unwrap();
                    assert_eq!(sessions.len(), 1, "{list}");
                    assert_eq!(sessions[0]["id"], "other-session");
                    second.send(Message::Text(frame(serde_json::json!({"AttachToSession":{"session_id":"other-session","client_id":"second-session","capability_level":"FullTerminal"}})).to_string().into())).await.unwrap();
                    let attached = response(&mut second, "request").await;
                    let attachment = attached["response"]["SessionAttached"]["attachment"]["id"].as_str().unwrap();
                    second.send(Message::Text(serde_json::json!({"type":"subscribe","request_id":"second-subscribe","session_id":"other-session","attachment_id":attachment}).to_string().into())).await.unwrap();
                    assert!(response(&mut second, "second-subscribe").await["error"].is_null());
                    println!("ACCESS {{\"second_session\":true}}");
                } else if line == "next" {
                    loop {
                        let next = timeout(Duration::from_secs(5), socket.next())
                            .await
                            .unwrap();
                        match next {
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                                println!("ACCESS {{\"closed\":true}}");
                                break;
                            }
                            Some(Ok(Message::Ping(payload))) => {
                                let _ = socket.send(Message::Pong(payload)).await;
                            }
                            _ => {}
                        }
                    }
                } else {
                    let mut value: Value = serde_json::from_str(&line).unwrap();
                    if value["request"]["RequestKernelAccess"]["holder_pid"] == 0 {
                        value["request"]["RequestKernelAccess"]["holder_pid"] =
                            std::process::id().into();
                    }
                    socket
                        .send(Message::Text(value.to_string().into()))
                        .await
                        .unwrap();
                    let response =
                        response(&mut socket, value["request_id"].as_str().unwrap()).await;
                    println!("ACCESS {response}");
                }
                std::io::stdout().flush().unwrap();
            }
        });
}

#[test]
#[ignore = "subprocess entry point"]
fn kernel_access_descendant_child() {
    let Ok(root) = std::env::var("CHARIOX_ACCESS_TEST_CLIENT") else {
        return;
    };
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let (mut socket, _) = client_async("ws://localhost/kernel", tokio::net::UnixStream::connect(Path::new(&root).join("run/k.sock")).await.unwrap()).await.unwrap();
        socket.send(Message::Text(frame(serde_json::json!({"GetSessionState": {"session_id": SESSION}})).to_string().into())).await.unwrap();
        let result = response(&mut socket, "request").await;
        if std::env::var_os("CHARIOX_ACCESS_TEST_KERNEL_CHILD").is_some() {
            assert_eq!(result["error"]["code"], "kernel_access_denied", "kernel child inherited: {result}");
            socket.send(Message::Text(frame(serde_json::json!({"RequestKernelAccess":{"session_id":SESSION,"holder_pid":std::process::id()}})).to_string().into())).await.unwrap();
            assert!(response(&mut socket, "request").await["error"].is_object());
        } else { assert!(result["error"].is_null(), "{result}"); }
    });
}

#[test]
#[ignore = "subprocess entry point"]
fn kernel_access_idle_descendant() {
    let Ok(root) = std::env::var("CHARIOX_ACCESS_TEST_CLIENT") else {
        return;
    };
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let root = Path::new(&root);
        let (mut socket, _) = client_async("ws://localhost/kernel", tokio::net::UnixStream::connect(root.join("run/k.sock")).await.unwrap()).await.unwrap();
        socket.send(Message::Text(frame(serde_json::json!({"AttachToSession":{"session_id":SESSION,"client_id":"idle-descendant","capability_level":"FullTerminal"}})).to_string().into())).await.unwrap();
        let attach = response(&mut socket, "request").await;
        let attachment = attach["response"]["SessionAttached"]["attachment"]["id"].as_str().unwrap();
        socket.send(Message::Text(serde_json::json!({"type":"subscribe","request_id":"subscribe","session_id":SESSION,"attachment_id":attachment}).to_string().into())).await.unwrap();
        assert!(response(&mut socket, "subscribe").await["error"].is_null());
        std::fs::write(root.join("descendant-ready"), "ready").unwrap();
        timeout(Duration::from_secs(10), async { loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Ping(data))) => { let _ = socket.send(Message::Pong(data)).await; },
                _ => {},
            }
        }}).await.unwrap();
        std::fs::write(root.join("descendant-closed"), "closed").unwrap();
    });
}

async fn response<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    id: &str,
) -> Value {
    timeout(Duration::from_secs(15), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "response" && frame["request_id"] == id {
                        break frame;
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    socket.send(Message::Pong(data)).await.unwrap();
                }
                Some(Ok(_)) => {}
                other => panic!("socket closed while awaiting response: {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

fn frame(request: Value) -> Value {
    serde_json::json!({"type":"request", "request_id":"request", "command_id":format!("access-test-{:016x}", rand::random::<u64>()), "request":request})
}

struct Client {
    child: Child,
    input: std::process::ChildStdin,
    output: std::io::BufReader<std::process::ChildStdout>,
}
impl Client {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["kernel_access_client_child", "--ignored", "--nocapture"])
            .env("CHARIOX_ACCESS_TEST_CLIENT", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut result = Self {
            child,
            input,
            output,
        };
        loop {
            let mut line = String::new();
            assert_ne!(result.output.read_line(&mut line).unwrap(), 0);
            if line.trim_end().ends_with("ACCESS_READY") {
                break;
            }
        }
        result
    }
    fn send(&mut self, request: Value) {
        writeln!(self.input, "{}", frame(request)).unwrap();
    }
    fn command(&mut self, command: &str) {
        writeln!(self.input, "{command}").unwrap();
    }
    fn result(&mut self) -> Value {
        loop {
            let mut line = String::new();
            assert_ne!(self.output.read_line(&mut line).unwrap(), 0);
            if let Some(result) = line.strip_prefix("ACCESS ") {
                return serde_json::from_str(result).unwrap();
            }
        }
    }
    fn request(&mut self, request: Value) -> Value {
        self.send(request);
        self.result()
    }
    fn cached_request(&mut self, request: Value, command: &str) -> Value {
        let mut frame = frame(request);
        frame["command_id"] = command.into();
        writeln!(self.input, "{frame}").unwrap();
        self.result()
    }
    fn subscribe(&mut self) {
        let attach = self.request(serde_json::json!({"AttachToSession":{"session_id":SESSION,"client_id":"external-holder", "capability_level":"FullTerminal"}}));
        assert!(attach["error"].is_null(), "{attach}");
        let id = attach["response"]["SessionAttached"]["attachment"]["id"]
            .as_str()
            .unwrap();
        writeln!(self.input, "{}", serde_json::json!({"type":"subscribe", "request_id":"subscribe", "session_id":SESSION, "attachment_id":id})).unwrap();
        let subscribed = self.result();
        assert!(subscribed["error"].is_null(), "{subscribed}");
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Kernel {
    root: PathBuf,
    child: Child,
    tcp: WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}
impl Kernel {
    async fn start() -> Self {
        Self::start_with_order("").await
    }
    async fn start_with_order(order: &str) -> Self {
        Self::start_with_options(order, false).await
    }
    async fn start_with_options(order: &str, sudo: bool) -> Self {
        // Keep macOS's 104-byte sockaddr_un limit, including the temporary prefix.
        let root = std::env::temp_dir().join(format!("a{:08x}", rand::random::<u32>()));
        std::fs::create_dir(&root).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["kernel_access_child_server", "--ignored", "--nocapture"])
            .env("CHARIOX_ACCESS_TEST_ROOT", &root)
            .env("CHARIOX_ACCESS_TEST_GRANT_ORDER", order)
            .env("CHARIOX_ACCESS_TEST_SUDO", if sudo { "1" } else { "0" })
            .env("CHARIOX_HOME", root.join("state"))
            .env("HOME", root.join("home"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let addr = timeout(Duration::from_secs(20), async {
            loop {
                if let Ok(addr) = std::fs::read_to_string(root.join("ready")) {
                    break addr;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let mut request = format!("ws://{addr}").into_client_request().unwrap();
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {AUTH}")).unwrap(),
        );
        let (tcp, _) = connect_async(request).await.unwrap();
        let kernel = Self { root, child, tcp };
        timeout(Duration::from_secs(5), async {
            while !kernel.root.join("run/k.sock").exists() {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        kernel
    }
    async fn request(&mut self, request: Value) -> Value {
        self.tcp
            .send(Message::Text(frame(request).to_string().into()))
            .await
            .unwrap();
        response(&mut self.tcp, "request").await
    }
    async fn cached_request(&mut self, request: Value, command: &str) -> Value {
        let mut frame = frame(request);
        frame["command_id"] = command.into();
        self.tcp
            .send(Message::Text(frame.to_string().into()))
            .await
            .unwrap();
        response(&mut self.tcp, "request").await
    }
    async fn prompts(&mut self) -> Vec<Value> {
        self.prompts_for(SESSION).await
    }
    async fn prompts_for(&mut self, session: &str) -> Vec<Value> {
        // The owner snapshots include active interactions. The popup feed uses this same board.
        let response = self
            .request(serde_json::json!({"GetSessionState":{"session_id":session}}))
            .await;
        assert!(response["error"].is_null(), "{response}");
        response["response"]["SessionState"]["session"]["active_interactions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    async fn access_prompt(&mut self, suffix: &str) -> String {
        self.access_prompt_for(SESSION, suffix).await
    }
    async fn access_prompt_for(&mut self, session: &str, suffix: &str) -> String {
        timeout(Duration::from_secs(10), async {
            loop {
                for prompt in self.prompts_for(session).await {
                    if let Some(id) = prompt["id"].as_str() {
                        if id.ends_with(suffix) {
                            return id.into();
                        }
                    }
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn answer(&mut self, id: &str, passkey: Option<&str>, minutes: Option<u32>) -> Value {
        self.request(serde_json::json!({"RespondToInteraction": {"session_id":SESSION, "interaction_id":id, "choice_id":"approve", "passkey":passkey, "custom_reply":minutes.map(|m|m.to_string())}})).await
    }
    async fn control(&self, action: &str) {
        let _ = std::fs::remove_file(self.root.join("ack"));
        std::fs::write(self.root.join("command"), action).unwrap();
        timeout(Duration::from_secs(15), async {
            loop {
                if std::fs::read_to_string(self.root.join("ack")).is_ok_and(|a| a == action) {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Kernel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Ok(pid) = std::fs::read_to_string(self.root.join("descendant-pid")) {
            let fields = pid.split_whitespace().collect::<Vec<_>>();
            if let [pid, start] = fields.as_slice() {
                if let (Ok(pid), Ok(start)) = (pid.parse::<u32>(), start.parse::<u64>()) {
                    if crate::runtime::kernel_access::process::inspect(pid)
                        .is_ok_and(|(p, _)| p.start == start)
                    {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGTERM);
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn grant(kernel: &mut Kernel, holder: &mut Client) -> String {
    grant_for_holder(kernel, holder, 0).await
}

async fn grant_for_holder(kernel: &mut Kernel, holder: &mut Client, holder_pid: u32) -> String {
    holder.send(
        serde_json::json!({"RequestKernelAccess":{"session_id":SESSION,"holder_pid":holder_pid}}),
    );
    let id = kernel.access_prompt("-grant").await;
    let prompt = kernel
        .prompts()
        .await
        .into_iter()
        .find(|p| p["id"] == id)
        .unwrap();
    let message = prompt["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "pid {}",
            if holder_pid == 0 {
                holder.child.id()
            } else {
                holder_pid
            }
        )),
        "{message}"
    );
    assert!(message.contains(SESSION) && message.contains("30 minutes"));
    assert!(message.contains(
        std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    ));
    kernel.control("guest").await;
    let missing = kernel.answer(&id, None, None).await;
    assert!(missing["error"]["message"]
        .as_str()
        .unwrap()
        .contains("PASSKEY_REQUIRED"));
    let wrong = kernel.answer(&id, Some("wrong"), None).await;
    assert!(wrong["error"]["message"]
        .as_str()
        .unwrap()
        .contains("PASSKEY_REJECTED"));
    let approved = kernel.answer(&id, Some(PASSKEY), Some(15)).await;
    assert!(approved["error"].is_null(), "{approved}");
    let result = holder.result();
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(
        result["response"]["KernelAccessGranted"]["grant"]["lifetime_minutes"],
        15
    );
    result["response"]["KernelAccessGranted"]["grant"]["grant_id"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn kernel_access_grants_identify_holder_descendants_and_refuse_sibling_scope_and_critical() {
    let mut kernel = Kernel::start().await;
    let tcp_request = kernel.request(serde_json::json!({"RequestKernelAccess":{"session_id":SESSION, "holder_pid":std::process::id()}})).await;
    assert!(
        tcp_request["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ws+unix://"),
        "{tcp_request}"
    );
    assert!(kernel
        .request(serde_json::json!({"GetSessionState":{"session_id":SESSION}}))
        .await["error"]
        .is_null());
    for header in ["Authorization", "Origin"] {
        let mut request = "ws://localhost/kernel".into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header, HeaderValue::from_static("refused"));
        let stream = tokio::net::UnixStream::connect(kernel.root.join("run/k.sock"))
            .await
            .unwrap();
        let result = client_async(request, stream).await;
        assert!(
            matches!(result, Err(WebSocketError::Http(response)) if response.status() == StatusCode::FORBIDDEN)
        );
    }
    let mut holder = Client::start(&kernel.root);
    let mut sibling = Client::start(&kernel.root);
    let get = serde_json::json!({"GetSessionState":{"session_id":SESSION}});
    assert_eq!(
        sibling.request(get.clone())["error"]["code"],
        "kernel_access_denied"
    );
    let forged_holder = sibling.request(serde_json::json!({"RequestKernelAccess":{"session_id":SESSION,"holder_pid":holder.child.id()}}));
    assert!(forged_holder["error"].is_object(), "{forged_holder}");
    let grant_id = grant(&mut kernel, &mut holder).await;
    assert!(holder.request(get.clone())["error"].is_null());
    let own_ref = holder.request(serde_json::json!({"ResolveSession":{"session_ref":"access-s"}}));
    assert!(own_ref["error"].is_null(), "{own_ref}");
    // A sibling presents no credential; knowing the public grant id changes nothing.
    assert_eq!(
        sibling.request(get)["error"]["code"],
        "kernel_access_denied"
    );
    holder.command("child");
    assert_eq!(holder.result()["descendant"], true);
    kernel.control("noinherit").await;
    for request in [
        serde_json::json!({"ResolveSession":{"session_ref":"other-session"}}),
        serde_json::json!({"DetachFromSession":{"attachment_id":"foreign-attachment"}}),
        serde_json::json!({"SubmitPrompts":{"session_id":SESSION,"attachment_id":"foreign-attachment","prompts":[{"session_id":"other-session","target_agent_id":"a","prompt":"out of scope"}]}}),
        serde_json::json!({"GetSessionState":{"session_id":"other-session"}}),
        serde_json::json!({"GetDaemonHealth":null}),
        serde_json::json!({"RespondToInteraction":{"session_id":SESSION,"interaction_id":"critical-test","choice_id":"approve","passkey":PASSKEY}}),
        serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":grant_id}}),
    ] {
        assert_eq!(
            holder.request(request)["error"]["code"],
            "kernel_access_denied"
        );
    }
    let unfiltered = kernel
        .cached_request(
            serde_json::json!({"ListSessions":null}),
            "terminal-list-cache",
        )
        .await;
    assert_eq!(
        unfiltered["response"]["SessionsListed"]["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let list = holder.cached_request(
        serde_json::json!({"ListSessions":null}),
        "terminal-list-cache",
    );
    assert_eq!(
        list["response"]["SessionsListed"]["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{list}"
    );
    kernel.control("notice").await;
    let extension = kernel.access_prompt("-extension").await;
    assert!(
        kernel.answer(&extension, None, None).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("PASSKEY_REQUIRED")
    );
    assert!(kernel.answer(&extension, Some(PASSKEY), Some(45)).await["error"].is_null());
    timeout(Duration::from_secs(5), async {
        loop {
            let listed = kernel
                .request(serde_json::json!({"ListKernelAccessGrants":{}}))
                .await;
            if listed["response"]["KernelAccessGrantsListed"]["grants"][0]["lifetime_minutes"] == 45
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    holder.subscribe();
    let revoke = kernel
        .request(serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":grant_id}}))
        .await;
    assert_eq!(revoke["response"]["KernelAccessRevoked"]["revoked"], 1);
    holder.command("next");
    assert_eq!(holder.result()["closed"], true);
    // A CLI helper can name its OS ancestor. The ancestor also launched the
    // kernel, but the kernel's own agent processes still receive no grant.
    let mut ancestor_requester = Client::start(&kernel.root);
    grant_for_holder(&mut kernel, &mut ancestor_requester, std::process::id()).await;
    kernel.control("kernelchild").await;
    let revoked = kernel
        .request(serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":null}}))
        .await;
    assert_eq!(revoked["response"]["KernelAccessRevoked"]["revoked"], 1);
}

#[tokio::test]
async fn kernel_access_grants_expire_revoke_on_pid_reuse_and_real_rotation() {
    for action in ["expire", "reuse", "rotate"] {
        let mut kernel = Kernel::start().await;
        let mut holder = Client::start(&kernel.root);
        grant(&mut kernel, &mut holder).await;
        holder.subscribe();
        let mut pending = if action == "rotate" {
            let mut client = Client::start(&kernel.root);
            client.send(
                serde_json::json!({"RequestKernelAccess":{"session_id":SESSION,"holder_pid":0}}),
            );
            kernel.access_prompt("-grant").await;
            Some(client)
        } else {
            None
        };
        kernel.control(action).await;
        if let Some(pending) = &mut pending {
            assert!(
                pending.result()["error"].is_object(),
                "rotation failed to cancel pending access"
            );
        }
        holder.command("next");
        assert_eq!(holder.result()["closed"], true, "{action}");
        let mut reconnect = Client::start(&kernel.root);
        assert_eq!(
            reconnect.request(serde_json::json!({"GetSessionState":{"session_id":SESSION}}))
                ["error"]["code"],
            "kernel_access_denied"
        );
        let list = kernel
            .request(serde_json::json!({"ListKernelAccessGrants":{}}))
            .await;
        assert!(
            list["response"]["KernelAccessGrantsListed"]["grants"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{action}: {list}"
        );
    }
}

#[tokio::test]
async fn kernel_access_grants_process_exit_session_end_and_no_terminal_timeout() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    holder.command("subscriber");
    assert_eq!(holder.result()["subscriber"], true);
    drop(holder);
    timeout(Duration::from_secs(5), async {
        while !kernel.root.join("descendant-closed").exists() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            let list = kernel
                .request(serde_json::json!({"ListKernelAccessGrants":{}}))
                .await;
            if list["response"]["KernelAccessGrantsListed"]["grants"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut pending = Client::start(&kernel.root);
    pending.send(serde_json::json!({"RequestKernelAccess":{"session_id":SESSION,"holder_pid":0}}));
    kernel.access_prompt("-grant").await;
    kernel.control("timeout").await;
    assert!(pending.result()["error"].is_object());
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    holder.subscribe();
    let end = kernel
        .request(serde_json::json!({"EndSession":{"session_id":SESSION}}))
        .await;
    assert!(end["error"].is_null(), "{end}");
    holder.command("next");
    assert_eq!(holder.result()["closed"], true);
}

#[tokio::test]
async fn kernel_access_overlapping_grants_keep_subscriptions_bound_and_select_by_session() {
    for order in ["ancestor-first", "descendant-first"] {
        let mut kernel = Kernel::start_with_order(order).await;
        let mut helper = Client::start(&kernel.root);
        // The test process is the holder; helper, descendant and kernel are separate processes.
        let ancestor_id = grant_for_holder(&mut kernel, &mut helper, std::process::id()).await;
        let mut descendant = Client::start(&kernel.root);
        descendant.subscribe();
        // Approve its own session B grant on the socket already subscribed to A.
        descendant.send(serde_json::json!({"RequestKernelAccess": {
            "session_id":"other-session", "holder_pid":0
        }}));
        let prompt = kernel.access_prompt_for("other-session", "-grant").await;
        let approved = kernel
            .request(serde_json::json!({"RespondToInteraction": {
                "session_id":"other-session", "interaction_id":prompt,
                "choice_id":"approve", "passkey":PASSKEY
            }}))
            .await;
        assert!(approved["error"].is_null(), "{approved}");
        let granted = descendant.result();
        assert!(granted["error"].is_null(), "{granted}");
        let descendant_id = granted["response"]["KernelAccessGranted"]["grant"]["grant_id"]
            .as_str()
            .unwrap();
        assert_eq!(
            ancestor_id.as_str() < descendant_id,
            order == "ancestor-first"
        );
        assert_eq!(
            descendant
                .request(serde_json::json!({"GetSessionState":{"session_id":"other-session"}}))
                ["error"]["code"],
            "kernel_access_denied"
        );
        // A fresh socket selects B even when the inherited A grant sorts first.
        // Both clients are descendants of A's holder. Use B's exact same OS
        // process via a second connection in the client helper.
        descendant.command("second-session");
        assert_eq!(descendant.result()["second_session"], true);
        kernel
            .request(serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":ancestor_id}}))
            .await;
        descendant.command("next");
        assert_eq!(descendant.result()["closed"], true, "{order}");
        let grants = kernel
            .request(serde_json::json!({"ListKernelAccessGrants":{}}))
            .await;
        assert_eq!(
            grants["response"]["KernelAccessGrantsListed"]["grants"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        descendant.command("second-session");
        assert_eq!(descendant.result()["second_session"], true);
    }
}

#[tokio::test]
async fn external_sudo_unix_socket_requires_grant_projects_identity_and_expires_without_terminal() {
    let mut kernel = Kernel::start_with_options("", true).await;
    let mut holder = Client::start(&kernel.root);
    let request = serde_json::json!({"RequestKernelSudo":{"agent_id":"access-vault-agent","prompt":"full external\nprompt"}});
    assert!(!holder.request(request.clone())["error"].is_null());
    grant(&mut kernel, &mut holder).await;
    let tcp = kernel.request(request.clone()).await;
    assert!(tcp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ws+unix://"));
    holder.send(request.clone());
    let popup = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(p) = kernel
                .prompts()
                .await
                .into_iter()
                .find(|p| p["id"].as_str().is_some_and(|id| id.starts_with("sudo:")))
            {
                break p;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let id = popup["id"].as_str().unwrap();
    let message = popup["message"].as_str().unwrap();
    assert!(message.contains(&format!("OS pid {}", holder.child.id())));
    assert!(message.contains("access-vault-agent") && message.contains(SESSION));
    assert!(message.contains("Requester-supplied prompt:\nfull external\nprompt"));
    let refused = kernel.request(serde_json::json!({"RespondToInteraction":{"session_id":SESSION,"interaction_id":id,"choice_id":"refuse"}})).await;
    assert!(refused["error"].is_null(), "{refused}");
    assert!(holder.result()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("refused"));
    kernel.tcp.close(None).await.unwrap();
    holder.send(request);
    kernel.control("sudo-timeout").await;
    let expired = holder.result();
    assert!(
        expired["error"]["message"]
            .as_str()
            .unwrap()
            .contains("sudo request expired"),
        "{expired}"
    );
}
