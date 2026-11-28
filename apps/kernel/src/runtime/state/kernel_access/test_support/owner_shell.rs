//! Row 550: the shell of the owner's own terminal is the owner: what runs in it needs no access request.
//! A process the kernel launched for anything else still needs its own grant.
use super::*;

fn read_pid(path: &std::path::Path) -> Option<u32> {
    for _ in 0..100 {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

fn stop(pid: u32) {
    assert!(pid > 1, "refusing to signal pid {pid}");
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
}

#[tokio::test]
async fn a_program_in_the_owners_terminal_shell_is_covered_and_another_kernel_child_is_not() {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state();
    let dir = std::env::temp_dir().join(format!(
        "chariox-owner-shell-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // The terminal's shell is a child of the kernel (here: of the test), and runs a program.
    let mut shell = std::process::Command::new("sh")
        .args(["-c", "sleep 60 & echo $! > \"$D/inside\"; wait"])
        .env("D", &dir)
        .spawn()
        .unwrap();
    // A program the kernel launched for something else: an agent's tool, say.
    let mut elsewhere = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let inside = read_pid(&dir.join("inside")).expect("the shell started its program");
    // Let the program finish its exec, or its identity changes between two looks.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let identity = |pid| process::inspect(pid).unwrap().0;

    let covered = |pid| state.access_grants_for(&identity(pid)).unwrap();
    assert!(
        covered(inside).is_empty(),
        "before the owner's shell is covered it has no access"
    );

    state.admit_owner_shell_access(shell.id()).unwrap();
    let grants = covered(inside);
    assert_eq!(
        grants.len(),
        1,
        "the program in the shell has the shell's access"
    );
    assert!(grants[0]
        .summary
        .grant_id
        .starts_with(crate::runtime::kernel_access::OWNER_SHELL_GRANT_PREFIX));
    assert!(
        covered(elsewhere.id()).is_empty(),
        "a kernel child outside that shell needs its own grant"
    );
    assert!(
        covered(shell.id()).len() == 1,
        "the shell itself is covered"
    );

    stop(inside);
    shell.kill().unwrap();
    shell.wait().unwrap();
    elsewhere.kill().unwrap();
    elsewhere.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

async fn state_for_terminals() -> crate::runtime::state::KernelRuntimeState {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state()
}

/// What the kernel says about its own terminals, for a failure message.
async fn terminals_said(state: &crate::runtime::state::KernelRuntimeState) -> String {
    use crate::runtime::command::{KernelCaller, KernelCommandSource};
    let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    let listed = state
        .kernel_terminal_for_client(
            &caller,
            crate::local::KernelTerminalRequest {
                directory: None,
                operation: crate::local::RoomTerminalOperation::List,
            },
        )
        .await;
    format!("{listed:?}")
}

async fn open_shell(
    state: &crate::runtime::state::KernelRuntimeState,
    class: crate::local::KernelConnectionClass,
) -> String {
    use crate::runtime::command::{KernelCaller, KernelCommandSource};
    let caller =
        KernelCaller::for_source(&KernelCommandSource::LocalCli).with_connection_class(class);
    let opened = state
        .kernel_terminal_for_client(
            &caller,
            crate::local::KernelTerminalRequest {
                directory: Some(std::env::temp_dir().display().to_string()),
                operation: crate::local::RoomTerminalOperation::Open {
                    cols: 80,
                    rows: 24,
                    title: None,
                    command: None,
                    stdin: None,
                },
            },
        )
        .await
        .expect("the client opens a kernel terminal");
    let crate::local::LocalDaemonResponse::RoomTerminal {
        result: crate::local::RoomTerminalResult::Opened { terminal },
    } = opened
    else {
        panic!("unexpected {opened:?}");
    };
    terminal.terminal_id
}

/// The grants the kernel lists for owners' shells after its sweep: a grant bound to a wrapper that
/// then exec'd the shell would be gone by now.
fn shell_grants(state: &crate::runtime::state::KernelRuntimeState) -> usize {
    let owner = state.owned.config_projection.local_owner_user_id();
    state
        .list_kernel_access(&owner)
        .iter()
        .filter(|grant| {
            grant
                .grant_id
                .starts_with(crate::runtime::kernel_access::OWNER_SHELL_GRANT_PREFIX)
        })
        .count()
}

#[tokio::test]
async fn a_shell_the_owners_own_client_opened_is_covered() {
    let state = state_for_terminals().await;
    open_shell(&state, crate::local::KernelConnectionClass::Terminal).await;
    assert_eq!(shell_grants(&state), 1, "{}", terminals_said(&state).await);
}

#[tokio::test]
async fn a_shell_opened_through_an_agents_grant_is_not_the_owners() {
    let state = state_for_terminals().await;
    open_shell(&state, crate::local::KernelConnectionClass::ExternalAgent).await;
    assert_eq!(
        shell_grants(&state),
        0,
        "an agent's processes keep needing their own grant"
    );
}

#[tokio::test]
async fn the_owners_shell_grant_ends_with_its_shell() {
    let state = state_for_terminals().await;
    let terminal_id = open_shell(&state, crate::local::KernelConnectionClass::Terminal).await;
    assert_eq!(shell_grants(&state), 1);
    use crate::runtime::command::{KernelCaller, KernelCommandSource};
    state
        .kernel_terminal_for_client(
            &KernelCaller::for_source(&KernelCommandSource::LocalCli),
            crate::local::KernelTerminalRequest {
                directory: None,
                operation: crate::local::RoomTerminalOperation::Close { terminal_id },
            },
        )
        .await
        .expect("the owner closes the terminal");
    for _ in 0..100 {
        if shell_grants(&state) == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the grant of a shell that exited is still listed");
}
