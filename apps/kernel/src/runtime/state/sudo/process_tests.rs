//! Real PTY launches and a throwaway kernel; no provider accounts or credentials.
use super::tests::{fixture_with_provider, popup, running, PASSKEY};
use super::*;
use crate::runtime::kernel_access::process::{inspect, ProcessIdentity};
use futures_util::{SinkExt, StreamExt};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

struct ShellDrillShutdown(Option<tokio::sync::oneshot::Sender<()>>);
impl Drop for ShellDrillShutdown {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            let _ = stop.send(());
        }
    }
}

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

fn cleanup_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|pid| *pid > 1)
}

#[test]
fn sudo_shell_cleanup_rejects_special_and_overflow_pids() {
    for pid in [0, 1, u32::MAX, i32::MAX as u32 + 1] {
        assert_eq!(cleanup_pid(pid), None, "unsafe signal target {pid}");
    }
    assert_eq!(cleanup_pid(2), Some(2));
}

struct SiblingCleanup(Vec<ProcessIdentity>);
impl SiblingCleanup {
    fn new(pids: Vec<u32>) -> Self {
        Self(
            pids.into_iter()
                .filter_map(|pid| inspect(pid).ok().map(|p| p.0))
                .collect(),
        )
    }
}
impl Drop for SiblingCleanup {
    fn drop(&mut self) {
        for identity in &self.0 {
            // No process groups or special PIDs; recheck birth identity before
            // signaling only the exact descendants this fixture started.
            if let Some(pid) = cleanup_pid(identity.pid).filter(|_| identity.alive()) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
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
    let _children = SiblingCleanup::new(vec![target, sibling]);
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
                    std::fs::read_to_string(root.join("probe-frame.json"))
                        .unwrap_or_else(|_| {
                            serde_json::json!({"type":"request","request_id":"probe",
                            "command_id":"same-command-each-turn", "request":{"ListSessions":null}})
                            .to_string()
                        })
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
    let mut shutdown = ShellDrillShutdown(Some(stop));
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
    // The OS birth fence deliberately rejects same-tick births. On a fast
    // builder the fixture can reach its probe within the admission tick.
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(scratch_root.join("probe"), "probe").unwrap();
    let allowed: serde_json::Value =
        serde_json::from_str(&file(&scratch_root.join("result")).await).unwrap();
    assert!(allowed["error"].is_null(), "{allowed}");
    assert!(!allowed["response"]["SessionsListed"]["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
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
    // Settle through the actual completion seam, rather than cancellation.
    f.state
        .owned
        .complete_local_prompt_without_advance(
            session.id(),
            f.request.target_agent_id.as_deref().unwrap(),
            Some(f.run.id()),
        )
        .unwrap()
        .expect("completed sudo prompt");
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
    let _ = shutdown.0.take().unwrap().send(());
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
    let _children = SiblingCleanup::new(vec![old, new, grandchild]);
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

#[tokio::test]
async fn sudo_shell_interrupt_and_rotation_drop_process_authority() {
    for rotate in [false, true] {
        let f = fixture_with_provider(Some("while :; do sleep 1; done"));
        let turn = running(&f);
        let identity = root(&f);
        assert_eq!(f.state.sudo_for_peer(&identity).unwrap(), turn);
        if rotate {
            // The verifier uses the fixture's product-created encrypted Vault.
            let vault = f
                .state
                .owned
                .config_projection
                .snapshot()
                .user_config
                .credential_vault
                .path;
            f.state
                .change_vault_passphrase(
                    "local",
                    Path::new(&vault),
                    zeroize::Zeroizing::new(PASSKEY.into()),
                    zeroize::Zeroizing::new("rotated shell fixture".into()),
                )
                .await
                .unwrap();
        } else {
            let session = f
                .state
                .owned
                .session_store
                .get_session(&turn.session_id)
                .unwrap();
            f.state
                .owned
                .prompt_state_owner
                .begin_cancelling_active_prompt(&session, &turn.agent_id)
                .unwrap();
        }
        assert!(
            identity.alive(),
            "test must leave the provider process alive"
        );
        assert!(f.state.sudo_for_peer(&identity).is_err());
        assert!(!f.state.sudo_peer_live(&turn.entry_id, &identity));
    }
}

// MP-08/MP-10/MP-11 F1: Unix subscriptions share the window's owner/session fence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sudo_review_shell_subscriptions_are_owner_session_bound() {
    let scratch = ShellDrillScratch::new();
    let executable = std::env::current_exe().unwrap();
    let script = format!("while :; do if test -f '{0}/probe'; then rm '{0}/probe'; CHARIOX_SUDO_SHELL_PROBE_ROOT='{0}' '{1}' sudo_shell_cli_child --ignored; fi; sleep .02; done", scratch.path().display(), executable.display());
    let f = fixture_with_provider(Some(&script));
    let turn = running(&f);
    let other = crate::test_support::TestWorktree::new("sudo-other-session");
    let (other_session, guest, other_attachment) = {
        let mut app = f.app.lock().await;
        let mut sessions = crate::app::KernelSessionService::new(&mut app);
        let (session, _) = sessions.create_session(other.session_request()).unwrap();
        let attachment = sessions
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "other-owner",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let guest = sessions
            .attach(crate::attachment::AttachRequest::for_user(
                &turn.session_id,
                "guest",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "guest",
            ))
            .unwrap();
        (session, guest, attachment)
    };
    let socket = scratch.path().join("kernel.sock");
    let mut config = f.state.owned.config_projection.snapshot();
    config.local_socket_path = socket.clone();
    config.runtime_mcp_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    f.state.owned.config_projection.update(config);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let mut shutdown = ShellDrillShutdown(Some(stop));
    let server = tokio::spawn(
        crate::runtime_transport::run_kernel_websocket_server_with_router_on_listener(
            Arc::new(f.router.clone()),
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
    let mut results = Vec::new();
    for (label, session, attachment, allowed) in [
        (
            "own",
            turn.session_id.as_str(),
            f.request.attachment_id.as_str(),
            true,
        ),
        (
            "other session",
            other_session.id(),
            other_attachment.id(),
            false,
        ),
        (
            "guest attachment",
            turn.session_id.as_str(),
            guest.id(),
            false,
        ),
    ] {
        let frame = serde_json::json!({"type":"subscribe","request_id":"probe","session_id":session,"attachment_id":attachment});
        std::fs::write(scratch.path().join("probe-frame.json"), frame.to_string()).unwrap();
        std::fs::write(scratch.path().join("probe"), "probe").unwrap();
        let result: serde_json::Value =
            serde_json::from_str(&file(&scratch.path().join("result")).await).unwrap();
        results.push((label, result["error"].is_null(), allowed));
        std::fs::remove_file(scratch.path().join("result")).unwrap();
    }
    let _ = shutdown.0.take().unwrap().send(());
    server.await.unwrap().unwrap();
    for (label, actual, expected) in results {
        assert_eq!(
            actual, expected,
            "{label}: subscription owner/session fence"
        );
    }
}

// MP-08/MP-10/MP-11 F5: descendants belong to the bound turn that started them.
#[tokio::test]
async fn sudo_review_shell_cutoff_refreshes_for_each_bound_turn() {
    let scratch = ShellDrillScratch::new();
    let script = format!("while :; do for name in first next; do if test -f '{0}/start-'$name; then rm '{0}/start-'$name; sleep 300 & echo $! > '{0}/pid-'$name; fi; done; sleep .02; done", scratch.path().display());
    let f = fixture_with_provider(Some(&script));
    let mut turn = running(&f);
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(scratch.path().join("start-first"), "start").unwrap();
    let first = file(&scratch.path().join("pid-first"))
        .await
        .trim()
        .parse::<u32>()
        .unwrap();
    let mut children = SiblingCleanup::new(vec![first]);
    let first_identity = inspect(first).unwrap().0;
    assert!(f.state.sudo_for_peer(&first_identity).is_ok());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let owner = &f.state.owned.prompt_state_owner;
    let mut projected = session.clone();
    owner.project_into_session(&mut projected);
    owner.restore_session_state(&projected);
    assert!(
        f.state.sudo_for_peer(&first_identity).is_ok(),
        "same prompt restoration preserves the cutoff"
    );
    owner.hold_sudo_work(&session, &turn.agent_id, &turn.entry_id, "sudo-exact-turn");
    owner
        .cancel_active_prompt_only(&session, &turn.agent_id)
        .unwrap();
    assert!(owner.admit_sudo_work_prompt(
        &session,
        &turn.agent_id,
        &turn.entry_id,
        "sudo-next-turn"
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    let prompt = PromptQueueItem::new(
        "sudo-next-turn",
        &f.request.attachment_id,
        &turn.agent_id,
        "continue",
        PromptStatus::Queued,
    );
    owner
        .submit_prepared_prompt_with_queue_policy(&session, prompt, false, false)
        .unwrap();
    assert!(owner.bind_sudo_turn(&session, &turn.agent_id, "sudo-next-turn", &turn.entry_id));
    turn.prompt_id = Some("sudo-next-turn".into());
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(turn.entry_id.clone(), turn);
    let retained_allowed = f.state.sudo_for_peer(&first_identity).is_ok();
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(scratch.path().join("start-next"), "start").unwrap();
    let next = file(&scratch.path().join("pid-next"))
        .await
        .trim()
        .parse::<u32>()
        .unwrap();
    children.0.push(inspect(next).unwrap().0);
    assert!(f.state.sudo_for_peer(&inspect(next).unwrap().0).is_ok());
    assert!(
        !retained_allowed,
        "prior turn descendant must lose shell authority"
    );
}

// MP-08/MP-10/MP-11: a spawn launcher is not the endpoint-serving provider.
#[tokio::test]
async fn sudo_shell_wrapper_native_endpoint_preserves_turn_fence() {
    for (mode, runtime_completion) in [("spawn", true), ("spawn", false), ("exec", true)] {
        let scratch = ShellDrillScratch::new();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let script = scratch.path().join("wrapper.py");
        std::fs::write(
            &script,
            r#"
import os, socket, sys, time
root, port, mode = sys.argv[1:]
def wait(name):
    while not os.path.exists(root+'/'+name): time.sleep(.01)
def idle():
    while True: time.sleep(1)
if mode == 'spawn':
    child = os.fork()
    if child == 0: os.execl(sys.executable, sys.executable, __file__, root, port, 'native')
    idle()
sock = socket.socket()
sock.bind(('127.0.0.1', int(port)))
sock.listen()
old = os.fork()
if old == 0:
    sock.close()
    open(root+'/old', 'w').write(str(os.getpid()))
    wait('old-go')
    if os.fork() == 0:
        open(root+'/old-grandchild', 'w').write(str(os.getpid()))
    idle()
open(root+'/native', 'w').write(str(os.getpid()))
for name in ['first', 'next']:
    wait('start-'+name)
    child = os.fork()
    if child == 0:
        sock.close()
        open(root+'/pid-'+name, 'w').write(str(os.getpid()))
        idle()
idle()
"#,
        )
        .unwrap();
        let f = super::tests::fixture_with_run_endpoint(
            Some(&format!(
                "exec python3 '{}' '{}' {port} {mode}",
                script.display(),
                scratch.path().display()
            )),
            false,
            "dev-stub",
            "dev-stub",
            Some(format!("http://127.0.0.1:{port}")),
        );
        let native = file(&scratch.path().join("native"))
            .await
            .parse::<u32>()
            .unwrap();
        let old = file(&scratch.path().join("old"))
            .await
            .parse::<u32>()
            .unwrap();
        let mut children = SiblingCleanup::new(vec![native, old]);
        let started = crate::app::StartedProviderLaunch {
            run: f.run.clone(),
            previous_active_run_id: None,
            provider_credential_env: Default::default(),
        };
        if runtime_completion {
            f.state.finish_provider_launch(&started, None).await;
        } else {
            f.app
                .lock()
                .await
                .finish_provider_launch(&started, None)
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        let turn = running(&f);
        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::write(scratch.path().join("start-first"), "start").unwrap();
        std::fs::write(scratch.path().join("old-go"), "start").unwrap();
        let first = file(&scratch.path().join("pid-first"))
            .await
            .parse::<u32>()
            .unwrap();
        let grandchild = file(&scratch.path().join("old-grandchild"))
            .await
            .parse::<u32>()
            .unwrap();
        children.0.push(inspect(first).unwrap().0);
        children.0.push(inspect(grandchild).unwrap().0);
        let first_identity = inspect(first).unwrap().0;
        assert_eq!(
            f.state
                .sudo_for_peer(&first_identity)
                .expect("wrapper native tool child must have current-turn authority"),
            turn
        );
        assert_eq!(
            f.state.sudo_for_peer(&inspect(native).unwrap().0).unwrap(),
            turn
        );
        assert!(f.state.sudo_for_peer(&inspect(old).unwrap().0).is_err());
        assert!(f
            .state
            .sudo_for_peer(&inspect(grandchild).unwrap().0)
            .is_err());
        let session = f
            .state
            .owned
            .session_store
            .get_session(&turn.session_id)
            .unwrap();
        let owner = &f.state.owned.prompt_state_owner;
        owner.hold_sudo_work(&session, &turn.agent_id, &turn.entry_id, "sudo-exact-turn");
        owner
            .cancel_active_prompt_only(&session, &turn.agent_id)
            .unwrap();
        assert!(owner.admit_sudo_work_prompt(
            &session,
            &turn.agent_id,
            &turn.entry_id,
            "wrapper-next-turn"
        ));
        let prompt = PromptQueueItem::new(
            "wrapper-next-turn",
            &f.request.attachment_id,
            &turn.agent_id,
            "continue",
            PromptStatus::Queued,
        );
        owner
            .submit_prepared_prompt_with_queue_policy(&session, prompt, false, false)
            .unwrap();
        assert!(owner.bind_sudo_turn(
            &session,
            &turn.agent_id,
            "wrapper-next-turn",
            &turn.entry_id
        ));
        let mut next_turn = turn;
        next_turn.prompt_id = Some("wrapper-next-turn".into());
        f.state
            .owned
            .sudo_turns
            .lock()
            .unwrap()
            .insert(next_turn.entry_id.clone(), next_turn.clone());
        assert!(
            f.state.sudo_for_peer(&first_identity).is_err(),
            "retained child must remain denied"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::write(scratch.path().join("start-next"), "start").unwrap();
        let next = file(&scratch.path().join("pid-next"))
            .await
            .parse::<u32>()
            .unwrap();
        children.0.push(inspect(next).unwrap().0);
        assert_eq!(
            f.state.sudo_for_peer(&inspect(next).unwrap().0).unwrap(),
            next_turn
        );
    }
}
