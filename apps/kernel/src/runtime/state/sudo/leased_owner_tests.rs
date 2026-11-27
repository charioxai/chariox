//! Row 410: an agent leased to a worker kernel acts with the owner's authority on its home
//! kernel like a local one (row 405): no /sudo window, fence, card or passkey; payments and
//! secrets stay with the person; another account's leased agent gets nothing.
use super::forgotten_machine_tests::{forwarded_context, lease_agent_to_machine};
use super::tests::{fixture, fixture_for_owner, Fixture};
use super::*;
use serde_json::{json, Value};

const REQUEST: &str = "chariox_kernel_request";

/// The home router as the worker's forwarded call reaches it: the lease is admitted, and the
/// leased agent has no sudo window and no worker fence.
fn home_for_leased(f: &Fixture) -> crate::runtime::router::CommandRouter {
    let worker = crate::config::DaemonConfig::for_tests();
    crate::config::DaemonConfig::approve_remote_machine("worker-machine").unwrap();
    lease_agent_to_machine(f, &worker.daemon_id);
    f.router
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            &worker.daemon_id,
        ))
}

/// A home dispatch that succeeds is deep in an unoptimized build: run the cell on a 32 MiB stack.
fn on_big_stack<Fut: std::future::Future<Output = ()>>(
    cell: impl FnOnce() -> Fut + Send + 'static,
) {
    std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(cell());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn leased_call(
    f: &Fixture,
    router: &crate::runtime::router::CommandRouter,
    request: Value,
) -> Result<Value, DaemonError> {
    let worker = crate::config::DaemonConfig::for_tests();
    Box::pin(router.dispatch_forwarded_meta_runtime_tool_call(
        forwarded_context(f, &worker.daemon_id),
        REQUEST.into(),
        json!({ "request": request }),
    ))
    .await
    .map(|result| result.payload)
}

// Cell L1: ListProjects from a leased agent with no window: zero sudo windows, zero prompts.
#[test]
fn a_leased_agent_lists_projects_on_its_home_with_no_sudo_window() {
    crate::test_support::isolated_env_test!();
    on_big_stack(|| async {
        let f = fixture();
        let router = home_for_leased(&f);
        let listed = leased_call(
            &f,
            &router,
            json!({"ListProjects": {"include_archived": false}}),
        )
        .await;
        assert!(listed.is_ok(), "the leased agent was refused: {listed:?}");
        assert!(f.state.list_sudo_turns("local").is_empty());
        assert!(f.state.passkey_prompts_for("local").is_empty());
    });
}

// Cell L2: an ordinary owner action naming a second owner kernel is admitted for a leased
// agent as for a local one: the refusal, if any, is the missing relay, never sudo or ownership.
#[tokio::test]
async fn a_leased_agent_reaches_the_owners_other_kernel_without_sudo() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let router = home_for_leased(&f);
    let worker = crate::config::DaemonConfig::for_tests();
    let reached = Box::pin(router.dispatch_forwarded_meta_runtime_tool_call(
        forwarded_context(&f, &worker.daemon_id),
        REQUEST.into(),
        json!({"request": {"ListProjects": {"include_archived": false}}, "kernel": "second-kernel"}),
    ))
    .await;
    if let Err(error) = &reached {
        let text = error.to_string();
        assert!(
            !text.contains("sudo") && !text.contains("only agents of this kernel's owner"),
            "the leased agent was stopped at authority, not at the missing kernel: {text}"
        );
    }
    assert!(f.state.list_sudo_turns("local").is_empty());
}

// Cell L3: secrets and payments stay refused for a leased agent, with the reason.
#[tokio::test]
async fn a_leased_agent_is_still_refused_secrets_and_payments() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let router = home_for_leased(&f);
    let vault = leased_call(
        &f,
        &router,
        json!({"OwnerVault": {"action": "reveal", "id": "entry"}}),
    )
    .await;
    assert!(
        vault.is_err(),
        "a leased agent revealed a Vault entry: {vault:?}"
    );
    let rentals = leased_call(
        &f,
        &router,
        json!({"SetUserConfigValue": {"path": "agents.offload_without_asking", "value": "true"}}),
    )
    .await;
    assert!(
        rentals.is_err(),
        "a leased agent turned on machine rentals, which spend the owner's money: {rentals:?}"
    );
    for refused in [&vault, &rentals] {
        assert!(
            !refused.as_ref().unwrap_err().to_string().is_empty(),
            "a refusal names its reason"
        );
    }
}

// Cell L5: another account's agent leased to the same worker gets no owner authority.
#[tokio::test]
async fn another_accounts_leased_agent_gets_no_owner_authority() {
    crate::test_support::isolated_env_test!();
    let f = fixture_for_owner(None, false, "dev-stub", "dev-stub", None, "stranger");
    let router = home_for_leased(&f);
    let listed = leased_call(
        &f,
        &router,
        json!({"ListProjects": {"include_archived": false}}),
    )
    .await;
    assert!(
        matches!(&listed, Err(error) if error.to_string().contains("only agents of this kernel's owner")),
        "a stranger's leased agent acted for the owner, or without saying why: {listed:?}"
    );
}

// Cell L4: the worker keeps no state this needs. The home never pushed a worker fence, so a
// restarted worker has none to lose; its next run is the home's current one, and the run it
// replaced is refused.
#[test]
fn a_leased_agent_after_a_worker_restart_still_acts_and_the_old_run_does_not() {
    crate::test_support::isolated_env_test!();
    on_big_stack(|| async {
        let f = fixture();
        let router = home_for_leased(&f);
        let worker = crate::config::DaemonConfig::for_tests();
        let old_context = forwarded_context(&f, &worker.daemon_id);
        f.state
            .owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                f.request.target_agent_id.as_deref().unwrap(),
                Some("restarted-run".into()),
            )
            .unwrap();
        let args = json!({"request": {"ListProjects": {"include_archived": false}}});
        let refused = router
            .dispatch_forwarded_meta_runtime_tool_call(
                old_context.clone(),
                REQUEST.into(),
                args.clone(),
            )
            .await;
        assert!(
            refused.is_err(),
            "the run the restart replaced still acted: {refused:?}"
        );
        let mut current = old_context;
        current.worker_provider_run_id = "restarted-run".into();
        let listed = router
            .dispatch_forwarded_meta_runtime_tool_call(current, REQUEST.into(), args)
            .await;
        assert!(
            listed.is_ok(),
            "the restarted worker's run was refused: {listed:?}"
        );
        assert!(f.state.list_sudo_turns("local").is_empty());
    });
}

/// The home router as a forwarded call from a worker of ANOTHER account reaches it: the
/// lease and the peer key are admitted, but the peer's relay identity is not the owner's.
fn home_for_foreign_worker(f: &Fixture) -> crate::runtime::router::CommandRouter {
    let worker = crate::config::DaemonConfig::for_tests();
    crate::config::DaemonConfig::approve_remote_machine("worker-machine").unwrap();
    lease_agent_to_machine(f, &worker.daemon_id);
    let mut peer = crate::runtime::relay_peer_authority::test_peer_authority(&worker.daemon_id);
    peer.owners_kernel = false;
    f.router.with_relay_peer_authority(peer)
}

// Review P1 (L15 squash 1088804874): a worker run by another Cloud user cannot forge a leased
// agent's kernel request into the owner's authority: with no live /sudo window bound to the
// running home prompt it is refused, here and naming another of the owner's kernels.
#[test]
fn a_foreign_workers_forwarded_kernel_request_needs_the_window() {
    crate::test_support::isolated_env_test!();
    on_big_stack(|| async {
        let f = fixture();
        let router = home_for_foreign_worker(&f);
        let listed = leased_call(
            &f,
            &router,
            json!({"ListProjects": {"include_archived": false}}),
        )
        .await;
        assert!(
            matches!(&listed, Err(error) if error.to_string().contains("no sudo authority")),
            "another account's worker acted with the owner's authority: {listed:?}"
        );
        let worker = crate::config::DaemonConfig::for_tests();
        let gateway = Box::pin(router.dispatch_forwarded_meta_runtime_tool_call(
            forwarded_context(&f, &worker.daemon_id),
            REQUEST.into(),
            json!({"request": {"ListProjects": {"include_archived": false}}, "kernel": "second-kernel"}),
        ))
        .await;
        assert!(
            matches!(&gateway, Err(error) if error.to_string().contains("reaches only its home kernel")),
            "another account's worker reached the owner's other kernel: {gateway:?}"
        );
        assert!(f.state.list_sudo_turns("local").is_empty());
    });
}

// Review P2 (L15 squash 1088804874): an agent's SpawnAgent through chariox_kernel_request
// makes the agent's own child (child_finished comes back to it), not a root agent.
#[test]
fn an_agents_spawn_through_kernel_request_is_its_own_child() {
    crate::test_support::isolated_env_test!();
    on_big_stack(|| async {
        let f = super::tests::fixture_with_options(None, true);
        let parent = f.run.agent_instance_id().unwrap().to_string();
        let spawn = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            initial_task: None,
            peer_spawn: None,
            max_context_tokens: None,
            no_subscribe: false,
            session_id: f.request.session_id.clone(),
            alias: Some("reach405-child".into()),
            provider: Some("dev-stub".into()),
            account_profile: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        });
        for (alias, request) in [
            ("reach405-child", serde_json::to_value(&spawn).unwrap()),
            (
                "reach405-batch",
                json!({"SpawnAgents": {
                    "session_id": f.request.session_id,
                    "no_subscribe": true,
                    "agents": [{"alias": "reach405-batch", "provider": "dev-stub", "metaagent": false}]
                }}),
            ),
        ] {
            let result = Box::pin(f.router.dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                REQUEST,
                json!({ "request": request }),
            ))
            .await;
            assert!(
                result.as_ref().is_ok_and(|result| result.ok),
                "the owner's agent could not spawn: {result:?}"
            );
            let child = f
                .state
                .owned
                .agent_store
                .list_agents()
                .into_iter()
                .find(|agent| agent.alias() == Some(alias))
                .expect("the spawned agent");
            assert_eq!(
                child.parent_agent_id(),
                Some(parent.as_str()),
                "an agent's spawn through chariox_kernel_request became a root agent"
            );
        }
    });
}

// Review P1 (L15 squash 1088804874): the leased agent's reach identity itself. Under a call that
// another account's worker forwarded it resolves to nothing, whichever tool the call came
// through; under the owner's own worker it is the owner's.
#[test]
fn a_foreign_workers_call_resolves_no_leased_reach() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let worker = crate::config::DaemonConfig::for_tests();
    crate::config::DaemonConfig::approve_remote_machine("worker-machine").unwrap();
    lease_agent_to_machine(&f, &worker.daemon_id);
    let context = forwarded_context(&f, &worker.daemon_id);
    let run = crate::provider::projected_leased_provider_run_id(
        &context.leased_agent_id,
        &context.worker_provider_run_id,
    );
    let mut foreign = crate::runtime::relay_peer_authority::test_peer_authority(&worker.daemon_id);
    foreign.owners_kernel = false;
    let refused = f
        .state
        .with_relay_peer_authority(foreign)
        .reach_caller_for_run(&run);
    assert!(
        matches!(&refused, Err(error) if error.to_string().contains("only the owner's own workers")),
        "another account's worker resolved the owner's reach: {refused:?}"
    );
    let owners = f
        .state
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            &worker.daemon_id,
        ))
        .reach_caller_for_run(&run);
    assert!(
        owners.is_ok(),
        "the owner's own worker lost reach: {owners:?}"
    );
}
