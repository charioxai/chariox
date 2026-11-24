//! A linked-kernel directory request made under a live sudo window acts for the window's agent, and a window that
//! is not live or not the kernel owner's refuses it.
use super::tests::fixture_with_options;
use super::window_tests::open;
use super::*;
use crate::local::AccountKernelsRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};

fn command(entry_id: &str, request: AccountKernelsRequest) -> KernelCommand {
    let request = LocalDaemonRequest::CloudAccountKernels(request);
    let mut command = KernelCommand::from_local_request("account-kernels", None, None, &request);
    command.caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    command.caller.caller_id = entry_id.into();
    command
}

fn both(entry_id: &str) -> [KernelCommand; 2] {
    [
        command(entry_id, AccountKernelsRequest::List {}),
        command(
            entry_id,
            AccountKernelsRequest::Unlink {
                target_id: "target-1".into(),
            },
        ),
    ]
}

#[tokio::test]
async fn a_live_sudo_window_lists_and_unlinks_for_its_agent() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    for command in both(&window.entry_id) {
        assert_eq!(
            f.state.account_kernel_actor(&command),
            Some(window.agent_id.clone()),
            "an owner's live sudo window is the owner's authorization for the audit's agent"
        );
    }
}

#[tokio::test]
async fn a_sudo_identity_without_a_live_window_is_refused() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    for command in both("sudo:never-opened") {
        assert_eq!(f.state.account_kernel_actor(&command), None);
    }
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .remove(&window.entry_id);
    for command in both(&window.entry_id) {
        assert_eq!(
            f.state.account_kernel_actor(&command),
            None,
            "a window that ended gives no authority"
        );
    }
}
