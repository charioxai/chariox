use super::permission_prompt::{insert_run_for_request, rpc};
use super::*;

const TOKEN: &str = "claude-print-tool-loading-token";
const BUDGET: &str = include_str!("../../../../agent_context_budget.json");

/// The one list of upfront tools (`agent_upfront_tools.json`): what a Claude agent must have in
/// context before its first call, and what an agent another agent started has; every other
/// runtime tool is found through Claude Code's tool search.
fn core_tools(role: &str) -> Vec<String> {
    super::super::tool_loading::upfront_names("claude", role).to_vec()
}

fn always_load(tool: &Value) -> bool {
    tool["_meta"]["anthropic/alwaysLoad"] == true
}

fn model_visible_bytes(tool: &Value) -> usize {
    serde_json::json!({
        "name": tool["name"],
        "description": tool["description"],
        "inputSchema": tool["inputSchema"],
    })
    .to_string()
    .len()
}

async fn claude_catalog(sub_agent: bool) -> (Value, Vec<Value>) {
    provider_catalog("claude", "sonnet", sub_agent).await
}

async fn provider_catalog(provider: &str, model: &str, sub_agent: bool) -> (Value, Vec<Value>) {
    let root = std::env::temp_dir().join(format!(
        "chariox-tool-loading-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
        .expect("session should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        provider,
        provider,
        "default",
        model,
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::Chariox)
    .with_execution_mode(crate::provider::AgentExecutionMode::Build)
    .with_permission_level(crate::provider::AgentPermissionLevel::Required)
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        "http://127.0.0.1:1/mcp",
        TOKEN,
    ))
    .with_sub_agent(sub_agent);
    insert_run_for_request(&mut app, "provider-run-tool-loading", &request);
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let initialize = rpc(
        &router,
        TOKEN,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    )
    .await;
    let tools = rpc(&router, TOKEN, "tools/list", serde_json::json!({})).await["result"]["tools"]
        .as_array()
        .expect("tools should be an array")
        .clone();
    (initialize, tools)
}

#[tokio::test]
async fn claude_run_loads_its_core_tools_upfront_and_finds_the_rest_on_demand() {
    let (initialize, tools) = claude_catalog(false).await;
    let mut upfront: Vec<&str> = tools
        .iter()
        .filter(|tool| always_load(tool))
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    upfront.sort_unstable();
    assert_eq!(upfront, core_tools("agent"));

    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for name in &names {
        assert!(
            name.starts_with("chariox.") || !names.contains(&format!("chariox.{name}").as_str()),
            "{name} is listed next to its canonical twin"
        );
    }

    let instructions = initialize["result"]["instructions"]
        .as_str()
        .expect("the server must say that more tools exist and how to load them");
    assert!(instructions.contains("ToolSearch"), "{instructions}");
    assert!(instructions.len() <= 1_000, "{} bytes", instructions.len());
}

#[tokio::test]
async fn a_sub_agent_loads_only_its_inbox_tools_and_finds_the_rest_on_demand() {
    let (initialize, tools) = claude_catalog(true).await;
    let mut upfront: Vec<&str> = tools
        .iter()
        .filter(|tool| always_load(tool))
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    upfront.sort_unstable();
    assert_eq!(upfront, core_tools("sub_agent"));

    let (_, agent_tools) = claude_catalog(false).await;
    assert_eq!(
        tools.len(),
        agent_tools.len(),
        "a sub-agent keeps every tool; they load when it asks"
    );
    let instructions = initialize["result"]["instructions"]
        .as_str()
        .expect("a sub-agent is told how to load the other tools");
    assert!(instructions.contains("ToolSearch"), "{instructions}");
    assert!(
        instructions.contains("chariox.send_agent_message"),
        "{instructions}"
    );
    assert!(instructions.len() <= 1_000, "{} bytes", instructions.len());
    assert!(
        !instructions.contains("grouped by purpose"),
        "a worker is told about its own tools, not the whole catalog: {instructions}"
    );
    let vault_generate = tools
        .iter()
        .find(|tool| tool["name"] == "chariox.vault.generate")
        .expect("a tool the worker note does not name is still listed for ToolSearch");
    assert!(!always_load(vault_generate));
}

/// Budget for what a Claude agent's context pays for the runtime catalog. A
/// change that grows it must raise the committed number with its reason.
#[tokio::test]
async fn claude_runtime_catalog_matches_the_committed_context_budget() {
    let (initialize, tools) = claude_catalog(false).await;
    let (worker_initialize, worker_tools) = claude_catalog(true).await;
    let core: Vec<&Value> = tools.iter().filter(|tool| always_load(tool)).collect();
    let worker_core: Vec<&Value> = worker_tools
        .iter()
        .filter(|tool| always_load(tool))
        .collect();
    let measured = serde_json::json!({
        "claude_tools_listed": tools.len(),
        "claude_tools_list_json_bytes": tools.iter().map(model_visible_bytes).sum::<usize>(),
        "claude_core_tools": core.len(),
        "claude_core_json_bytes": core.iter().map(|tool| model_visible_bytes(tool)).sum::<usize>(),
        "claude_worker_core_tools": worker_core.len(),
        "claude_worker_core_json_bytes": worker_core
            .iter()
            .map(|tool| model_visible_bytes(tool))
            .sum::<usize>(),
        "claude_worker_server_instructions_bytes": worker_initialize["result"]["instructions"]
            .as_str()
            .map(str::len),
        "claude_worker_runtime_note_bytes": crate::prompt_assembly::bundled_prompt_template(
            "runtime/sub-agent-base",
        )
        .map(str::len),
        "claude_server_instructions_bytes": initialize["result"]["instructions"].as_str().map_or(0, str::len),
    });
    let budget: Value = serde_json::from_str(BUDGET).expect("budget file should be JSON");
    // The hidden runtime note is measured next to the prompt templates.
    let budget: serde_json::Map<String, Value> = budget
        .as_object()
        .expect("budget file should be an object")
        .iter()
        .filter(|(key, _)| key.starts_with("claude_"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let budget = Value::Object(budget);
    assert_eq!(
        measured, budget,
        "set apps/kernel/agent_context_budget.json to the measured values; before cutting text to fit, drill the tools it describes"
    );
}

/// Every provider is told each tool once, by its canonical name: a Codex or OpenCode run lists
/// the tools a Claude run does (Claude alone adds its permission prompt), with no bare twin.
/// The startup size per provider is printed for the record.
#[tokio::test]
async fn every_provider_lists_each_runtime_tool_once_by_its_canonical_name() {
    let names = |tools: &[Value]| -> std::collections::BTreeSet<String> {
        tools
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
    };
    let (_, claude) = provider_catalog("claude", "sonnet", false).await;
    let mut expected = names(&claude);
    expected.remove("chariox.permission_prompt");
    for (provider, model) in [
        ("codex", "gpt-6.2-sol"),
        ("opencode", "opencode-go/deepseek-v4.1-flash"),
    ] {
        for sub_agent in [false, true] {
            let (initialize, tools) = provider_catalog(provider, model, sub_agent).await;
            eprintln!(
                "STARTUP {provider} sub_agent={sub_agent} tools={} json_bytes={} instructions_bytes={}",
                tools.len(),
                tools.iter().map(model_visible_bytes).sum::<usize>(),
                initialize["result"]["instructions"].as_str().map_or(0, str::len),
            );
            let listed = names(&tools);
            assert_eq!(
                listed.len(),
                tools.len(),
                "{provider}: a tool is listed twice"
            );
            let twins: Vec<&String> = listed
                .iter()
                .filter(|name| {
                    !name.starts_with("chariox.") && listed.contains(&format!("chariox.{name}"))
                })
                .collect();
            assert!(
                twins.is_empty(),
                "{provider} sub_agent={sub_agent}: {twins:?}"
            );
            assert_eq!(listed, expected, "{provider} sub_agent={sub_agent}");
            assert!(
                tools.iter().all(|tool| !always_load(tool)),
                "{provider} has no deferred loading, so it marks nothing"
            );
        }
    }
}
