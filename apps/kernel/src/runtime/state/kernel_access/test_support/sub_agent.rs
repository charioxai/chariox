//! A process the kernel launched (a sub-agent) cannot ask for kernel access. Nothing about
//! the refusal is secret, so it names the way out in plain words.
use super::*;
use std::process::{Command, Stdio};

const SUB_AGENT_SENTENCE: &str = "A sub-agent cannot ask for kernel access. Ask your parent agent.";

struct Launched(std::process::Child);

impl Drop for Launched {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// The test process stands in for the kernel, so its child is a process the kernel launched.
#[tokio::test]
async fn a_sub_agent_is_told_to_ask_its_parent_agent() {
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
                holder_pid: None,
                no_wait: false,
                term: None,
            },
        )
        .await
        .unwrap_err();
    let seen = crate::runtime_transport::unix_access::access_transport_error(&refused);
    assert_eq!(seen.code, "kernel_access_sub_agent", "{refused}");
    assert_eq!(seen.message, SUB_AGENT_SENTENCE);
    assert!(!seen.retryable);
}
