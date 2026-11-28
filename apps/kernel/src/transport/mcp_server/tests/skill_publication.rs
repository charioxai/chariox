//! Row762: unchanged reviewed bytes publish through authenticated MCP and ordinary owner approval.
use super::permission_prompt::{insert_run_for_request, rpc};
use super::skill_contract::skill_success_payload;
use super::*;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;
use crate::skill::v2::{SkillsV2Request, SkillsV2Response};
use std::time::Duration;

const TOKEN: &str = "mcp-skill-publication-token";
const PROMPT: &str = "mcp-skill-publication-prompt";
const MANAGE: &str = crate::transport::runtime_tools::MANAGE_SKILLS_TOOL;

struct Fixture {
    router: Arc<CommandRouter>,
    prompt_id: String,
    session_id: String,
    agent_id: String,
    _worktree: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("mcp-skill-publication");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request().with_agent_defaults(
                crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                },
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "mcp-skill-owner-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "opencode-go/claude-haiku-5-5",
        )
        .with_agent_id(agent.id())
        .with_execution_mode(crate::provider::AgentExecutionMode::Build)
        .with_permission_level(crate::provider::AgentPermissionLevel::Required)
        .with_client_interface(crate::provider::ProviderClientInterface::Chariox)
        .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
            "http://127.0.0.1:1/mcp",
            TOKEN,
        ));
        insert_run_for_request(&mut app, "skill-publication-run", &request);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                crate::session::PromptQueueItem::new(
                    PROMPT,
                    attachment.id(),
                    agent.id(),
                    "Review this Skills change",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
            panic!("the attached publication prompt must start before its skill calls");
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
            .expect("the publication prompt must hold the agent's turn authority");
        assert_eq!(active.id(), prompt_id);
        assert_eq!(active.status(), crate::session::PromptStatus::Running);
        Self {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
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
            serde_json::json!({
                "name":MANAGE,"arguments":arguments,
            }),
        )
        .await
    }

    async fn client(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
        let command = KernelCommand::from_local_request(
            format!("skill-publication-client-{:016x}", rand::random::<u64>()),
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await
    }

    async fn revision(&self) -> u64 {
        let response = self
            .client(LocalDaemonRequest::SkillsV2(SkillsV2Request::Catalog {
                project_id: None,
                agent_id: None,
            }))
            .await
            .unwrap();
        let LocalDaemonResponse::SkillsV2 {
            result: SkillsV2Response::Catalog { revision, .. },
        } = response
        else {
            panic!("the public Catalog request must return its revision");
        };
        revision
    }

    async fn preview(&self, mutation: &Value) -> String {
        let result = self
            .manage(serde_json::json!({
                "origin_prompt_id":self.prompt_id.as_str(),"request":{"action":"preview","mutation":mutation},
            }))
            .await;
        let payload = skill_success_payload(&result);
        assert_eq!(payload["kind"], "preview");
        assert_eq!(payload["expected_revision"], mutation["expected_revision"]);
        let receipt = payload["review_receipt"]["id"].as_str().unwrap();
        assert!(!receipt.is_empty());
        receipt.into()
    }

    async fn asking(
        &self,
        arguments: Value,
    ) -> (
        tokio::task::JoinHandle<Value>,
        crate::session::RuntimeInteraction,
    ) {
        let router = self.router.clone();
        let mut call = tokio::spawn(async move {
            rpc(
                &router,
                TOKEN,
                "tools/call",
                serde_json::json!({"name":MANAGE,"arguments":arguments}),
            )
            .await
        });
        let interaction = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if call.is_finished() {
                    panic!(
                        "Required publication returned before asking the owner: {:?}",
                        (&mut call).await
                    );
                }
                let snapshot = self
                    .router
                    .runtime_state()
                    .session_snapshot_projection(&self.session_id, 0)
                    .unwrap();
                if let Some(interaction) = snapshot
                    .session
                    .active_interactions()
                    .iter()
                    .find(|interaction| interaction.title() == Some("Skills catalog change"))
                {
                    assert_eq!(interaction.agent_id(), Some(self.agent_id.as_str()));
                    assert_eq!(
                        interaction.kind(),
                        crate::session::RuntimeInteractionKind::Permission
                    );
                    break interaction.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("normal Required policy must publish a Skills permission interaction");
        (call, interaction)
    }

    async fn answer(
        &self,
        interaction: &crate::session::RuntimeInteraction,
        choice: &str,
    ) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
        self.client(LocalDaemonRequest::RespondToInteraction(
            crate::local::RespondToInteractionRequest {
                session_id: self.session_id.clone(),
                interaction_id: interaction.id().into(),
                choice_id: choice.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            },
        ))
        .await
    }
}

fn mutation() -> Value {
    serde_json::json!({"scope":{"kind":"chariox"},"expected_revision":0,
        "actions":[{"action":"create_package","package_id":"approval-package","name":"Approval package"}]})
}

fn publish(origin_prompt_id: &str, mutation: &Value, receipt: &str) -> Value {
    serde_json::json!({"origin_prompt_id":origin_prompt_id,
        "request":{"action":"publish","mutation":mutation,"review_receipt_id":receipt}})
}

async fn result(call: tokio::task::JoinHandle<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .expect("publication must settle after the owner's decision")
        .expect("MCP publication should join")
}

#[tokio::test]
async fn mcp_skill_reviewed_publication_asks_and_advances_the_revision_only_once() {
    let fixture = Fixture::new();
    let mutation = mutation();
    let receipt = fixture.preview(&mutation).await;
    let arguments = publish(&fixture.prompt_id, &mutation, &receipt);
    assert_eq!(fixture.revision().await, 0);
    let mut stale = arguments.clone();
    stale["origin_prompt_id"] = serde_json::json!("finished-turn");
    let refused_origin = fixture.manage(stale).await;
    assert!(refused_origin.get("result").is_none(), "{refused_origin}");
    assert_eq!(refused_origin["error"]["code"], -32000);
    assert!(
        refused_origin.to_string().contains("current prompt_id"),
        "{refused_origin}"
    );
    let mut changed = arguments.clone();
    changed["request"]["mutation"]["actions"][0]["name"] = serde_json::json!("Unreviewed change");
    let refused_change = fixture.manage(changed).await;
    assert!(refused_change.get("result").is_none(), "{refused_change}");
    assert_eq!(refused_change["error"]["code"], -32000);
    assert!(
        refused_change
            .to_string()
            .contains("skill review does not match"),
        "{refused_change}"
    );

    let (call, interaction) = fixture.asking(arguments.clone()).await;
    assert_eq!(fixture.revision().await, 0, "asking is not publication");
    fixture.answer(&interaction, "deny").await.unwrap();
    let denied = result(call).await;
    assert!(denied.get("error").is_none(), "{denied}");
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert_eq!(denied["result"]["structuredContent"]["published"], false);
    assert_eq!(
        denied["result"]["structuredContent"]["reason"],
        "permission_denied"
    );
    assert_eq!(fixture.revision().await, 0);

    let (call, interaction) = fixture.asking(arguments.clone()).await;
    fixture.answer(&interaction, "allow").await.unwrap();
    let published = result(call).await;
    let published = skill_success_payload(&published);
    assert_eq!(published["kind"], "published");
    assert_eq!(published["revision"], 1);
    assert_eq!(fixture.revision().await, 1);
    let repeated = fixture.manage(arguments).await;
    assert!(repeated.get("result").is_none(), "{repeated}");
    assert_eq!(repeated["error"]["code"], -32000);
    assert!(
        repeated.to_string().contains("catalog changed"),
        "{repeated}"
    );
    assert_eq!(
        fixture.revision().await,
        1,
        "a repeated old batch cannot publish twice"
    );
}

#[tokio::test]
async fn mcp_skill_late_answer_withdraws_approval_without_publishing() {
    let fixture = Fixture::new();
    let mutation = mutation();
    let receipt = fixture.preview(&mutation).await;
    let (call, interaction) = fixture
        .asking(publish(&fixture.prompt_id, &mutation, &receipt))
        .await;
    let completed = fixture
        .client(LocalDaemonRequest::CompletePrompt(
            crate::local::CompletePromptRequest {
                session_id: fixture.session_id.clone(),
            },
        ))
        .await
        .unwrap();
    let LocalDaemonResponse::PromptCompleted { completion } = completed else {
        panic!("completion must return the actual prompt receipt");
    };
    assert_eq!(completion.completed.id(), fixture.prompt_id);
    assert_eq!(
        completion.completed.status(),
        crate::session::PromptStatus::Completed
    );
    assert!(completion.started_next.is_none());
    let ended = public_session(&fixture).await;
    assert!(ended.active_prompt_for_agent(&fixture.agent_id).is_none());
    // Completion withdraws the prompt's RuntimeGrant; a late answer cannot revive it.
    let late_answer = fixture
        .answer(&interaction, "allow")
        .await
        .expect_err("an ended turn must not accept a late allow answer");
    match late_answer {
        crate::error::DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "interaction_closed");
            assert_eq!(message, "someone else answered or it expired");
        }
        other => panic!("a completed prompt's late answer must be closed: {other}"),
    }
    let refused = result(call).await;
    // Withdrawal sends a resolution with no choice, so this path is a denied tool result.
    assert!(refused.get("error").is_none(), "{refused}");
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert_eq!(refused["result"]["structuredContent"]["published"], false);
    assert_eq!(
        refused["result"]["structuredContent"]["reason"],
        "permission_denied"
    );
    assert_eq!(fixture.revision().await, 0);

    // A fresh MCP call with that ended origin instead returns the normal JSON-RPC error.
    let ended_origin = fixture
        .manage(publish(&fixture.prompt_id, &mutation, &receipt))
        .await;
    assert!(ended_origin.get("result").is_none(), "{ended_origin}");
    assert_eq!(ended_origin["error"]["code"], -32000);
    assert!(
        ended_origin["error"]["message"]
            .as_str()
            .unwrap()
            .contains("current prompt_id"),
        "{ended_origin}"
    );
    assert_eq!(fixture.revision().await, 0);
}

async fn public_session(fixture: &Fixture) -> crate::session::RuntimeSession {
    let response = fixture
        .client(LocalDaemonRequest::GetSessionState(
            crate::local::GetSessionStateRequest {
                session_id: fixture.session_id.clone(),
            },
        ))
        .await
        .unwrap();
    let LocalDaemonResponse::SessionState { session, .. } = response else {
        panic!("the public session request must return its current state");
    };
    session
}

#[tokio::test]
async fn mcp_skill_accepted_approval_rechecks_the_origin_before_publication() {
    let fixture = Fixture::new();
    let mutation = mutation();
    let receipt = fixture.preview(&mutation).await;
    // Keep the actual local MCP future owned here; it is never spawned independently.
    let mut call = Box::pin(rpc(
        &fixture.router,
        TOKEN,
        "tools/call",
        serde_json::json!({"name":MANAGE,"arguments":publish(&fixture.prompt_id, &mutation, &receipt)}),
    ));
    let interaction = tokio::time::timeout(
        Duration::from_secs(5),
        std::future::poll_fn(|context| {
            if let std::task::Poll::Ready(response) =
                std::future::Future::poll(call.as_mut(), context)
            {
                panic!("publication settled before its real owner decision: {response}");
            }
            let snapshot = fixture
                .router
                .runtime_state()
                .session_snapshot_projection(&fixture.session_id, 0)
                .unwrap();
            match snapshot
                .session
                .active_interactions()
                .iter()
                .find(|interaction| interaction.title() == Some("Skills catalog change"))
            {
                Some(interaction) => {
                    assert_eq!(interaction.agent_id(), Some(fixture.agent_id.as_str()));
                    assert_eq!(
                        interaction.kind(),
                        crate::session::RuntimeInteractionKind::Permission
                    );
                    std::task::Poll::Ready(interaction.clone())
                }
                None => std::task::Poll::Pending,
            }
        }),
    )
    .await
    .expect("the real publication future must register its Required permission card");

    // Do not poll `call` again until the public completion receipt and ended state below.
    let before_allow = public_session(&fixture).await;
    let active = before_allow
        .active_prompt_for_agent(&fixture.agent_id)
        .expect("approval must still belong to a running prompt");
    assert_eq!(active.id(), fixture.prompt_id);
    assert_eq!(active.status(), crate::session::PromptStatus::Running);
    let answered = fixture.answer(&interaction, "allow").await.unwrap();
    let LocalDaemonResponse::InteractionResponded {
        interaction_id,
        choice_id,
        outcome,
    } = answered
    else {
        panic!("the live public allow must return its actual answer receipt");
    };
    assert_eq!(interaction_id, interaction.id());
    assert_eq!(choice_id, "allow");
    assert_eq!(outcome, crate::local::InteractionOutcome::Answered);
    let after_allow = public_session(&fixture).await;
    let active = after_allow
        .active_prompt_for_agent(&fixture.agent_id)
        .expect("the public allow must be accepted while its prompt is live");
    assert_eq!(active.id(), fixture.prompt_id);
    assert_eq!(active.status(), crate::session::PromptStatus::Running);
    assert!(after_allow
        .active_interactions()
        .iter()
        .all(|remaining| remaining.id() != interaction.id()));
    assert_eq!(fixture.revision().await, 0);

    let completed = fixture
        .client(LocalDaemonRequest::CompletePrompt(
            crate::local::CompletePromptRequest {
                session_id: fixture.session_id.clone(),
            },
        ))
        .await
        .unwrap();
    let LocalDaemonResponse::PromptCompleted { completion } = completed else {
        panic!("the public completion must return its actual prompt receipt");
    };
    assert_eq!(completion.completed.id(), fixture.prompt_id);
    assert_eq!(
        completion.completed.status(),
        crate::session::PromptStatus::Completed
    );
    assert!(completion.started_next.is_none());
    let ended = public_session(&fixture).await;
    assert!(ended.active_prompt_for_agent(&fixture.agent_id).is_none());

    // The accepted resolution resumes only now, after its original prompt really ended.
    let refused = tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .expect("the original MCP future must settle after the accepted decision");
    assert!(refused.get("result").is_none(), "{refused}");
    assert_eq!(refused["error"]["code"], -32000);
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("current prompt_id"),
        "{refused}"
    );
    assert_eq!(fixture.revision().await, 0);
}
