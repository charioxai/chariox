use super::*;

struct TestMetaRuntimeEnv {
    root: std::path::PathBuf,
}

impl TestMetaRuntimeEnv {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-m23-metaagent-runtime-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test meta runtime root should be created");
        Self { root }
    }
}

impl Drop for TestMetaRuntimeEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn node_supports_workflow_code_typescript(node: &std::path::Path) -> bool {
    std::process::Command::new(node)
        .arg("--no-warnings")
        .arg("--input-type=module")
        .arg("-e")
        .arg(
            "const mod = await import('node:module'); if (typeof mod.stripTypeScriptTypes !== 'function') process.exit(1)",
        )
        .status()
        .is_ok_and(|status| status.success())
}

fn mark_test_agent_controlled_by_metaagent(
    app: &mut DaemonApp,
    agent_id: &str,
    metaagent_id: &str,
) {
    app.agents_mut()
        .set_controlled_by_metaagent_id(agent_id, Some(metaagent_id.to_string()))
        .expect("test agent should exist");
}

fn run_large_stack_async_test<Fut>(name: &str, test: fn() -> Fut)
where
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("test runtime should build")
                .block_on(test());
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should not panic");
}

fn activate_test_agent_meta_mode(
    app: &mut DaemonApp,
    agent: crate::agent::AgentInstance,
) -> crate::agent::AgentInstance {
    app.agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("test agent should enter meta mode")
}

#[tokio::test]
async fn runtime_mcp_retires_meta_tools_for_regular_and_legacy_runs() {
    let env = TestMetaRuntimeEnv::new("tool-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let standard_run = launch_test_provider(
        &mut app,
        session.id(),
        standard_agent.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let standard_auth_token = standard_run
        .runtime_mcp_auth_token()
        .expect("standard run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    for auth in [&standard_auth_token, &meta_auth_token] {
        let specs = router.runtime_state.runtime_tool_specs_for_auth_token(auth);
        assert!(specs.iter().all(|spec| crate::transport::runtime_tools::canonical_meta_tool_name(&spec.name).is_none()));
        let denied = router.dispatch_authenticated_runtime_tool_call(
            auth, crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL, serde_json::json!({}),
        ).await.unwrap_err();
        assert!(denied.to_string().contains("retired"));
        assert!(denied.to_string().contains("/sudo"));
    }
}

// These exercise trusted legacy settlement/source-package validation, not the
// retired public MCP surface. Keep their common workflow regression coverage.
mod workflow_code_crud;
mod workflow_code_patterns;
