//! MP-08/MP-09/MP-10/MP-11 G06: forgetting a machine revokes the trust of its
//! worker leases. The owner's `/machine forget` ends the leased agent's sudo
//! window with a reason, and later calls through the lease are refused.
use super::tests::{fixture_with_options, Fixture};
use super::window_tests::{open, outcomes};
use super::*;
use crate::runtime::command::{KernelCommand, KernelCommandSource};

const MACHINE: &str = "worker-machine";

fn lease_agent_to_machine(f: &Fixture, worker_kernel_id: &str) {
    f.state
        .owned
        .agent_store
        .bind_remote_execution(
            f.request.target_agent_id.as_deref().unwrap(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_kernel_id.into(),
                worker_machine_id: MACHINE.into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
}

fn forwarded_context(
    f: &Fixture,
    worker_kernel_id: &str,
) -> crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
    crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: f.state.owned.config_projection.snapshot().daemon_id,
        home_session_id: f.request.session_id.clone(),
        home_agent_id: f.request.target_agent_id.clone().unwrap(),
        home_prompt_id: None,
        leased_agent_id: "leased-agent".into(),
        worker_kernel_id: worker_kernel_id.into(),
        worker_machine_id: MACHINE.into(),
        worker_provider_run_id: "worker-run".into(),
        worker_worktree_path: "/work".into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("/work".to_string()),
    }
}

async fn forget(f: &Fixture) {
    let request = LocalDaemonRequest::ForgetRemoteMachine(ForgetRemoteMachineRequest {
        machine_ref: MACHINE.into(),
    });
    let command = KernelCommand::from_local_request_with_caller(
        "forget-machine",
        KernelCommandSource::LocalCli,
        f.router
            .local_command_caller(
                KernelCommandSource::LocalCli,
                KernelConnectionClass::Terminal,
            )
            .await,
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::RemoteMachineForgotten { .. }
    ));
}

#[tokio::test]
async fn forgetting_the_worker_machine_ends_the_leased_sudo_window_and_refuses_later_calls() {
    crate::test_support::isolated_env_test!();
    let f = fixture_with_options(None, true);
    let worker = crate::config::DaemonConfig::for_tests();
    crate::config::DaemonConfig::approve_remote_machine(MACHINE, Some("amrig worker".into()))
        .unwrap();
    let window = open(&f, None).await;
    lease_agent_to_machine(&f, &worker.daemon_id);
    let state = f.state.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&worker.daemon_id),
    );
    let context = forwarded_context(&f, &worker.daemon_id);
    state
        .authorize_forwarded_workspace_context(&context)
        .expect("an approved machine's lease is admitted");
    assert_eq!(f.state.list_sudo_turns("local").len(), 1);

    forget(&f).await;

    assert!(
        f.state.list_sudo_turns("local").is_empty(),
        "the leased agent's sudo window ends with its lease"
    );
    assert!(outcomes(&f, &window.entry_id).contains(&"machine_forgotten".to_string()));
    let refusal = state
        .authorize_forwarded_workspace_context(&context)
        .expect_err("a call through a revoked lease is refused");
    assert!(
        refusal.to_string().contains("was forgotten by the owner"),
        "{refusal}"
    );
}
