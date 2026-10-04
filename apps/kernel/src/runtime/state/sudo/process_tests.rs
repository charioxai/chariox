//! Real PTY launches and a throwaway kernel; no provider accounts or credentials.
use super::tests::{fixture_with_provider, popup, running, PASSKEY};
use super::*;
use crate::runtime::kernel_access::process::{inspect, ProcessIdentity};
use futures_util::{SinkExt, StreamExt};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

struct ShellDrillScratch(std::path::PathBuf);

impl ShellDrillScratch {
    fn new() -> Self {
        // macOS TMPDIR can make a normal test-worktree socket path exceed
        // sockaddr_un.sun_path. A canonical /tmp child stays short on both OSes.
        let root = std::fs::canonicalize("/tmp").unwrap().join(format!(
            "chariox-sudo-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ShellDrillScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn file(path: &Path) -> String {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(path) {
                if !value.is_empty() {
                    break value;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn root(f: &super::tests::Fixture) -> ProcessIdentity {
    f.state
        .owned
        .provider_process_tracking
        .read()
        .processes
        .values()
        .next()
        .unwrap()
        .identity
        .clone()
        .unwrap()
}

#[tokio::test]
async fn sudo_shell_process_identity_rejects_pid_reuse_and_shared_provider() {
    let f = fixture_with_provider(Some("while :; do sleep 1; done"));
    let turn = running(&f);
    let identity = root(&f);
    assert_eq!(
        unsafe { libc::getsid(identity.pid as i32) },
        identity.pid as i32
    );
    assert_eq!(f.state.sudo_for_peer(&identity).unwrap(), turn);
    {
        let mut tracking = f.state.owned.provider_process_tracking.write();
        let launched = tracking
            .processes
            .values_mut()
            .next()
            .unwrap()
            .identity
            .as_mut()
            .unwrap();
        launched.executable = "launcher-before-exec".into();
        launched.version = launched.version.wrapping_add(1);
    }
    assert_eq!(f.state.sudo_for_peer(&identity).unwrap(), turn);
    let mut reused_peer = identity.clone();
    reused_peer.start += 1;
    assert!(f.state.sudo_for_peer(&reused_peer).is_err());
    {
        let mut tracking = f.state.owned.provider_process_tracking.write();
        tracking
            .processes
            .values_mut()
            .next()
            .unwrap()
            .identity
            .as_mut()
            .unwrap()
            .start += 1;
    }
    assert!(f.state.sudo_for_peer(&identity).is_err());
    {
        let mut tracking = f.state.owned.provider_process_tracking.write();
        let process = tracking.processes.values_mut().next().unwrap();
        process.identity = Some(identity.clone());
        process.owner_provider_run_ids.push("sibling-run".into());
    }
    assert!(f.state.sudo_for_peer(&identity).is_err());
}

#[tokio::test]
async fn sudo_shell_piped_claude_has_dedicated_session_and_tracked_identity() {
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    let f = fixture_with_provider(None);
    let launch = LaunchProviderRequest::new(
        &f.request.session_id,
        "claude",
        "claude",
        "default",
        "default",
    )
    .with_agent_id(f.request.target_agent_id.as_deref().unwrap());
    let mut run = RuntimeProviderRun::new(
        f.run.id(),
        &launch,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude:stream-json".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                r#"while IFS= read -r line; do printf '%s\n' "$line"; done"#.into(),
            ],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    {
        let mut providers = f.state.owned.provider_store.write();
        providers.insert_run_for_test(run.clone());
        providers.initialize_runtime(&run).unwrap();
    }
    let identity = f
        .state
        .owned
        .provider_store
        .read()
        .launched_process_identities()
        .pop()
        .unwrap()
        .1;
    assert_eq!(
        unsafe { libc::getsid(identity.pid as i32) },
        identity.pid as i32
    );
    let turn = running(&f);
    assert_eq!(f.state.sudo_for_peer(&identity).unwrap(), turn);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &turn.agent_id)
        .unwrap();
    assert!(f.state.sudo_for_peer(&identity).is_err());
    // Fixture teardown owns the provider service and its piped child.
}

struct SiblingCleanup(Vec<u32>);
impl Drop for SiblingCleanup {
    fn drop(&mut self) {
        for pid in &self.0 {
            unsafe {
                libc::kill(*pid as i32, libc::SIGKILL);
            }
        }
    }
}

#[tokio::test]
async fn sudo_shell_sibling_joining_process_group_has_no_authority() {
    let scratch = crate::test_support::TestWorktree::new("sudo-group");
    let script = scratch.path().join("siblings.py");
    std::fs::write(
        &script,
        r#"
import os, time, sys
root=sys.argv[1]
target=os.fork()
if target==0:
    os.setpgid(0,0)
    open(root+'/target','w').write(str(os.getpid()))
    while True: time.sleep(1)
while not os.path.exists(root+'/target'): time.sleep(.01)
sibling=os.fork()
if sibling==0:
    os.setpgid(0,target)
    open(root+'/sibling','w').write(str(os.getpid()))
    while True: time.sleep(1)
while True: time.sleep(1)
"#,
    )
    .unwrap();
    let f = fixture_with_provider(Some(&format!(
        "exec python3 '{}' '{}'",
        script.display(),
        scratch.path().display()
    )));
    running(&f);
    let target = file(&scratch.path().join("target"))
        .await
        .parse::<u32>()
        .unwrap();
    let sibling = file(&scratch.path().join("sibling"))
        .await
        .parse::<u32>()
        .unwrap();
    let _children = SiblingCleanup(vec![target, sibling]);
    assert_eq!(unsafe { libc::getpgid(target as i32) }, unsafe {
        libc::getpgid(sibling as i32)
    });
    let target_identity = inspect(target).unwrap().0;
    // Model the target's tracked boundary in a shared OS session, deliberately
    // weaker than today's setsid launch, to prove group membership is irrelevant.
    f.state
        .owned
        .provider_process_tracking
        .write()
        .processes
        .values_mut()
        .next()
        .unwrap()
        .identity = Some(target_identity.clone());
    assert!(f.state.sudo_for_peer(&target_identity).is_ok());
    assert!(f.state.sudo_for_peer(&inspect(sibling).unwrap().0).is_err());
}

#[test]
#[ignore = "shell CLI subprocess for the throwaway-kernel drill"]
fn sudo_shell_cli_child() {
    let Ok(root) = std::env::var("CHARIOX_SUDO_SHELL_PROBE_ROOT") else {
        return;
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = Path::new(&root);
            let (mut socket, _) = tokio_tungstenite::client_async(
                "ws://localhost/kernel",
                tokio::net::UnixStream::connect(root.join("kernel.sock"))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({"type":"request","request_id":"probe",
            "command_id":"same-command-each-turn", "request":{"ListSessions":null}})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            loop {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    continue;
                };
                let result: serde_json::Value = serde_json::from_str(&text).unwrap();
                if result["request_id"] == "probe" {
                    std::fs::write(root.join("result"), text.as_bytes()).unwrap();
                    break;
                }
            }
        });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sudo_shell_live_throwaway_kernel_cli_loses_authority_after_yield() {
    let scratch = ShellDrillScratch::new();
    let scratch_root = scratch.path();
    let executable = std::env::current_exe().unwrap();
    let cli = std::env::var("CHARIOX_SUDO_SHELL_CLI").ok();
    std::fs::write(scratch_root.join("command.chx"), "session list\n").unwrap();
    let cli_command = cli.as_ref().map(|cli| format!(
        "node '{cli}' run '{0}/command.chx' --kernel-url 'ws+unix://{0}/kernel.sock' > '{0}/cli-output' 2>&1; echo $? > '{0}/cli-status'", scratch_root.display()
    )).unwrap_or_default();
    let script = format!(
        r#"
while :; do
    if test -f '{0}/probe'; then
        rm '{0}/probe'
        CHARIOX_SUDO_SHELL_PROBE_ROOT='{0}' '{1}' sudo_shell_cli_child --ignored --nocapture
        {2}
    fi
    sleep .02
done
"#,
        scratch_root.display(),
        executable.display(),
        cli_command
    );
    let f = fixture_with_provider(Some(&script));
    let socket = scratch_root.join("kernel.sock");
    {
        let mut config = f.state.owned.config_projection.snapshot();
        config.local_socket_path = socket.clone();
        config.runtime_mcp_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        f.state.owned.config_projection.update(config);
    }
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = Arc::new(f.router.clone());
    let server = tokio::spawn(
        crate::runtime_transport::run_kernel_websocket_server_with_router_on_listener(
            router,
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            async {
                let _ = stopped.await;
            },
        ),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while !socket.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let state = f.state.clone();
    let request = f.request.clone();
    let entry = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    let response = entry.await.unwrap().unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::PromptSubmitted { .. }
    ));
    std::fs::write(scratch_root.join("probe"), "probe").unwrap();
    let allowed: serde_json::Value =
        serde_json::from_str(&file(&scratch_root.join("result")).await).unwrap();
    assert!(allowed["error"].is_null(), "{allowed}");
    assert!(
        allowed["response"]["SessionsListed"]["sessions"]
            .as_array()
            .unwrap()
            .len()
            >= 1
    );
    if cli.is_some() {
        let status = file(&scratch_root.join("cli-status")).await;
        let output = file(&scratch_root.join("cli-output")).await;
        assert_eq!(status.trim(), "0", "{output}");
        println!(
            "chariox-shell sudo: {}",
            file(&scratch_root.join("cli-output")).await.trim()
        );
        std::fs::remove_file(scratch_root.join("cli-status")).unwrap();
    }
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, f.request.target_agent_id.as_deref().unwrap())
        .unwrap();
    std::fs::remove_file(scratch_root.join("result")).unwrap();
    std::fs::write(scratch_root.join("probe"), "probe").unwrap();
    let denied: serde_json::Value =
        serde_json::from_str(&file(&scratch_root.join("result")).await).unwrap();
    assert_eq!(denied["error"]["code"], "kernel_access_denied", "{denied}");
    if cli.is_some() {
        let status = file(&scratch_root.join("cli-status")).await;
        let output = file(&scratch_root.join("cli-output")).await;
        assert_ne!(status.trim(), "0", "{output}");
        assert!(
            output.contains("Unix peer has no live authority"),
            "{output}"
        );
        println!("chariox-shell after yield: {}", output.trim());
    }
    println!("live shell CLI: test passkey admitted; setsid root={}; ListSessions accepted; same command after yield refused", root(&f).pid);
    let _ = stop.send(());
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn sudo_shell_excludes_old_descendants_and_their_new_children() {
    let scratch = ShellDrillScratch::new();
    let script = scratch.path().join("birth-fence.py");
    std::fs::write(
        &script,
        r#"
import os, sys, time
root = sys.argv[1]
def wait(name):
    while not os.path.exists(root+'/'+name): time.sleep(.01)
def idle():
    while True: time.sleep(1)
old = os.fork()
if old == 0:
    open(root+'/old', 'w').write(str(os.getpid()))
    wait('go')
    grandchild = os.fork()
    if grandchild == 0:
        open(root+'/grandchild', 'w').write(str(os.getpid()))
        idle()
    idle()
wait('go')
new = os.fork()
if new == 0:
    open(root+'/new', 'w').write(str(os.getpid()))
    idle()
idle()
"#,
    )
    .unwrap();
    let f = fixture_with_provider(Some(&format!(
        "exec python3 '{}' '{}'",
        script.display(),
        scratch.path().display()
    )));
    let old = file(&scratch.path().join("old"))
        .await
        .parse::<u32>()
        .unwrap();
    // Make the old process unambiguously earlier than the admission tick.
    tokio::time::sleep(Duration::from_millis(30)).await;
    let turn = running(&f);
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(scratch.path().join("go"), "go").unwrap();
    let new = file(&scratch.path().join("new"))
        .await
        .parse::<u32>()
        .unwrap();
    let grandchild = file(&scratch.path().join("grandchild"))
        .await
        .parse::<u32>()
        .unwrap();
    let _children = SiblingCleanup(vec![old, new, grandchild]);
    assert!(f.state.sudo_for_peer(&inspect(old).unwrap().0).is_err());
    assert!(f
        .state
        .sudo_for_peer(&inspect(grandchild).unwrap().0)
        .is_err());
    assert_eq!(
        f.state.sudo_for_peer(&inspect(new).unwrap().0).unwrap(),
        turn
    );
    assert_eq!(
        f.state
            .sudo_for_peer(&inspect(root(&f).pid).unwrap().0)
            .unwrap(),
        turn
    );
}
