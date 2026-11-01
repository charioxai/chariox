//! The `access request --no-wait` waiter outlives the shell that started it. The owner's
//! approval can come minutes after that shell's chain is gone, and still admits the grant.
use super::*;

/// The requester: connects, asks for access for `CHARIOX_ACCESS_TEST_HOLDER` (a pid, or
/// `parent` for its parent shell) with the term in `CHARIOX_ACCESS_TEST_TERM` (default
/// until-exit), then records its pid and waits for the owner's answer.
#[test]
#[ignore = "subprocess entry point"]
fn kernel_access_waiter_child() {
    let (Ok(root), Ok(holder)) = (
        std::env::var("CHARIOX_ACCESS_TEST_CLIENT"),
        std::env::var("CHARIOX_ACCESS_TEST_HOLDER"),
    ) else {
        return;
    };
    let holder_pid: u32 = match holder.as_str() {
        "parent" => unsafe { libc::getppid() as u32 },
        pid => pid.parse().unwrap(),
    };
    let term: Value = std::env::var("CHARIOX_ACCESS_TEST_TERM")
        .map(|term| serde_json::from_str(&term).unwrap())
        .unwrap_or_else(|_| "until_holder_exits".into());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = Path::new(&root);
            let (mut socket, _) = client_async(
                "ws://localhost/kernel",
                tokio::net::UnixStream::connect(root.join("run/k.sock"))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            socket
                .send(Message::Text(
                    frame(serde_json::json!({"RequestKernelAccess":{
                        "holder_pid": holder_pid, "term": term
                    }}))
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            std::fs::write(
                root.join("waiter-sent"),
                format!("{} {holder_pid}", std::process::id()),
            )
            .unwrap();
            let reply = timeout(Duration::from_secs(60), async {
                loop {
                    match socket.next().await {
                        Some(Ok(Message::Text(text))) => {
                            let frame: Value = serde_json::from_str(&text).unwrap();
                            if frame["type"] == "response" {
                                break frame;
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            socket.send(Message::Pong(data)).await.unwrap()
                        }
                        Some(Ok(_)) => {}
                        other => {
                            break serde_json::json!({"error": format!("socket closed: {other:?}")})
                        }
                    }
                }
            })
            .await
            .unwrap();
            std::fs::write(root.join("waiter-result"), reply.to_string()).unwrap();
        });
}

/// `sh` is the requester's parent: it runs the waiter in the background and waits for it,
/// the way a shell runs the detached `chariox access request`. Killing the shell re-parents
/// the waiter to init, as the CLI's `--no-wait` process exiting does.
pub(super) fn shell_with_waiter(root: &Path, holder: &str) -> Child {
    shell_with_waiter_term(root, holder, serde_json::json!("until_holder_exits"))
}

/// `term` is the request's term, for example `{"minutes":30}`.
pub(super) fn shell_with_waiter_term(root: &Path, holder: &str, term: Value) -> Child {
    Command::new("sh")
        .args([
            "-c",
            r#""$0" kernel_access_waiter_child --ignored --nocapture --quiet & wait"#,
        ])
        .arg(std::env::current_exe().unwrap())
        .env("CHARIOX_ACCESS_TEST_CLIENT", root)
        .env("CHARIOX_ACCESS_TEST_HOLDER", holder)
        .env("CHARIOX_ACCESS_TEST_TERM", term.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

pub(super) async fn file_text(path: PathBuf) -> String {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&path) {
                break text;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} never appeared", path.display()))
}

/// Starts the waiter under `shell`, waits for its popup, then ends the shell so the waiter
/// is no longer anything's descendant. Returns the popup id.
async fn orphaned_waiter(kernel: &mut Kernel, shell: &mut Child) -> String {
    let sent = file_text(kernel.root.join("waiter-sent")).await;
    let waiter: u32 = sent.split(' ').next().unwrap().parse().unwrap();
    let id = kernel.access_prompt("-grant").await;
    shell.kill().unwrap();
    shell.wait().unwrap();
    timeout(Duration::from_secs(5), async {
        while crate::runtime::kernel_access::process::inspect(waiter)
            .unwrap()
            .1
            == shell.id()
        {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the waiter should be re-parented once its shell is gone");
    id
}

pub(super) async fn grants(kernel: &mut Kernel) -> Vec<Value> {
    let list = kernel
        .request(serde_json::json!({"ListKernelAccessGrants":{}}))
        .await;
    list["response"]["KernelAccessGrantsListed"]["grants"]
        .as_array()
        .unwrap()
        .clone()
}

// The owner approves after the waiter lost its shell: the grant binds to the holder.
#[tokio::test]
async fn an_approval_after_the_waiter_was_re_parented_admits_the_grant() {
    let mut kernel = Kernel::start().await;
    let mut shell = shell_with_waiter(&kernel.root, &std::process::id().to_string());
    let id = orphaned_waiter(&mut kernel, &mut shell).await;
    let approved = kernel
        .answer_with(&id, Some(PASSKEY), Some("until_exit".into()))
        .await;
    assert!(approved["error"].is_null(), "{approved}");
    let result: Value =
        serde_json::from_str(&file_text(kernel.root.join("waiter-result")).await).unwrap();
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(
        result["response"]["KernelAccessGranted"]["grant"]["holder_pid"],
        std::process::id()
    );
    assert_eq!(grants(&mut kernel).await.len(), 1);
    // A client inside the holder's tree is covered by that grant.
    let mut covered = Client::start(&kernel.root);
    let state = covered.request(serde_json::json!({"GetSessionState": {"session_id": SESSION}}));
    assert!(state["error"].is_null(), "{state}");
}

// The holder is the waiter's shell, which exits before the owner decides: no grant.
#[tokio::test]
async fn an_approval_after_the_holder_exited_admits_nothing() {
    let mut kernel = Kernel::start().await;
    let mut shell = shell_with_waiter(&kernel.root, "parent");
    let id = orphaned_waiter(&mut kernel, &mut shell).await;
    let _ = kernel
        .answer_with(&id, Some(PASSKEY), Some("until_exit".into()))
        .await;
    let result: Value =
        serde_json::from_str(&file_text(kernel.root.join("waiter-result")).await).unwrap();
    assert!(result["error"].is_object(), "{result}");
    assert!(grants(&mut kernel).await.is_empty());
}

#[tokio::test]
async fn a_refused_request_admits_nothing() {
    let mut kernel = Kernel::start().await;
    let mut shell = shell_with_waiter(&kernel.root, &std::process::id().to_string());
    let id = orphaned_waiter(&mut kernel, &mut shell).await;
    let refused = kernel
        .request(serde_json::json!({"RespondToInteraction":{
            "session_id":"kernel-access", "interaction_id":id, "choice_id":"refuse"
        }}))
        .await;
    assert!(refused["error"].is_null(), "{refused}");
    let result: Value =
        serde_json::from_str(&file_text(kernel.root.join("waiter-result")).await).unwrap();
    assert!(result["error"].is_object(), "{result}");
    assert!(grants(&mut kernel).await.is_empty());
}

// A holder the requester does not sit under is refused when asked, not when approved.
#[tokio::test]
async fn a_requester_outside_the_holder_is_refused_at_request_time() {
    let kernel = Kernel::start().await;
    let mut other = Command::new("sleep").arg("300").spawn().unwrap();
    let mut shell = shell_with_waiter(&kernel.root, &other.id().to_string());
    let result: Value =
        serde_json::from_str(&file_text(kernel.root.join("waiter-result")).await).unwrap();
    assert!(result["error"].is_object(), "{result}");
    shell.kill().unwrap();
    shell.wait().unwrap();
    other.kill().unwrap();
    other.wait().unwrap();
}
