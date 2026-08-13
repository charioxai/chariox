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

fn activate_test_agent_meta_mode(
    app: &mut DaemonApp,
    agent: crate::agent::AgentInstance,
) -> crate::agent::AgentInstance {
    app.agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("test agent should enter meta mode")
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

#[tokio::test]
async fn runtime_mcp_advertises_meta_tools_only_to_metaagent_provider_runs() {
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

    let standard_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&standard_auth_token);
    assert!(
        standard_specs.iter().all(|spec| {
            spec.name != crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
        }),
        "standard agents must not see metaagent runtime tools"
    );

    let meta_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&meta_auth_token);
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL),
        "metaagents should see the metaagent runtime MCP tools"
    );
    for workflow_code_tool in [
        crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_READ_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_LIST_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_UPDATE_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_DELETE_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_EXPORT_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_IMPORT_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL,
    ] {
        assert!(
            meta_specs
                .iter()
                .any(|spec| spec.name == workflow_code_tool),
            "metaagents should see workflow-code runtime MCP tool `{workflow_code_tool}`"
        );
    }
    for workflow_registry_tool in [
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_LIST_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_GET_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_ADD_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_DELETE_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_LOAD_TOOL,
        crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_RUN_TOOL,
    ] {
        assert!(
            meta_specs
                .iter()
                .any(|spec| spec.name == workflow_registry_tool),
            "metaagents should see workflow registry runtime MCP tool `{workflow_registry_tool}`"
        );
    }
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL),
        "metaagents should see read-only workspace context tools"
    );
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL),
        "metaagents should see recall tools"
    );
    assert!(
        meta_specs
            .iter()
            .all(|spec| spec.name.starts_with("chariox.meta.")
                || spec.name == crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL
                || spec.name == crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL
                || spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL
                || spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                || spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL
                || spec.name == crate::transport::runtime_tools::QUERY_RECALL_TOOL),
        "metaagents should only see meta, agent collaboration, read-only workspace, and recall tools: {meta_specs:?}"
    );

    let denied_direct_tool = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            serde_json::json!({ "path": "README.md", "content_text": "nope" }),
        )
        .await
        .expect("metaagent direct mutation tools should return structured denials");
    assert!(
        !denied_direct_tool.ok
            && denied_direct_tool
                .payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| message.contains("not available to agents in Meta mode")),
        "{:?}",
        denied_direct_tool.payload
    );

    let canvas_contract = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("metaagent should read workflow-code canvas contract");
    assert!(canvas_contract.ok, "{:?}", canvas_contract.payload);
    assert_eq!(
        canvas_contract
            .payload
            .pointer("/canvas_contract/coordinate_space")
            .and_then(serde_json::Value::as_str),
        Some(crate::workflow_code::WORKFLOW_CODE_CANVAS_COORDINATE_SPACE)
    );
    assert_eq!(
        canvas_contract
            .payload
            .pointer("/canvas_contract/minimum_gap")
            .and_then(serde_json::Value::as_i64),
        Some(crate::workflow_code::WORKFLOW_CODE_CANVAS_MIN_GAP)
    );

    let denied = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &standard_auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect_err("standard agents should not be able to guess-call meta tools");
    assert!(
        denied
            .to_string()
            .contains("exactly one active provider run for an agent in Meta mode"),
        "{denied:?}"
    );
}

mod event_subscriptions;
mod events;
mod interactions;
mod overview_docs;
mod run_command_delegation;
mod run_command_lifecycle;
mod scoped_requests;
mod task_scope;
mod trace_projection;
mod workflow_code_crud;
mod workflow_code_patterns;
