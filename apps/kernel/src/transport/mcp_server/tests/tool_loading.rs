use super::permission_prompt::{insert_claude_run, rpc};
use super::*;

const TOKEN: &str = "claude-print-tool-loading-token";
const BUDGET: &str = include_str!("../../../../agent_context_budget.json");

/// What a Claude run must have in context before its first call; every other
/// runtime tool is found through Claude Code's tool search.
const CORE_TOOLS: [&str; 9] = [
    "chariox.events.ack",
    "chariox.events.blocked",
    "chariox.events.inbox",
    "chariox.events.process",
    "chariox.events.timer",
    "chariox.events.yield",
    "chariox.permission_prompt",
    "chariox.send_agent_message",
    "chariox.vault.use_env",
];

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

async fn claude_catalog() -> (Value, Vec<Value>) {
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
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-tool-loading",
        TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
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
    let (initialize, tools) = claude_catalog().await;
    let mut upfront: Vec<&str> = tools
        .iter()
        .filter(|tool| always_load(tool))
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    upfront.sort_unstable();
    assert_eq!(upfront, CORE_TOOLS);

    let instructions = initialize["result"]["instructions"]
        .as_str()
        .expect("the server must say that more tools exist and how to load them");
    assert!(instructions.contains("ToolSearch"), "{instructions}");
    assert!(instructions.len() <= 1_000, "{} bytes", instructions.len());
}

/// Budget for what a Claude agent's context pays for the runtime catalog. A
/// change that grows it must raise the committed number with its reason.
#[tokio::test]
async fn claude_runtime_catalog_matches_the_committed_context_budget() {
    let (_, tools) = claude_catalog().await;
    let core: Vec<&Value> = tools.iter().filter(|tool| always_load(tool)).collect();
    let measured = serde_json::json!({
        "claude_tools_listed": tools.len(),
        "claude_tools_list_json_bytes": tools.iter().map(model_visible_bytes).sum::<usize>(),
        "claude_core_tools": core.len(),
        "claude_core_json_bytes": core.iter().map(|tool| model_visible_bytes(tool)).sum::<usize>(),
    });
    let budget: Value = serde_json::from_str(BUDGET).expect("budget file should be JSON");
    assert_eq!(
        measured, budget,
        "set apps/kernel/agent_context_budget.json to the measured values; before cutting text to fit, drill the tools it describes"
    );
}
