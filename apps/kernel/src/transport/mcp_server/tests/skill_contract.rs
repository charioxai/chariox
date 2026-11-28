//! Row762: exercise the exported schema and authenticated dispatcher, without a provider process.
use super::permission_prompt::{insert_run_for_request, rpc};
use super::*;

const TOKEN: &str = "mcp-skill-contract-token";
const PROMPT: &str = "mcp-skill-contract-prompt";
const MANAGE: &str = crate::transport::runtime_tools::MANAGE_SKILLS_TOOL;

struct Fixture {
    router: Arc<CommandRouter>,
    prompt_id: String,
    _worktree: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new(provider: &str, model: &str) -> Self {
        let worktree = crate::test_support::TestWorktree::new("mcp-skill-contract");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "mcp-skill-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            provider,
            provider,
            "default",
            model,
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::Chariox)
        .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
            "http://127.0.0.1:1/mcp",
            TOKEN,
        ));
        insert_run_for_request(&mut app, "skill-contract-run", &request);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                crate::session::PromptQueueItem::new(
                    PROMPT,
                    attachment.id(),
                    agent.id(),
                    "Review skills",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
            panic!("the attached prompt must start before its skill calls");
        };
        let prompt_id = prompt.id().to_string();
        assert_eq!(prompt.status(), crate::session::PromptStatus::Running);
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            8,
        ));
        let snapshot = router
            .runtime_state()
            .session_snapshot_projection(session.id(), 0)
            .unwrap();
        let active = snapshot
            .session
            .active_prompt_for_agent(agent.id())
            .expect("the submitted prompt must hold the agent's turn authority");
        assert_eq!(active.id(), prompt_id);
        assert_eq!(active.status(), crate::session::PromptStatus::Running);
        Self {
            router,
            prompt_id,
            _worktree: worktree,
        }
    }

    async fn manage(&self, arguments: Value) -> Value {
        rpc(
            &self.router,
            TOKEN,
            "tools/call",
            serde_json::json!({"name":MANAGE,"arguments":arguments}),
        )
        .await
    }
}

#[tokio::test]
async fn mcp_skill_catalog_keeps_the_operation_union_below_the_provider_root() {
    let mut common_schema = None;
    for (provider, model) in [
        ("opencode", "opencode-go/claude-haiku-5-5"),
        ("opencode", "opencode-go/glm-5.3"),
        ("codex", "gpt-6.2-sol"),
        ("claude", "sonnet"),
    ] {
        let fixture = Fixture::new(provider, model);
        let catalog = rpc(&fixture.router, TOKEN, "tools/list", serde_json::json!({})).await;
        let tools = catalog["result"]["tools"].as_array().unwrap();
        let matching: Vec<_> = tools.iter().filter(|tool| tool["name"] == MANAGE).collect();
        assert_eq!(matching.len(), 1, "keep the canonical tool available once");
        let schema = &matching[0]["inputSchema"];
        assert_eq!(schema["type"], "object");
        // The captured Haiku HTTP400 identifies this exact provider constraint.
        for keyword in ["oneOf", "allOf", "anyOf"] {
            assert!(
                schema.get(keyword).is_none(),
                "row762: {provider}/{model} refuses root {keyword}; preserve it inside request"
            );
        }
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["request"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert!(
            schema["properties"]["request"]["properties"]["mutation"]["properties"]["actions"]
                ["items"]["oneOf"]
                .as_array()
                .is_some_and(|actions| actions.len() == 10)
        );
        let validator = jsonschema::JSONSchema::compile(schema).unwrap();
        assert!(validator.is_valid(&schema["examples"][0]));
        if let Some(common) = &common_schema {
            assert_eq!(
                schema, common,
                "all clients use the same operation contract"
            );
        } else {
            common_schema = Some(schema.clone());
        }
        let repeated = rpc(&fixture.router, TOKEN, "tools/list", serde_json::json!({})).await;
        assert_eq!(catalog, repeated, "relisting must not change the contract");
    }
}

#[tokio::test]
async fn mcp_skill_request_reaches_dispatch_and_retains_origin_and_source_guards() {
    let fixture = Fixture::new("opencode", "opencode-go/claude-haiku-5-5");
    let arguments = serde_json::json!({
        "origin_prompt_id":fixture.prompt_id.as_str(),
        "request":{"action":"resolve_repository","url":"file:///private/skills"},
    });
    let rejected_source = fixture.manage(arguments.clone()).await;
    assert!(
        rejected_source
            .to_string()
            .contains("only public https repositories"),
        "row762: nested request must reach the existing repository source guard: {rejected_source}"
    );
    let mut stale = arguments.clone();
    stale["origin_prompt_id"] = serde_json::json!("finished-turn");
    let rejected_origin = fixture.manage(stale).await;
    assert!(
        rejected_origin.to_string().contains("current prompt_id"),
        "{rejected_origin}"
    );
    assert!(!rejected_origin
        .to_string()
        .contains("only public https repositories"));

    for extra in ["mutation", "review_receipt_id", "package_id"] {
        let mut invalid = arguments.clone();
        invalid["request"][extra] = serde_json::json!("not accepted on this action");
        let rejected = fixture.manage(invalid).await;
        assert!(
            rejected
                .to_string()
                .contains("invalid manage_skills arguments"),
            "{rejected}"
        );
    }
}

#[tokio::test]
async fn mcp_skill_preview_issues_a_review_and_publish_rejects_unreviewed_bytes() {
    let fixture = Fixture::new("opencode", "opencode-go/claude-haiku-5-5");
    let mutation = serde_json::json!({
        "scope":{"kind":"chariox"},"expected_revision":0,
        "actions":[{"action":"create_package","package_id":"review-package","name":"Review package"}],
    });
    let preview = fixture
        .manage(serde_json::json!({
            "origin_prompt_id":fixture.prompt_id.as_str(),"request":{"action":"preview","mutation":mutation},
        }))
        .await;
    let reviewed = skill_success_payload(&preview);
    assert_eq!(reviewed["kind"], "preview");
    assert_eq!(reviewed["expected_revision"], 0);
    let receipt = reviewed["review_receipt"]["id"].as_str().unwrap();

    let mut changed = mutation.clone();
    changed["actions"][0]["name"] = serde_json::json!("Unreviewed change");
    let rejected_change = fixture
        .manage(serde_json::json!({
            "origin_prompt_id":fixture.prompt_id.as_str(),
            "request":{"action":"publish","mutation":changed,"review_receipt_id":receipt},
        }))
        .await;
    assert!(rejected_change.get("result").is_none(), "{rejected_change}");
    assert_eq!(rejected_change["error"]["code"], -32000);
    assert!(
        rejected_change
            .to_string()
            .contains("skill review does not match"),
        "{rejected_change}"
    );
    let rejected_receipt = fixture
        .manage(serde_json::json!({
            "origin_prompt_id":fixture.prompt_id.as_str(),
            "request":{"action":"publish","mutation":mutation,"review_receipt_id":"not-issued"},
        }))
        .await;
    assert!(
        rejected_receipt.get("result").is_none(),
        "{rejected_receipt}"
    );
    assert_eq!(rejected_receipt["error"]["code"], -32000);
    assert!(
        rejected_receipt
            .to_string()
            .contains("skill review is missing"),
        "{rejected_receipt}"
    );
    // A preview and refused publications leave the catalog revision available for another review.
    let repeated = fixture
        .manage(serde_json::json!({
            "origin_prompt_id":fixture.prompt_id.as_str(),"request":{"action":"preview","mutation":mutation},
        }))
        .await;
    let repeated = skill_success_payload(&repeated);
    assert_eq!(repeated["kind"], "preview");
    assert_eq!(repeated["expected_revision"], 0);
}

/// Successful skill calls deliberately carry one JSON text block, without a duplicate payload.
pub(super) fn skill_success_payload(response: &Value) -> Value {
    assert!(response.get("error").is_none(), "{response}");
    let result = response
        .get("result")
        .expect("skill call must have a result");
    assert!(result.get("structuredContent").is_none(), "{response}");
    assert!(result.get("isError").is_none(), "{response}");
    let content = result["content"].as_array().expect("MCP content array");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(content[0]["text"].as_str().expect("skill JSON text"))
        .expect("skill text must contain the actual JSON payload")
}
