//! Development standing grants through a real kernel and real requester processes: a holder
//! the owner listed in the config file gets its `until-exit` access without a popup.
use super::waiter::{file_text, grants, shell_with_waiter, shell_with_waiter_term};
use super::*;

fn list_holders(kernel: &Kernel, body: &str) {
    std::fs::create_dir_all(kernel.root.join("private")).unwrap();
    std::fs::write(kernel.root.join("private/config.toml"), body).unwrap();
}

fn entry(harness: &str, executable: &str) -> String {
    format!(
        "[[kernel_access.standing_holders]]\nharness = \"{harness}\"\nexecutable = \"{executable}\"\n"
    )
}

/// The kernel treats `holder` as the Claude harness, the way a real Claude Code is recognized.
async fn kernel_for(holder: &Path) -> Kernel {
    Kernel::start_with_env(false, false, &[("CHARIOX_CLAUDE_BIN", holder)]).await
}

fn executable_of(pid: u32) -> String {
    crate::runtime::kernel_access::process::inspect(pid)
        .unwrap()
        .0
        .executable
}

/// The executable a running `sh` has, which the OS may resolve elsewhere than
/// `/bin/sh` itself (macOS selects the shell binary), settled once its exec is over.
async fn running_sh() -> String {
    let mut shell = Command::new("sh")
        .args(["-c", "read line"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut executable = executable_of(shell.id());
    for _ in 0..20 {
        sleep(Duration::from_millis(50)).await;
        let current = executable_of(shell.id());
        if current == executable {
            break;
        }
        executable = current;
    }
    shell.kill().unwrap();
    shell.wait().unwrap();
    executable
}

async fn result_of(kernel: &Kernel) -> Value {
    let result = file_text(kernel.root.join("waiter-result")).await;
    for file in ["waiter-sent", "waiter-result"] {
        std::fs::remove_file(kernel.root.join(file)).unwrap();
    }
    serde_json::from_str(&result).unwrap()
}

// The test process is the holder: the request is granted with nobody answering, a client
// inside the holder is covered, `access revoke` ends the grant, and the same holder is
// granted again at its next request.
#[tokio::test]
async fn a_listed_holder_is_granted_without_a_popup_and_revoking_does_not_stop_the_next() {
    let me = std::env::current_exe().unwrap();
    let executable = executable_of(std::process::id());
    let mut kernel = kernel_for(&me).await;
    list_holders(&kernel, &entry("claude", &executable));
    let mut shell = shell_with_waiter(&kernel.root, &std::process::id().to_string());
    let granted = result_of(&kernel).await;
    let grant = &granted["response"]["KernelAccessGranted"]["grant"];
    assert_eq!(grant["holder_pid"], std::process::id(), "{granted}");
    assert_eq!(grant["term"], "until_holder_exits");
    assert_eq!(grants(&mut kernel).await.len(), 1);
    let mut covered = Client::start(&kernel.root);
    let state = covered.request(serde_json::json!({"GetSessionState": {"session_id": SESSION}}));
    assert!(state["error"].is_null(), "{state}");
    let revoked = kernel
        .request(serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":grant["grant_id"]}}))
        .await;
    assert!(revoked["error"].is_null(), "{revoked}");
    assert!(grants(&mut kernel).await.is_empty());
    let mut again_shell = shell_with_waiter(&kernel.root, &std::process::id().to_string());
    let again = result_of(&kernel).await;
    assert!(again["error"].is_null(), "{again}");
    assert_eq!(grants(&mut kernel).await.len(), 1);
    for shell in [&mut shell, &mut again_shell] {
        shell.kill().unwrap();
        shell.wait().unwrap();
    }
}

// The holder is a shell that is killed and started again (a new pid, the same executable):
// each one is granted without the owner.
#[tokio::test]
async fn a_restarted_listed_holder_is_granted_again() {
    let sh = running_sh().await;
    let mut kernel = kernel_for(Path::new(&sh)).await;
    list_holders(&kernel, &entry("claude", &sh));
    let mut pids = Vec::new();
    for _ in 0..2 {
        let mut shell = shell_with_waiter(&kernel.root, "parent");
        let granted = result_of(&kernel).await;
        assert_eq!(
            granted["response"]["KernelAccessGranted"]["grant"]["holder_pid"],
            shell.id(),
            "{granted}"
        );
        pids.push(shell.id());
        shell.kill().unwrap();
        shell.wait().unwrap();
    }
    assert_ne!(pids[0], pids[1]);
    let _ = grants(&mut kernel).await;
}

// Everything that is not an until-exit request of a listed harness and executable still
// goes to the owner's popup, and a malformed entry neither grants nor hides a valid one.
#[tokio::test]
async fn requests_outside_the_list_wait_for_the_owner() {
    let sh = running_sh().await;
    let sh = sh.as_str();
    let mut kernel = kernel_for(Path::new(sh)).await;
    let listed = entry("claude", sh);
    let minutes = serde_json::json!({"minutes": 30});
    let until_exit = serde_json::json!("until_holder_exits");
    let cases = [
        ("an empty list", String::new(), until_exit.clone()),
        ("a timed request", listed.clone(), minutes),
        (
            "another executable",
            entry("claude", "/not/the/holder"),
            until_exit.clone(),
        ),
        ("another harness", entry("codex", sh), until_exit.clone()),
        (
            "a malformed entry",
            "[[kernel_access.standing_holders]]\nharness = \"claude\"\n".into(),
            until_exit.clone(),
        ),
    ];
    for (name, config, term) in cases {
        list_holders(&kernel, &config);
        let mut shell = shell_with_waiter_term(&kernel.root, "parent", term);
        let id = kernel.access_prompt("-grant").await;
        let refused = kernel
            .request(serde_json::json!({"RespondToInteraction":{
                "session_id":"kernel-access", "interaction_id":id, "choice_id":"refuse"
            }}))
            .await;
        assert!(refused["error"].is_null(), "{name}: {refused}");
        let result = result_of(&kernel).await;
        assert!(result["error"].is_object(), "{name} was admitted: {result}");
        shell.kill().unwrap();
        shell.wait().unwrap();
    }
    list_holders(
        &kernel,
        &format!("[[kernel_access.standing_holders]]\nharness = \"claude\"\n\n{listed}"),
    );
    let mut shell = shell_with_waiter(&kernel.root, "parent");
    let granted = result_of(&kernel).await;
    assert!(
        granted["error"].is_null(),
        "a malformed entry hid a valid one: {granted}"
    );
    shell.kill().unwrap();
    shell.wait().unwrap();
    assert!(grants(&mut kernel).await.len() <= 1);
}
