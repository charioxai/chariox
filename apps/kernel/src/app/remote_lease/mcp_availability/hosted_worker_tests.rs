use super::*;
use crate::slice::hosted_worker_test_support::*;

fn assert_materialization(machine: &str, worker: &str, slice: Option<&str>, expected: bool) {
    let environment = Environment::new(machine, worker, slice);
    std::env::set_var(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        environment.root.join("capabilities"),
    );
    let mut config = crate::config::DaemonConfig::for_tests();
    config.daemon_id = worker.into();
    config.host_machine_id = machine.into();
    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .unwrap();
    let leased = LeasedAgent::new(
        "leased".into(),
        "lease".into(),
        "home-agent".into(),
        "codex".into(),
        "default".into(),
        None,
        None,
        None,
        None,
        session.id().into(),
        agent.id().into(),
        "attachment".into(),
    );
    let definition =
        crate::mcp::CharioxMcpServerConfig::stdio("public-test-mcp", "/bin/true", Vec::new());
    let required = RequiredRemoteMcp {
        definition_hash: definition.definition_hash().unwrap(),
        config: definition.clone(),
    };
    let result = RemoteLeaseRuntime::new(&mut app)
        .ensure_required_remote_mcps_available(&leased, &[required]);
    let registry = crate::mcp::CharioxMcpRegistry::new(vec![
        crate::mcp::CharioxMcpRegistry::user_root().unwrap(),
    ]);
    let installed = registry.get("public-test-mcp").unwrap();
    assert_eq!(
        installed.as_ref() == Some(&definition),
        expected,
        "materialized definition"
    );
    assert_eq!(result.is_ok(), expected, "{result:?}");
}

#[test]
fn hosted_slice_context_mcp_materializes_isolated_definition_without_room() {
    assert_materialization(MACHINE, &canonical_worker(0), Some(SLICE), true);
}
#[test]
fn hosted_slice_context_mcp_preserves_private_synthetic_worker() {
    assert_materialization("slice:private", "private-worker", None, true);
}
#[test]
fn hosted_slice_context_mcp_does_not_copy_to_ordinary_worker_with_slice_alias() {
    assert_materialization(MACHINE, "ordinary-worker", Some(SLICE), false);
}
#[test]
fn hosted_slice_context_mcp_rejects_canonical_worker_bound_to_other_machine() {
    assert_materialization("machine-other", &canonical_worker(0), Some(SLICE), false);
}
