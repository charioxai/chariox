//! Row 551: only a running agent of this kernel's owner restarts a kernel through `chariox.kernels.restart`, this kernel
//! (no argument) or another kernel of the owner (`kernel`). An agent of another person running here, and a leased agent
//! whose run belongs to another kernel, are refused before anything restarts, with the next step.
use super::tests::{fixture, fixture_for_owner, Fixture};
use crate::local::LocalDaemonRequest;
use crate::room_exec::gateway_placement::{GrantedSend, SendFuture};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TOKEN: &str = "sudo-fixture-bearer";

/// Another kernel of the owner as the gateway reaches it in tests: it counts the restarts it was asked for.
struct SecondKernel(AtomicUsize);
impl GrantedSend for SecondKernel {
    fn send<'a>(&'a self, _command_id: String, request: &'a LocalDaemonRequest) -> SendFuture<'a> {
        assert!(matches!(request, LocalDaemonRequest::RestartKernel(_)));
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(serde_json::json!({"KernelRestarting": {"hot_reload": true}})) })
    }
}

fn second_kernel(name: &str) -> Arc<SecondKernel> {
    let kernel = Arc::new(SecondKernel(AtomicUsize::new(0)));
    super::super::job_compute::stand_in_kernels()
        .lock()
        .unwrap()
        .insert(name.into(), kernel.clone());
    kernel
}

fn leased(f: &Fixture) {
    let agent = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .agent_store
        .bind_remote_execution(
            agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease-1".into(),
                leased_agent_id: agent.into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: None,
                worker_worktree_id: None,
            },
        )
        .unwrap();
}

async fn refused(f: &Fixture, arguments: serde_json::Value) -> String {
    match f.state.kernel_restart_tool(TOKEN, arguments).await {
        Ok(result) => panic!("the restart was accepted: {}", result.payload),
        Err(error) => error.to_string(),
    }
}

fn says_who_and_what_next(text: &str) {
    assert!(
        text.contains("Only a running agent of this kernel's owner"),
        "the refusal says who may restart a kernel: {text}"
    );
    assert!(
        text.contains("Ask the owner"),
        "the refusal names the next step: {text}"
    );
}

#[tokio::test]
async fn an_agent_of_another_person_cannot_restart_this_kernel_or_another() {
    let f = fixture_for_owner(None, false, "dev-stub", "dev-stub", None, "someone-else");
    says_who_and_what_next(&refused(&f, serde_json::json!({})).await);
    let second = second_kernel("restart-551-foreign-second");
    says_who_and_what_next(
        &refused(
            &f,
            serde_json::json!({"kernel": "restart-551-foreign-second"}),
        )
        .await,
    );
    assert_eq!(
        second.0.load(Ordering::SeqCst),
        0,
        "no restart reached the other kernel"
    );
}

#[tokio::test]
async fn a_leased_agent_cannot_restart_this_kernel_or_another() {
    let f = fixture();
    leased(&f);
    says_who_and_what_next(&refused(&f, serde_json::json!({})).await);
    let second = second_kernel("restart-551-leased-second");
    says_who_and_what_next(
        &refused(
            &f,
            serde_json::json!({"kernel": "restart-551-leased-second"}),
        )
        .await,
    );
    assert_eq!(
        second.0.load(Ordering::SeqCst),
        0,
        "no restart reached the other kernel"
    );
}

#[tokio::test]
async fn the_owners_running_agent_still_restarts_this_kernel_and_another() {
    let f = fixture();
    // A test process was not started as a kernel program: its own hot reload cannot be prepared and says why,
    // which shows the request got past the owner check.
    let local = refused(&f, serde_json::json!({})).await;
    assert!(
        local.contains("hot reload") && local.contains("It keeps running"),
        "{local}"
    );
    let second = second_kernel("restart-551-owner-second");
    let result = f
        .state
        .kernel_restart_tool(
            TOKEN,
            serde_json::json!({"kernel": "restart-551-owner-second"}),
        )
        .await
        .unwrap();
    assert_eq!(result.payload["restarting"], true);
    assert_eq!(second.0.load(Ordering::SeqCst), 1);
}
