//! Row 575: `chariox.privileged.run` through the real MCP router and real child
//! processes. A fake `sudo` stands in for the setuid program: it reads the
//! password from stdin the way `sudo -S` does, records how it was started, and
//! then runs the command. The real sudo is the live drill.
use super::tests::{fixture, Fixture};
use super::vault_tests::{call, names, unlock};
use super::*;
use serde_json::json;

// The public tool names, pinned literally so these tests also run on the base.
const RUN: &str = "chariox.privileged.run";
const CAPTURE: &str = "chariox.vault.capture_command";
const SECRET: &str = "pw-9f8e7d6c5b4a-sudo-canary";
const OTHER: &str = "pw-1a2b3c4d5e6f-other-canary";

fn workspace(f: &Fixture) -> std::path::PathBuf {
    f.run.working_directory().unwrap().to_path_buf()
}

fn machine(f: &Fixture) -> String {
    f.state.owned.config_projection.snapshot().daemon_id
}

fn origin(f: &Fixture) -> String {
    format!(
        "https://{}.machine.chariox.invalid",
        machine(f).to_lowercase()
    )
}

/// A sudo that accepts exactly `accepts`, echoes what it typed the way a
/// terminal would, records its argv and environment, and runs the command. An
/// empty `accepts` is a NOPASSWD rule: it never reads stdin.
fn fake_sudo(dir: &std::path::Path, accepts: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let asks = if accepts.is_empty() {
        String::new()
    } else {
        format!(
            "IFS= read -r typed\n\
             if [ \"$typed\" != '{accepts}' ]; then\n\
               echo 'Sorry, try again.' >&2\n\
               echo 'sudo: 1 incorrect password attempt' >&2\n\
               exit 1\n\
             fi\n\
             echo \"typed $typed\" >&2\n"
        )
    };
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{dir}/argv'\n\
         env > '{dir}/env'\n\
         {asks}\
         shift 5\n\
         exec \"$@\"\n",
        dir = dir.display()
    );
    let path = dir.join("sudo");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The owner's entry for this machine's sudo password.
async fn sudo_entry(f: &Fixture, request_id: &str, value: &str, origin: &str) -> String {
    let host = origin.trim_start_matches("https://").to_string();
    let made = call(
        f,
        CAPTURE,
        json!({"request_id": request_id, "host": host, "label": "sudo",
            "argv": ["/bin/sh", "-c", format!("printf '%s\\n' '{value}'")],
            "extract": {"kind": "stdout"},
            "use_binding": {"target": {"kind": "command_stdin", "origin": origin}}}),
    )
    .await
    .unwrap();
    made["credential_id"].as_str().unwrap().to_string()
}

fn with_fake_sudo(f: &Fixture, accepts: &str) -> std::path::PathBuf {
    let dir = workspace(f).join("fake-sudo");
    std::fs::create_dir_all(&dir).unwrap();
    fake_sudo(&dir, accepts);
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    dir
}

#[tokio::test]
async fn the_owners_agent_runs_a_command_with_sudo_and_never_sees_the_password() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    assert!(names(&f).iter().any(|name| name == RUN));
    let dir = with_fake_sudo(&f, SECRET);
    let handle = sudo_entry(&f, "sudo", SECRET, &origin(&f)).await;
    let arguments = json!({"credential_id": handle,
        "argv": ["/bin/sh", "-c", "echo ran; cat; exit 7"]});
    assert!(!arguments.to_string().contains(SECRET));
    let used = call(&f, RUN, arguments).await.unwrap();
    let mask = format!("[vault:{handle}]");
    assert_eq!(
        used,
        json!({"status": "exited", "exit_code": 7, "stdout": "ran\n",
            "stderr": format!("typed {mask}\n"), "machine": machine(&f),
            "ran_as": "root", "truncated": false})
    );
    assert!(!used.to_string().contains(SECRET));
    // Sudo got the password on stdin only: not in its words, not in its environment.
    let argv = std::fs::read_to_string(dir.join("argv")).unwrap();
    // The program that ends the command is an absolute path, found before sudo asks for anything.
    let words: Vec<_> = argv.lines().collect();
    assert_eq!(
        &words[..9],
        [
            "-S",
            "-k",
            "-p",
            "",
            "--",
            "sh",
            "-c",
            "exec \"$@\" </dev/null",
            "sh"
        ]
    );
    assert!(
        words[9].starts_with('/')
            && words[9]
                .rsplit('/')
                .next()
                .is_some_and(|name| name == "timeout" || name == "gtimeout"),
        "{argv}"
    );
    assert_eq!(&words[10..14], ["-k", "3", "120", "/bin/sh"]);
    let env = std::fs::read_to_string(dir.join("env")).unwrap();
    for text in [&argv, &env] {
        assert!(!text.contains(SECRET), "{text}");
    }
    let names: Vec<_> = env.lines().filter_map(|l| l.split('=').next()).collect();
    for name in &names {
        assert!(
            [
                "PATH",
                "LC_ALL",
                "LANGUAGE",
                "CHARIOX_AGENT_WAKE_ID",
                "PWD",
                "SHLVL",
                "_",
                "OLDPWD"
            ]
            .contains(name),
            "{name}"
        );
    }
    let turn = f.run.session_id().to_string();
    let session = serde_json::to_string(&f.state.owned.session_snapshot(&turn).unwrap()).unwrap();
    assert!(!session.contains(SECRET));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_wrong_password_is_refused_with_the_next_step_and_nothing_runs() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    with_fake_sudo(&f, SECRET);
    let handle = sudo_entry(&f, "wrong", OTHER, &origin(&f)).await;
    let marker = workspace(&f).join("must-not-run");
    let refused = call(
        &f,
        RUN,
        json!({"credential_id": handle, "argv": ["/usr/bin/touch", marker]}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(refused.contains("wrong_password"), "{refused}");
    assert!(
        refused.contains("Ask the owner to update the entry"),
        "{refused}"
    );
    for value in [SECRET, OTHER] {
        assert!(!refused.contains(value), "{refused}");
    }
    assert!(!marker.exists());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn an_entry_for_another_machine_or_another_machine_is_refused_before_any_run() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let dir = with_fake_sudo(&f, SECRET);
    let elsewhere = sudo_entry(
        &f,
        "else",
        SECRET,
        "https://other-kernel.machine.chariox.invalid",
    )
    .await;
    let refused = call(
        &f,
        RUN,
        json!({"credential_id": elsewhere, "argv": ["/bin/true"]}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(refused.contains("entry_wrong_machine"), "{refused}");
    assert!(refused.contains(&origin(&f)), "{refused}");
    let here = sudo_entry(&f, "here", SECRET, &origin(&f)).await;
    let refused = call(
        &f,
        RUN,
        json!({"credential_id": here, "argv": ["/bin/true"], "machine": "kernel-not-mine"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(refused.contains("machine_unknown"), "{refused}");
    for bad in [
        json!({"credential_id": "missing", "argv": ["/bin/true"]}),
        json!({"credential_id": here, "argv": []}),
        json!({"credential_id": here, "argv": ["/bin/true"], "timeout_seconds": 0}),
        json!({"credential_id": here, "argv": ["/bin/true"], "env": {"A": "b"}}),
    ] {
        assert!(call(&f, RUN, bad.clone()).await.is_err(), "{bad}");
    }
    // None of the refusals reached sudo.
    assert!(!dir.join("argv").exists());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_command_that_outlives_its_timeout_is_ended_inside_sudo() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    with_fake_sudo(&f, SECRET);
    let handle = sudo_entry(&f, "slow", SECRET, &origin(&f)).await;
    let started = std::time::Instant::now();
    let slow = call(
        &f,
        RUN,
        json!({"credential_id": handle, "argv": ["/bin/sleep", "30"], "timeout_seconds": 1}),
    )
    .await
    .unwrap();
    assert_eq!(slow["status"], "timed_out");
    assert!(started.elapsed() < Duration::from_secs(10));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_sudo_that_never_reads_the_password_still_gives_the_command_no_input() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    // A NOPASSWD rule (or a kernel that is already root): sudo leaves the pipe unread.
    with_fake_sudo(&f, "");
    let handle = sudo_entry(&f, "nopasswd", SECRET, &origin(&f)).await;
    let used = call(
        &f,
        RUN,
        json!({"credential_id": handle, "argv": ["/bin/cat"]}),
    )
    .await
    .unwrap();
    assert_eq!(used["stdout"], "", "{used}");
    assert_eq!(used["exit_code"], 0);
    assert!(!used.to_string().contains(SECRET));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_command_that_ignores_the_stop_request_is_still_reported_as_timed_out() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    with_fake_sudo(&f, SECRET);
    let handle = sudo_entry(&f, "stubborn", SECRET, &origin(&f)).await;
    let started = std::time::Instant::now();
    let slow = call(
        &f,
        RUN,
        json!({"credential_id": handle, "timeout_seconds": 1,
            "argv": ["/bin/sh", "-c", "trap '' TERM; exec sleep 30"]}),
    )
    .await
    .unwrap();
    assert_eq!(slow["status"], "timed_out", "{slow}");
    assert!(started.elapsed() < Duration::from_secs(12));
    // A command that kills itself at once is not a timeout.
    let killed = call(
        &f,
        RUN,
        json!({"credential_id": handle, "argv": ["/bin/sh", "-c", "kill -9 $$"]}),
    )
    .await
    .unwrap();
    assert_eq!(killed["status"], "exited", "{killed}");
    assert_eq!(killed["exit_code"], 137);
    f.state.shutdown_cleanup().await.unwrap();
}

#[test]
fn sudos_own_refusals_each_name_the_next_step_and_a_command_failure_passes_through() {
    use super::super::agent_process_watch::{KernelCommandExit, KernelCommandOutput};
    let said = |code: i32, stdout: &str, stderr: &str| KernelCommandOutput {
        exit: KernelCommandExit::Exited(code),
        stdout: zeroize::Zeroizing::new(stdout.as_bytes().to_vec()),
        stderr: zeroize::Zeroizing::new(stderr.as_bytes().to_vec()),
        truncated: false,
    };
    for (stderr, word, next) in [
        (
            "sudo: 3 incorrect password attempts",
            "wrong_password",
            "update the entry",
        ),
        (
            "lane is not in the sudoers file.",
            "not_in_sudoers",
            "adds that user to sudoers",
        ),
        (
            "Sorry, user lane is not allowed to execute '/bin/x' as root",
            "command_not_allowed",
            "widens the rule",
        ),
        (
            "sudo: sorry, you must have a tty to run sudo",
            "needs_terminal",
            "!requiretty",
        ),
        (
            "sudo: The \"no new privileges\" flag is set",
            "kernel_cannot_elevate",
            "NoNewPrivileges",
        ),
    ] {
        let refusal = refusal_for(&said(1, "", stderr)).unwrap();
        assert!(
            refusal.starts_with(word) && refusal.contains(next),
            "{refusal}"
        );
    }
    // A command that printed something, or failed on its own, is its own result.
    assert!(refusal_for(&said(1, "output", "sudo: 3 incorrect password attempts")).is_none());
    assert!(refusal_for(&said(2, "", "grep: no such file")).is_none());
    assert!(refusal_for(&said(0, "", "")).is_none());
}

fn refusal_for(output: &super::super::agent_process_watch::KernelCommandOutput) -> Option<String> {
    super::super::vault_command::sudo_refusal(output, "nelpx")
}

async fn owner(
    f: &Fixture,
    class: crate::local::KernelConnectionClass,
    request: crate::local::OwnerVaultRequest,
) -> Result<crate::local::LocalDaemonResponse, String> {
    let request = crate::local::LocalDaemonRequest::OwnerVault(request);
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-sudo",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(class);
    f.router
        .dispatch(command, request)
        .await
        .map_err(|error| error.to_string())
}

async fn owner_entry(f: &Fixture, value: &str) -> String {
    use crate::local::{LocalDaemonResponse, OwnerVaultRequest, OwnerVaultResult, VaultPassphrase};
    let created = owner(
        f,
        crate::local::KernelConnectionClass::Terminal,
        OwnerVaultRequest::Create {
            name: "sudo".into(),
            kind: "Command secret".into(),
            use_binding: crate::config::VaultUseBinding {
                target: crate::config::VaultUseTarget::CommandStdin { origin: origin(f) },
                value_kind: crate::config::VaultValueKind::Secret,
            },
            value: VaultPassphrase::new(value),
        },
    )
    .await
    .unwrap();
    let LocalDaemonResponse::OwnerVault {
        result: OwnerVaultResult::Saved { id },
    } = created
    else {
        panic!("owner creation failed")
    };
    id
}

#[tokio::test]
async fn the_owner_runs_the_same_request_and_gets_the_same_result() {
    use crate::local::{
        KernelConnectionClass, LocalDaemonResponse, OwnerVaultRequest, OwnerVaultResult,
    };
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    with_fake_sudo(&f, SECRET);
    let id = owner_entry(&f, SECRET).await;
    let run = |class, id: &str, argv: Vec<&str>| {
        owner(
            &f,
            class,
            OwnerVaultRequest::RunPrivileged {
                id: id.into(),
                argv: argv.into_iter().map(str::to_string).collect(),
                machine: None,
                timeout_seconds: None,
            },
        )
    };
    let LocalDaemonResponse::OwnerVault {
        result: OwnerVaultResult::PrivilegedRun { outcome, .. },
    } = run(
        KernelConnectionClass::Terminal,
        &id,
        vec!["/bin/sh", "-c", "echo ran; exit 7"],
    )
    .await
    .unwrap()
    else {
        panic!("owner run failed")
    };
    assert_eq!(
        serde_json::to_value(&outcome).unwrap(),
        json!({"status": "exited", "exit_code": 7, "stdout": "ran\n",
            "stderr": format!("typed [vault:{id}]\n"), "machine": machine(&f),
            "ran_as": "root", "truncated": false})
    );
    // The Vault's Activity shows the owner's use.
    let LocalDaemonResponse::OwnerVault {
        result: OwnerVaultResult::Activity { events, .. },
    } = owner(
        &f,
        KernelConnectionClass::Terminal,
        OwnerVaultRequest::Activity { id: id.clone() },
    )
    .await
    .unwrap()
    else {
        panic!("no activity")
    };
    assert!(events
        .iter()
        .any(|e| e.event.starts_with("Used by owner for sudo on")));
    // An agent's connection is not the owner's client: refused, nothing runs.
    let marker = workspace(&f).join("must-not-run");
    let marker_text = marker.display().to_string();
    for class in [
        KernelConnectionClass::ExternalAgent,
        KernelConnectionClass::KernelAgent,
    ] {
        let refused = run(class, &id, vec!["/usr/bin/touch", &marker_text])
            .await
            .unwrap_err();
        assert!(!refused.contains(SECRET), "{refused}");
    }
    assert!(!marker.exists());
    // A wrong password is the same refusal as for an agent.
    let wrong = owner_entry(&f, OTHER).await;
    let refused = run(KernelConnectionClass::Terminal, &wrong, vec!["/bin/true"])
        .await
        .unwrap_err();
    assert!(refused.contains("wrong_password"), "{refused}");
    for value in [SECRET, OTHER] {
        assert!(!refused.contains(value), "{refused}");
    }
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_privileged_run_cut_inside_the_password_returns_no_start_of_it() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    with_fake_sudo(&f, SECRET);
    let handle = sudo_entry(&f, "cut", SECRET, &origin(&f)).await;
    // The fake sudo writes "typed <password>\n" first; stderr is collected up to
    // 1 MiB and 12 bytes of the password the command prints fall before that limit.
    let typed = "typed \n".len() + SECRET.len();
    let before = (1 << 20) - 12 - typed;
    let (start, end) = SECRET.split_at(10);
    let script = format!(
        "head -c {before} /dev/zero | tr '\\0' x >&2; printf '%s%s' '{start}' '{end}' >&2; echo more >&2; exit 5"
    );
    let used = call(
        &f,
        RUN,
        json!({"credential_id": handle, "argv": ["/bin/sh", "-c", script]}),
    )
    .await
    .unwrap();
    assert_eq!(used["exit_code"], 5);
    assert_eq!(used["truncated"], true);
    let stderr = used["stderr"].as_str().unwrap();
    assert!(stderr.ends_with("xxx"), "{}", &stderr[stderr.len() - 40..]);
    assert!(!used.to_string().contains(&SECRET[..6]));
    let turn = f.run.session_id().to_string();
    let session = serde_json::to_string(&f.state.owned.session_snapshot(&turn).unwrap()).unwrap();
    assert!(!session.contains(&SECRET[..6]));
    f.state.shutdown_cleanup().await.unwrap();
}
