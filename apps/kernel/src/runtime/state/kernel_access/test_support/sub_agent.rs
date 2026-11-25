//! A kernel child needs a dedicated agent identity to request access.
use super::*;
use std::process::{Command, Stdio};

const SUB_AGENT_SENTENCE: &str =
    "This process is outside a dedicated agent run of the kernel owner. Run `chariox access request --no-wait` from that agent’s shell.";

struct Launched(std::process::Child);

impl Drop for Launched {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// The test process stands in for the kernel, so its child is a process the kernel launched.
#[tokio::test]
async fn an_untracked_kernel_child_cannot_request_access() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-sub-agent");
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let child = Launched(
        Command::new("sleep")
            .arg("600")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let sub_agent = process::inspect(child.0.id()).unwrap().0;
    let refused = state
        .request_kernel_access(
            sub_agent,
            RequestKernelAccessRequest {
                reason: None,
                holder_pid: None,
                no_wait: false,
                term: None,
            },
        )
        .await
        .unwrap_err();
    let seen = crate::runtime_transport::unix_access::access_transport_error(&refused);
    assert_eq!(seen.code, "kernel_access_agent_unavailable", "{refused}");
    assert_eq!(seen.message, SUB_AGENT_SENTENCE);
    assert!(!seen.retryable);
}
