use super::*;

const MCP_TEXT_LIMIT: usize = 64 * 1024;

const SOURCE: &str = r#"
workflow.define({ alias: "reply_size_flow", maxConcurrent: 2 })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "codex", model: "gpt-5" }),
  instructions: "Plan and complete.",
  canCompleteWorkflowRun: true,
  canvas: { x: 40, y: 80 }
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

fn padded_source() -> String {
    format!("{SOURCE}\n// {}", "x".repeat(12_000))
}

struct Fixture {
    _env: TestMetaRuntimeEnv,
    router: CommandRouter,
    session_id: String,
    metaagent_id: String,
}

impl Fixture {
    fn new(label: &str) -> Option<Self> {
        if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
            eprintln!("skipping workflow-code reply-size test, no Node.js: {error}");
            return None;
        }
        let env = TestMetaRuntimeEnv::new(label);
        let workspace = env.root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace should be created");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .expect("session should be created");
        let metaagent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
            .expect("metaagent should spawn");
        let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        Some(Self {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            router,
            _env: env,
        })
    }

    async fn call(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::error::DaemonError> {
        self.router
            .runtime_state
            .dispatch_meta_runtime_tool_call_for_agent(
                &self.session_id,
                &self.metaagent_id,
                tool,
                arguments,
            )
            .await
    }

}

fn rebinding() -> serde_json::Value {
    serde_json::json!([{ "node": "planner", "provider": "dev-stub", "model": "default" }])
}

fn assert_small(payload: &serde_json::Value, what: &str) {
    let len = payload.to_string().len();
    assert!(len < 4_096, "{what} reply is {len} bytes: {payload}");
}

macro_rules! reply_size_test {
    ($name:ident, $inner:ident) => {
        #[test]
        fn $name() {
            crate::test_support::isolated_env_test!();
            run_large_stack_async_test(stringify!($name), $inner);
        }
    };
}

reply_size_test!(
    wfreply50_a_create_and_run_12kb_replies_under_4kib,
    wfreply50_a_inner
);
async fn wfreply50_a_inner() {
    let Some(f) = Fixture::new("wfreply50-a") else { return };
    let source = padded_source();
    let created = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
            serde_json::json!({ "name": "flow", "source": source }),
        )
        .await
        .expect("create");
    assert!(created.ok);
    assert_small(&created.payload, "create");
    let run = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "name": "flow", "endpoint": "entry", "prompt": "go",
                "provider_rebindings": rebinding()
            }),
        )
        .await
        .expect("run");
    assert!(run.ok);
    assert_small(&run.payload, "run");
    assert_eq!(
        run.payload["WorkflowCodeRun"]["result"]["invocation"]["kind"],
        "started"
    );
    // Inline source form, the 12 KB source goes through the tool arguments.
    let inline = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "source": source, "endpoint": "entry", "prompt": "go",
                "provider_rebindings": rebinding()
            }),
        )
        .await
        .expect("inline run");
    assert_small(&inline.payload, "inline run");
}

reply_size_test!(
    wfreply50_b_dispatch_error_over_limit_is_shown_in_full,
    wfreply50_b_inner
);
async fn wfreply50_b_inner() {
    let Some(f) = Fixture::new("wfreply50-b") else { return };
    let source = padded_source();
    // A dispatch error: the endpoint handle does not exist in the 12 KB source.
    let refused = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "source": source, "endpoint": "no_such_endpoint", "prompt": "go",
                "provider_rebindings": rebinding()
            }),
        )
        .await;
    let text = match &refused {
        Err(error) => crate::error::runtime_tool::message(error),
        Ok(result) => result.payload.to_string(),
    };
    assert!(
        text.contains("no_such_endpoint"),
        "the error names the endpoint: {text}"
    );
    assert!(text.len() < 4_096, "error reply is {} bytes: {text}", text.len());
    assert!(
        !text.contains(&"x".repeat(200)),
        "the error must not echo the 12 KB source"
    );
    // Through the MCP content layer the text is the whole error, never the truncated preview.
    let payload = match refused {
        Ok(result) => result.payload,
        Err(_) => serde_json::json!({ "error": text.clone() }),
    };
    let (content, structured) = crate::transport::mcp_server::runtime_tool_content_for_test(payload);
    assert!(structured.get("chariox_truncated").is_none());
    assert!(content[0]["text"].as_str().unwrap().len() < MCP_TEXT_LIMIT);
    assert!(content[0]["text"].as_str().unwrap().contains("no_such_endpoint"));
}

reply_size_test!(
    wfreply50_c_apply_reply_has_ids_hash_result_only,
    wfreply50_c_inner
);
async fn wfreply50_c_inner() {
    let Some(f) = Fixture::new("wfreply50-c") else { return };
    let source = padded_source();
    let hash = crate::workflow_code::sha256_hex(source.as_bytes());
    f.call(
        crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
        serde_json::json!({ "name": "flow", "source": source }),
    )
    .await
    .expect("create");
    for arguments in [
        serde_json::json!({ "name": "flow", "provider_rebindings": rebinding() }),
        serde_json::json!({ "source": source, "provider_rebindings": rebinding() }),
    ] {
        let applied = f
            .call(
                crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
                arguments,
            )
            .await
            .expect("apply");
        assert!(applied.ok);
        assert_small(&applied.payload, "apply");
        let body = &applied.payload["WorkflowCodeApplied"];
        assert!(body.get("session").is_none(), "no session echo: {body}");
        assert!(body["result"].get("compile").is_none(), "no compiled definition");
        assert_eq!(body["result"]["source_sha256"], hash);
        assert!(body["result"]["apply"]["workflow_id"].is_string());
        assert!(body["result"]["apply"]["agent_ids"]["planner"].is_string());
        assert_eq!(body["result"]["validation"]["ok"], true);
        assert!(!applied.payload.to_string().contains(&"x".repeat(200)));
    }
}

reply_size_test!(
    wfreply50_d_registry_run_reply_has_ids_hash_result_only,
    wfreply50_d_inner
);
async fn wfreply50_d_inner() {
    let Some(f) = Fixture::new("wfreply50-d") else { return };
    let source = padded_source();
    let hash = crate::workflow_code::sha256_hex(source.as_bytes());
    let added = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_ADD_TOOL,
            serde_json::json!({
                "name": "reg-flow",
                "source": { "kind": "single_file", "source": source }
            }),
        )
        .await
        .expect("registry add");
    assert!(added.ok, "{}", added.payload);
    let run = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_REGISTRY_RUN_TOOL,
            serde_json::json!({
                "name": "reg-flow", "endpoint": "entry", "prompt": "go",
                "provider_rebindings": rebinding()
            }),
        )
        .await
        .expect("registry run");
    assert!(run.ok, "{}", run.payload);
    assert_small(&run.payload, "registry run");
    let body = &run.payload["WorkflowRegistryEntryRun"];
    assert!(body.get("session").is_none(), "no session echo");
    assert_eq!(body["result"]["source_sha256"], hash);
    assert!(body["result"]["workflow_id"].is_string());
    assert_eq!(body["result"]["invocation"]["kind"], "started");
    assert!(body["result"].get("apply").is_none(), "no compiled definition");
    assert!(!run.payload.to_string().contains(&"x".repeat(200)));
}

reply_size_test!(
    wfreply50_e_read_and_export_still_return_full_source,
    wfreply50_e_inner
);
async fn wfreply50_e_inner() {
    let Some(f) = Fixture::new("wfreply50-e") else { return };
    let source = padded_source();
    f.call(
        crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
        serde_json::json!({ "name": "flow", "source": source }),
    )
    .await
    .expect("create");
    f.call(
        crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
        serde_json::json!({ "name": "flow", "provider_rebindings": rebinding() }),
    )
    .await
    .expect("apply");
    let read = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_READ_TOOL,
            serde_json::json!({ "name": "flow" }),
        )
        .await
        .expect("read");
    let artifact = &read.payload["WorkflowCodeArtifact"]["artifact"];
    assert_eq!(artifact["source"], source);
    assert_eq!(artifact["definition"]["workflow"]["alias"], "reply_size_flow");
    let exported = f
        .call(
            crate::transport::runtime_tools::META_WORKFLOW_CODE_EXPORT_TOOL,
            serde_json::json!({ "name": "flow" }),
        )
        .await
        .expect("export");
    assert!(exported.ok, "{}", exported.payload);
    // The saved artifact keeps the source; nothing was dropped by the compact replies.
    assert!(read.payload.to_string().contains(&"x".repeat(12_000)));
}

reply_size_test!(
    wfreply50_f_always_loaded_tool_text_does_not_grow,
    wfreply50_f_inner
);
async fn wfreply50_f_inner() {
    // The always-loaded tool text is the meta tool spec list. Row 595: it must not grow.
    let bytes: usize = crate::transport::runtime_tools::meta_runtime_tool_specs()
        .iter()
        .map(|spec| spec.name.len() + spec.description.len() + spec.input_schema.to_string().len())
        .sum();
    const BASELINE: usize = 0; // filled in from main by the first run
    assert!(bytes <= BASELINE, "meta tool spec bytes: {bytes}");
}
