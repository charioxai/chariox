use std::collections::BTreeMap;
use std::path::Path;

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::config::WorkflowCodeLimitsConfig;
use crate::error::DaemonError;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::history::SessionHistoryEntry;
use crate::provider::{adapter_key_for_provider, AgentEndpointMode, ProviderRunState};
use crate::session::{
    CreateSessionRequest, RuntimeSession, SessionStateOwner, SessionStateReader, SessionStatus,
};
use crate::workflow_code::{
    compile_workflow_code_source_with_schema_import_root, WorkflowCodeAgentBinding,
    WorkflowCodeApplyReport, WorkflowCodeCompileAndApplyResult, WorkflowCodeCompileResult,
    WorkflowCodeDefinition, WorkflowCodeLanguage, WorkflowCodeValidationDiagnostic,
    WorkflowCodeValidationReport, WorkflowCodeValidationSeverity,
};

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

#[cfg(test)]
mod tests {
    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::WorkflowCodeLimitsConfig;
    use crate::extension::{ExtensionGrant, ExtensionKind};
    use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
    use crate::session::{
        CreateSessionRequest, SchedulerState, SessionStatus, WorkflowNodeRun,
        WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
    };
    use crate::workflow_code::{
        WorkflowCodeAgentBinding, WorkflowCodeAgentCreate, WorkflowCodeDefinition,
        WorkflowCodeEndpointDefinition, WorkflowCodeExistingAgent, WorkflowCodeNodeDefinition,
        WorkflowCodeProviderRebinding, WorkflowCodeQueueDefinition, WorkflowCodeWorkflow,
        WORKFLOW_CODE_PATTERN_EXAMPLES, WORKFLOW_CODE_SCHEMA_VERSION,
    };
    use crate::{DaemonApp, DaemonConfig};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    fn generated_workflow_code_definition() -> WorkflowCodeDefinition {
        WorkflowCodeDefinition {
            schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
            workflow: WorkflowCodeWorkflow {
                alias: Some("generated_agents".to_string()),
                prompt: None,
                flush_agent_context_before_run: Some(true),
                max_concurrent: Some(2),
                run_output_schema: None,
                intermediate_output_schema: None,
            },
            schemas: Vec::new(),
            nodes: vec![
                WorkflowCodeNodeDefinition {
                    handle: "planner".to_string(),
                    agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                        alias: Some("coded-planner".to_string()),
                        provider: "dev-stub".to_string(),
                        model: Some("default".to_string()),
                        effort: None,
                        account_profile: None,
                    }),
                    public_label: Some("Planner".to_string()),
                    instructions: Some("Plan.".to_string()),
                    can_complete_workflow_run: Some(false),
                    can_emit_intermediate_run_output: None,
                    wait_for_all_inputs: None,
                    intermediate_output_schema: None,
                    max_turns: None,
                    extensions: Vec::new(),
                    canvas: None,
                },
                WorkflowCodeNodeDefinition {
                    handle: "finisher".to_string(),
                    agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                        alias: Some("coded-finisher".to_string()),
                        provider: "dev-stub".to_string(),
                        model: Some("default".to_string()),
                        effort: None,
                        account_profile: None,
                    }),
                    public_label: Some("Finisher".to_string()),
                    instructions: Some("Finish.".to_string()),
                    can_complete_workflow_run: Some(true),
                    can_emit_intermediate_run_output: None,
                    wait_for_all_inputs: None,
                    intermediate_output_schema: None,
                    max_turns: None,
                    extensions: Vec::new(),
                    canvas: None,
                },
            ],
            edges: vec![crate::workflow_code::WorkflowCodeEdgeDefinition {
                handle: "planner_to_finisher".to_string(),
                from_node: "planner".to_string(),
                to_node: "finisher".to_string(),
                source_side: None,
                target_side: None,
                handoff_schema: None,
                validation_policy: None,
                canvas: None,
            }],
            endpoints: vec![WorkflowCodeEndpointDefinition {
                handle: "entry".to_string(),
                entry_node: "planner".to_string(),
                alias: Some("entry".to_string()),
                canvas: None,
            }],
            queues: Vec::new(),
            watchdogs: Vec::new(),
        }
    }

    fn existing_agent_workflow_code_definition(agent_id: &str) -> WorkflowCodeDefinition {
        let mut definition = generated_workflow_code_definition();
        definition.workflow.alias = Some("existing_agent".to_string());
        definition.nodes.truncate(1);
        definition.nodes[0].agent = WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
            agent_ref: agent_id.to_string(),
        });
        definition.edges.clear();
        definition
    }

    fn find_node_for_workflow_code_test() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("NODE") {
            candidates.push(PathBuf::from(path));
        }
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin/node"),
            PathBuf::from("/usr/local/bin/node"),
            PathBuf::from("/usr/bin/node"),
        ]);
        candidates.into_iter().find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn cache_test_provider_catalog(app: &mut DaemonApp) {
        app.cache_provider_catalog(OpenCodeProviderCatalog {
            all: vec![OpenCodeProviderInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                remote_machine_aliases: Vec::new(),
                models: BTreeMap::from([(
                    "gpt-5".to_string(),
                    OpenCodeProviderModel {
                        id: "gpt-5".to_string(),
                        name: "GPT-5".to_string(),
                        status: "available".to_string(),
                        limit: None,
                        variants: BTreeMap::new(),
                    },
                )]),
            }],
            default: BTreeMap::from([("codex".to_string(), "gpt-5".to_string())]),
            connected: vec!["codex".to_string()],
        });
    }

    fn unique_workflow_code_test_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "arroba-workflow-code-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test workspace should be created");
        path
    }

    fn install_test_skill(workspace: &std::path::Path, name: &str) {
        let skill_dir = workspace.join(".arroba").join("skills").join(name);
        fs::create_dir_all(&skill_dir).expect("skill dir should be created");
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Workflow-code test skill\n---\nUse this skill.\n"
            ),
        )
        .expect("skill should be written");
    }

    #[test]
    fn create_session_writes_durable_state_event() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let events = app
            .durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.created");
        assert_eq!(events[0].subject_id.as_deref(), Some(session.id()));
        assert_eq!(events[0].payload["session"]["id"], session.id());
        assert_eq!(events[0].payload["default_agent"]["id"], agent.id());
    }

    #[test]
    fn spawn_agent_and_end_session_write_durable_state_events() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let spawned = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("agent should spawn");
        crate::app::KernelSessionService::new(&mut app)
            .end_session(session.id())
            .expect("session should end");

        let events = app
            .durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load");
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["session.created", "agent.created", "session.ended"]
        );
        assert_eq!(events[1].subject_id.as_deref(), Some(spawned.id()));
        assert_eq!(events[2].subject_id.as_deref(), Some(session.id()));
    }

    #[test]
    fn workflow_code_apply_spawns_generated_agents_and_creates_workflow() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let definition = generated_workflow_code_definition();
        let report = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect("workflow-code should apply");

        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        let workflow = session
            .workflow(&report.workflow_id)
            .expect("workflow should exist");
        assert_eq!(workflow.nodes().len(), 2);
        assert_eq!(workflow.edges().len(), 1);
        assert_eq!(workflow.endpoints().len(), 1);
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 3);
        assert_eq!(report.agent_ids.len(), 2);
        assert!(report.queue_ids.contains_key("default"));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "default_queue_created"));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "canvas_auto_layout_applied"));

        let events = app
            .durable_state_store()
            .load_events_after(0)
            .expect("durable events should load");
        assert!(events
            .iter()
            .any(|event| event.kind == "workflow_code.applied"
                && event.subject_id.as_deref() == Some(report.workflow_id.as_str())));
    }

    #[test]
    fn workflow_code_apply_rebinds_generated_agent_provider() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let definition = generated_workflow_code_definition();
        let report = app
            .apply_workflow_code_definition_with_rebindings(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
                &[WorkflowCodeProviderRebinding {
                    node: "planner".to_string(),
                    provider: "opencode".to_string(),
                    model: Some("qwen3-coder".to_string()),
                    effort: Some("medium".to_string()),
                    account_profile: Some("profile-a".to_string()),
                }],
            )
            .expect("workflow-code should apply with provider rebinding");

        let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
        let planner = app
            .agents()
            .get_agent(planner_agent_id)
            .expect("planner agent should exist");
        assert_eq!(planner.provider(), "opencode");
        assert_eq!(planner.model(), Some("qwen3-coder"));
        assert_eq!(planner.effort(), Some("medium"));
        assert_eq!(planner.account_profile(), Some("profile-a"));
    }

    #[test]
    fn workflow_code_apply_rejects_unavailable_generated_agent_provider() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
            agent.provider = "missing-provider".to_string();
        }

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("workflow-code should reject unavailable generated-agent provider");

        let message = format!("{error}");
        assert!(message.contains("node `planner` requests unavailable provider `missing-provider`"));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    }

    #[test]
    fn workflow_code_apply_rebinding_can_replace_unavailable_provider() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
            agent.provider = "missing-provider".to_string();
        }

        let report = app
            .apply_workflow_code_definition_with_rebindings(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
                &[WorkflowCodeProviderRebinding {
                    node: "planner".to_string(),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }],
            )
            .expect("workflow-code should apply after rebinding unavailable provider");

        let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
        let planner = app
            .agents()
            .get_agent(planner_agent_id)
            .expect("planner agent should exist");
        assert_eq!(planner.provider(), "dev-stub");
    }

    #[test]
    fn workflow_code_apply_rejects_unavailable_generated_agent_model_from_catalog() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        cache_test_provider_catalog(&mut app);
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
            agent.provider = "codex".to_string();
            agent.model = Some("missing-model".to_string());
        }

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("workflow-code should reject unavailable generated-agent model");

        let message = format!("{error}");
        assert!(message.contains("unavailable_model"));
        assert!(message.contains("missing-model"));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    }

    #[test]
    fn workflow_code_apply_rebinding_can_replace_unavailable_model() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        cache_test_provider_catalog(&mut app);
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
            agent.provider = "codex".to_string();
            agent.model = Some("missing-model".to_string());
        }

        let report = app
            .apply_workflow_code_definition_with_rebindings(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
                &[WorkflowCodeProviderRebinding {
                    node: "planner".to_string(),
                    provider: "codex".to_string(),
                    model: Some("gpt-5".to_string()),
                    effort: None,
                    account_profile: None,
                }],
            )
            .expect("workflow-code should apply after rebinding unavailable model");

        let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
        let planner = app
            .agents()
            .get_agent(planner_agent_id)
            .expect("planner agent should exist");
        assert_eq!(planner.provider(), "codex");
        assert_eq!(planner.model(), Some("gpt-5"));
    }

    #[test]
    fn workflow_code_apply_rejects_runtime_queue_limit_before_spawning_agents() {
        let mut config = DaemonConfig::for_tests();
        config.user_config.workflow.max_queues_per_workflow = Some(1);
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let mut definition = generated_workflow_code_definition();
        definition.queues.push(WorkflowCodeQueueDefinition {
            handle: "urgent".to_string(),
            alias: "urgent".to_string(),
            priority: 5,
            enabled: true,
        });
        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("runtime queue limit should reject before spawning generated agents");

        let message = format!("{error:?}");
        assert!(message.contains("limit_exceeded"), "{message}");
        assert!(
            message.contains("runtime workflow queue limit 1"),
            "{message}"
        );
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
        let session_after = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(!session_after
            .workflows()
            .iter()
            .any(|workflow| workflow.alias() == Some("generated_agents")));
    }

    #[test]
    fn workflow_code_apply_rejects_exhausted_alias_allocation_before_spawning_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        for attempt in 0..crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS {
            let alias = if attempt == 0 {
                "generated_agents".to_string()
            } else {
                format!("generated_agents-{}", attempt + 1)
            };
            app.session_state_store()
                .write()
                .create_workflow(session.id(), Some(alias))
                .expect("workflow alias candidate should be created");
        }
        let agent_count_before = app.agents().get_session_agents(session.id()).len();
        let workflow_count_before = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist")
            .workflows()
            .len();

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &generated_workflow_code_definition(),
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("workflow-code should reject exhausted alias allocation before spawning");

        let message = format!("{error}");
        assert!(message.contains("workflow_alias_unavailable"), "{message}");
        assert_eq!(
            app.agents().get_session_agents(session.id()).len(),
            agent_count_before
        );
        let session_after = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert_eq!(session_after.workflows().len(), workflow_count_before);
    }

    #[test]
    fn workflow_code_apply_rejects_missing_node_extension_requirement() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        definition.nodes[0].extensions.push(ExtensionGrant::new(
            ExtensionKind::Skill,
            "missing-workflow-code-skill",
        ));

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("workflow-code should reject missing extension requirements");

        let message = format!("{error}");
        assert!(message.contains(
            "node `planner` extension requirement `skill:missing-workflow-code-skill` cannot be satisfied"
        ));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    }

    #[test]
    fn workflow_code_apply_preflights_existing_agent_authorization_without_partial_mutation() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let metaagent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
            .expect("metaagent should spawn");
        let metaagent = app
            .agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("agent should enter meta mode");
        let peer_worker = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
            .expect("peer worker should spawn");
        let agent_count_before = app.agents().get_session_agents(session.id()).len();

        let mut definition = generated_workflow_code_definition();
        definition.workflow.alias = Some("partial_mutation_guard".to_string());
        definition.nodes[1].agent = WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
            agent_ref: peer_worker.id().to_string(),
        });

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                metaagent.owner_user_id().to_string(),
                Some(metaagent.id().to_string()),
            )
            .expect_err("workflow-code should reject unauthorized existing-agent binding");

        let message = format!("{error}");
        assert!(message.contains("unauthorized_existing_agent_binding"));
        assert!(message.contains(peer_worker.id()));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(
            app.agents().get_session_agents(session.id()).len(),
            agent_count_before
        );
    }

    #[test]
    fn workflow_code_apply_rejects_metaagent_as_existing_node_agent_without_partial_mutation() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let metaagent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
            .expect("metaagent should spawn");
        let metaagent = app
            .agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("agent should enter meta mode");
        let agent_count_before = app.agents().get_session_agents(session.id()).len();
        let definition = existing_agent_workflow_code_definition(metaagent.id());

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("workflow-code should reject metaagent as workflow node agent");

        let message = format!("{error}");
        assert!(message.contains("invalid_existing_agent_binding"));
        assert!(message.contains(metaagent.id()));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(
            app.agents().get_session_agents(session.id()).len(),
            agent_count_before
        );
    }

    #[test]
    fn workflow_code_apply_grants_satisfied_node_extension_requirement() {
        let workspace = unique_workflow_code_test_workspace("extension-satisfied");
        install_test_skill(&workspace, "workflow-code-skill");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.display().to_string(),
                "worktree",
            ))
            .expect("session should create");
        let mut definition = generated_workflow_code_definition();
        definition.nodes[0].extensions.push(ExtensionGrant::new(
            ExtensionKind::Skill,
            "workflow-code-skill",
        ));

        let report = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect("workflow-code should apply when extension requirement exists");

        let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
        let planner = app
            .agents()
            .get_agent(planner_agent_id)
            .expect("planner agent should exist");
        assert!(planner
            .extension_grants()
            .iter()
            .any(|grant| grant.matches(&ExtensionKind::Skill, "workflow-code-skill")));
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn workflow_code_apply_grants_extensions_to_authorized_existing_agent() {
        let workspace = unique_workflow_code_test_workspace("existing-extension-satisfied");
        install_test_skill(&workspace, "workflow-code-skill");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.display().to_string(),
                "worktree",
            ))
            .expect("session should create");
        let existing_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
            .expect("existing worker should spawn");
        let mut definition = existing_agent_workflow_code_definition(existing_agent.id());
        definition.nodes[0].extensions.push(ExtensionGrant::new(
            ExtensionKind::Skill,
            "workflow-code-skill",
        ));

        let report = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect("workflow-code should grant extensions to an existing bound agent");

        assert_eq!(
            report.agent_ids.get("planner").map(String::as_str),
            Some(existing_agent.id())
        );
        let existing_agent = app
            .agents()
            .get_agent(existing_agent.id())
            .expect("existing worker should still exist");
        assert!(existing_agent
            .extension_grants()
            .iter()
            .any(|grant| grant.matches(&ExtensionKind::Skill, "workflow-code-skill")));
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn workflow_code_javascript_compile_and_apply_creates_generated_workflow() {
        let Some(node_path) = find_node_for_workflow_code_test() else {
            eprintln!("skipping workflow-code JS apply test because node is not available");
            return;
        };
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let source = r#"
workflow.define({ alias: "js_coded_flow", maxConcurrent: 2 })
const planner = workflow.node({
  agent: workflow.newAgent({ alias: "js-planner", provider: "dev-stub", model: "default" }),
  publicLabel: "JS Planner",
  instructions: "Plan from JS."
})
const finisher = workflow.node({
  agent: workflow.newAgent({ alias: "js-finisher", provider: "dev-stub", model: "default" }),
  publicLabel: "JS Finisher",
  instructions: "Finish from JS.",
  canCompleteWorkflowRun: true
})
workflow.edge(planner, finisher)
workflow.endpoint(planner, { alias: "entry" })
"#;

        let result = app
            .compile_and_apply_workflow_code_javascript(
                session.id(),
                &node_path,
                source,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect("workflow-code JS should compile and apply");

        assert!(result.compile.validation.ok);
        assert_eq!(
            result.compile.definition.workflow.alias.as_deref(),
            Some("js_coded_flow")
        );
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        let workflow = session
            .workflow(&result.apply.workflow_id)
            .expect("workflow should exist");
        assert_eq!(workflow.alias(), Some("js_coded_flow"));
        assert_eq!(workflow.nodes().len(), 2);
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 3);
    }

    #[test]
    fn workflow_code_javascript_apply_rejects_invalid_source_without_mutating_session() {
        let Some(node_path) = find_node_for_workflow_code_test() else {
            eprintln!("skipping invalid workflow-code JS apply test because node is not available");
            return;
        };
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let source = r#"
workflow.define({ alias: "invalid_coded_flow" })
workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
"#;

        let error = app
            .compile_and_apply_workflow_code_javascript(
                session.id(),
                &node_path,
                source,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                None,
            )
            .expect_err("invalid workflow-code should not apply");

        assert!(format!("{error}").contains("missing_endpoint"));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    }

    #[test]
    fn workflow_code_canonical_patterns_compile_and_apply_with_provider_rebindings() {
        let Some(node_path) = find_node_for_workflow_code_test() else {
            eprintln!("skipping workflow-code pattern apply test because node is not available");
            return;
        };
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let limits = WorkflowCodeLimitsConfig::default();

        for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
            let compiled = crate::workflow_code::compile_workflow_code_javascript(
                &node_path,
                example.source,
                &limits,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "workflow-code pattern `{}` at `{}` should compile: {error}",
                    example.slug, example.path
                )
            });
            assert!(
                compiled.validation.ok,
                "workflow-code pattern `{}` should validate before apply: {:?}",
                example.slug, compiled.validation.diagnostics
            );
            let provider_rebindings = compiled
                .definition
                .nodes
                .iter()
                .map(|node| WorkflowCodeProviderRebinding {
                    node: node.handle.clone(),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                })
                .collect::<Vec<_>>();

            let result = app
                .compile_and_apply_workflow_code_javascript_with_rebindings(
                    session.id(),
                    &node_path,
                    example.source,
                    &limits,
                    "local-user".to_string(),
                    None,
                    &provider_rebindings,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "workflow-code pattern `{}` should apply after provider rebinding: {error}",
                        example.slug
                    )
                });

            assert!(
                result.compile.validation.ok,
                "workflow-code pattern `{}` should compile with valid diagnostics",
                example.slug
            );
            assert_eq!(
                result.apply.node_ids.len(),
                result.compile.definition.nodes.len(),
                "workflow-code pattern `{}` should report every node id",
                example.slug
            );
            assert_eq!(
                result.apply.agent_ids.len(),
                result.compile.definition.nodes.len(),
                "workflow-code pattern `{}` should report every node agent id",
                example.slug
            );
            assert_eq!(
                result.apply.edge_ids.len(),
                result.compile.definition.edges.len(),
                "workflow-code pattern `{}` should report every edge id",
                example.slug
            );
            assert_eq!(
                result.apply.endpoint_ids.len(),
                result.compile.definition.endpoints.len(),
                "workflow-code pattern `{}` should report every endpoint id",
                example.slug
            );
            assert_eq!(
                result.apply.schema_refs.len(),
                result.compile.definition.schemas.len(),
                "workflow-code pattern `{}` should report every schema id",
                example.slug
            );
            assert!(
                result.apply.canvas_layout_applied,
                "workflow-code pattern `{}` should apply canvas layout",
                example.slug
            );

            let session_snapshot = app
                .sessions()
                .get_session(session.id())
                .expect("session should still exist");
            let workflow = session_snapshot
                .workflow(&result.apply.workflow_id)
                .unwrap_or_else(|| {
                    panic!(
                        "workflow-code pattern `{}` should create a workflow",
                        example.slug
                    )
                });
            assert_eq!(
                workflow.nodes().len(),
                result.compile.definition.nodes.len(),
                "workflow-code pattern `{}` should materialize all nodes",
                example.slug
            );
            assert_eq!(
                workflow.edges().len(),
                result.compile.definition.edges.len(),
                "workflow-code pattern `{}` should materialize all edges",
                example.slug
            );
            assert_eq!(
                workflow.endpoints().len(),
                result.compile.definition.endpoints.len(),
                "workflow-code pattern `{}` should materialize all endpoints",
                example.slug
            );
            assert_eq!(
                workflow.schemas().len(),
                result.compile.definition.schemas.len(),
                "workflow-code pattern `{}` should materialize all schemas",
                example.slug
            );
            assert!(
                workflow.run_output_schema_ref().is_some(),
                "workflow-code pattern `{}` should have final output schema",
                example.slug
            );
        }
    }

    #[test]
    fn workflow_code_apply_rejects_metaagent_binding_unowned_existing_agent() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let definition = existing_agent_workflow_code_definition(default_agent.id());

        let error = app
            .apply_workflow_code_definition(
                session.id(),
                &definition,
                &WorkflowCodeLimitsConfig::default(),
                "local-user".to_string(),
                Some("meta-1".to_string()),
            )
            .expect_err("metaagent should not bind an agent it does not control");

        assert!(format!("{error}").contains("not authorized"));
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert!(session.workflows().is_empty());
    }

    #[test]
    fn bootstrap_restores_created_session_and_agents_from_durable_state() {
        let config = DaemonConfig::for_tests();
        let (session_id, default_agent_id, reviewer_agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let reviewer = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("agent should spawn");
            (
                session.id().to_string(),
                default_agent.id().to_string(),
                reviewer.id().to_string(),
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        assert_eq!(restored_session.id(), session_id);
        assert_eq!(
            app.agents
                .get_agent(&default_agent_id)
                .expect("default agent should restore")
                .session_id(),
            session_id
        );
        assert_eq!(
            app.agents
                .get_agent(&reviewer_agent_id)
                .expect("spawned agent should restore")
                .session_id(),
            session_id
        );
    }

    #[test]
    fn bootstrap_restores_ended_session_without_live_agents() {
        let config = DaemonConfig::for_tests();
        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("agent should spawn");
            crate::app::KernelSessionService::new(&mut app)
                .end_session(session.id())
                .expect("session should end");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("ended session should restore");
        assert_eq!(restored_session.status(), SessionStatus::Ended);
        assert!(
            app.agents.get_session_agents(&session_id).is_empty(),
            "ended sessions should not restore live agents"
        );
    }

    #[test]
    fn bootstrap_restores_snapshot_then_replays_later_events() {
        let config = DaemonConfig::for_tests();
        let (session_id, reviewer_agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.save_durable_state_snapshot()
                .expect("snapshot should save");
            let reviewer = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("post-snapshot agent should spawn");
            (session.id().to_string(), reviewer.id().to_string())
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        app.sessions()
            .get_session(&session_id)
            .expect("snapshot session should restore");
        assert_eq!(
            app.agents
                .get_agent(&reviewer_agent_id)
                .expect("post-snapshot event should replay")
                .session_id(),
            session_id
        );
    }

    #[test]
    fn bootstrap_restores_metaagent_events_from_snapshot_then_replays_state() {
        let config = DaemonConfig::for_tests();
        let (metaagent_id, event_id, subscription_id, deleted_subscription_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let metaagent = app
                .agents_mut()
                .activate_agent_meta_mode(metaagent.id(), None)
                .expect("agent should enter meta mode");
            let metaagent_id = metaagent.id().to_string();
            let event = app.metaagent_event_store().record(
                crate::runtime::metaagent_event::NewMetaagentEvent {
                    session_id: session.id().to_string(),
                    metaagent_id: metaagent_id.clone(),
                    owner_user_id: metaagent.owner_user_id().to_string(),
                    kind: "agent.turn.completed".to_string(),
                    source_agent_id: None,
                    title: "Worker completed".to_string(),
                    summary: "Worker completed a turn".to_string(),
                    detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                    injected_prompt_id: Some("prompt-meta-1".to_string()),
                },
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.event.recorded",
                    Some(event.event_id.clone()),
                    serde_json::json!({ "record": &event }),
                )
                .expect("event record should persist");
            app.save_durable_state_snapshot()
                .expect("snapshot should save recorded event");

            let delivered = app
                .metaagent_event_store()
                .update_prompt_delivery_status(
                    &event.event_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued,
                    None,
                )
                .expect("event delivery status should update");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.delivery_updated",
                    Some(delivered.event_id.clone()),
                    serde_json::json!({ "record": &delivered }),
                )
                .expect("event delivery update should persist");

            let read = app
                .metaagent_event_store()
                .read(&metaagent_id, &event.event_id)
                .expect("event should read");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.read",
                    Some(read.event_id.clone()),
                    serde_json::json!({ "record": &read }),
                )
                .expect("event read should persist");

            let acked =
                app.metaagent_event_store()
                    .ack(&metaagent_id, &[event.event_id.clone()], None);
            let acked_event = acked.first().expect("event should ack");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.acked",
                    Some(acked_event.event_id.clone()),
                    serde_json::json!({ "record": acked_event }),
                )
                .expect("event ack should persist");

            let subscription = app.metaagent_event_store().subscribe(
                &metaagent_id,
                "workflow.run.completed".to_string(),
                None,
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.created",
                    Some(subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &subscription }),
                )
                .expect("subscription should persist");

            let deleted_subscription = app.metaagent_event_store().subscribe(
                &metaagent_id,
                "workflow.run.failed".to_string(),
                None,
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.created",
                    Some(deleted_subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &deleted_subscription }),
                )
                .expect("deleted subscription create should persist");
            let deleted_subscription = app
                .metaagent_event_store()
                .unsubscribe(&metaagent_id, &deleted_subscription.subscription_id)
                .expect("subscription should remove");
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.deleted",
                    Some(deleted_subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &deleted_subscription }),
                )
                .expect("subscription deletion should persist");

            (
                metaagent_id,
                event.event_id,
                subscription.subscription_id,
                deleted_subscription.subscription_id,
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_events = app.metaagent_event_store().list(
            &metaagent_id,
            Some("agent.turn.completed"),
            Some("acked"),
            10,
        );
        assert_eq!(restored_events.len(), 1);
        assert_eq!(restored_events[0].event_id, event_id);
        assert!(restored_events[0].read_at_ms.is_some());
        assert!(restored_events[0].ack_at_ms.is_some());
        assert_eq!(
            restored_events[0].injected_prompt_id.as_deref(),
            Some("prompt-meta-1")
        );
        assert_eq!(
            restored_events[0].prompt_delivery_status,
            crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
        );
        assert!(restored_events[0].prompt_delivery_updated_at_ms.is_some());

        let restored_subscriptions = app
            .metaagent_event_store()
            .list_subscriptions(&metaagent_id);
        assert_eq!(restored_subscriptions.len(), 1);
        assert_eq!(restored_subscriptions[0].subscription_id, subscription_id);
        assert_ne!(
            restored_subscriptions[0].subscription_id,
            deleted_subscription_id
        );
    }

    #[test]
    fn bootstrap_reconciles_stale_runtime_work_after_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, workflow_run_id, workflow_node_run_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            app.sessions_mut()
                .submit_prompt(
                    session.id(),
                    attachment.id(),
                    agent.id(),
                    "still running when the kernel stops",
                    Vec::new(),
                )
                .expect("prompt should start");

            let mut session = app
                .sessions()
                .get_session(session.id())
                .expect("session should still exist");
            let session_id = session.id().to_string();
            session.set_active_provider_run(Some("provider-run-stale".to_string()));
            let node_run = WorkflowNodeRun::new(
                "node-run-stale",
                "node-1",
                agent.id(),
                1,
                WorkflowNodeRunStatus::Running,
            );
            let mut workflow_run = WorkflowRun::new(
                "workflow-run-stale",
                "workflow-1",
                "endpoint-1",
                "node-1",
                Some("invoke".to_string()),
                None,
                vec![node_run],
                Vec::new(),
            );
            workflow_run.set_active_node_run("node-run-stale");
            workflow_run.set_status(WorkflowRunStatus::Running);
            session.create_workflow_run(workflow_run);
            app.sessions.restore_session(session);
            app.save_durable_state_snapshot()
                .expect("snapshot should save stale runtime state");
            (
                session_id,
                "workflow-run-stale".to_string(),
                "node-run-stale".to_string(),
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        assert_eq!(restored.active_provider_run_id(), None);
        assert!(restored.active_prompt().is_none());
        assert_eq!(restored.scheduler_state(), SchedulerState::Idle);
        let workflow_run = restored
            .workflow_run(&workflow_run_id)
            .expect("workflow run should restore");
        assert_eq!(workflow_run.status(), WorkflowRunStatus::Stopped);
        assert_eq!(workflow_run.active_node_run_id(), None);
        assert_eq!(
            workflow_run.node_runs()[0].status(),
            WorkflowNodeRunStatus::Stopped
        );
        assert_eq!(workflow_run.node_runs()[0].id(), workflow_node_run_id);
        assert!(workflow_run
            .failure_events()
            .iter()
            .any(|event| { event.message().contains("interrupted by kernel restart") }));
    }
}

pub(crate) struct KernelSessionReadService<'a> {
    app: &'a DaemonApp,
}

impl<'a> KernelSessionReadService<'a> {
    pub(crate) fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let mut session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let agents = self.app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        self.app.project_session_runtime_view(&mut session);
        self.app.update_session_projection(session.clone());
        Ok(session)
    }

    pub(crate) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self.app.attachments.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(crate) fn session_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let entries = self.app.load_session_history_entries(&session, None)?;
        self.app
            .session_history_projection_store()
            .update_entries(session.id(), entries.clone());
        Ok(entries)
    }
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        let session =
            SessionStateOwner::new(self.app.session_state_store()).create_session(request)?;
        let defaults = session.agent_defaults();
        let mut agent_request = CreateAgentRequest::new(session.id(), &defaults.provider)
            .with_owner_user_id(session.owner_user_id().to_string())
            .with_worktree(session.worktree_id());
        if let Some(model) = defaults.model.as_deref() {
            agent_request = agent_request.with_model(model.to_string());
        }
        if let Some(effort) = defaults.effort.as_deref() {
            agent_request = agent_request.with_effort(effort.to_string());
        }
        if let Some(account_profile) = defaults.account_profile.as_deref() {
            agent_request = agent_request.with_account_profile(account_profile.to_string());
        }
        if let Some(execution_mode) = defaults.execution_mode {
            agent_request = agent_request.with_execution_mode_override(execution_mode);
        }
        if let Some(permission_level) = defaults.permission_level {
            agent_request = agent_request.with_permission_level_override(permission_level);
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
        drop(sessions);
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session.id())?;
        self.app.durable_state_store().append_event(
            "session.created",
            Some(session.id().to_string()),
            serde_json::json!({
                "session": &session,
                "default_agent": &agent,
            }),
        )?;

        crate::logging::info_with_fields(
            "daemon.session",
            "session created with default agent",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_ref": agent.agent_ref(),
            }),
        );

        Ok((session, agent))
    }

    pub(crate) fn attach(
        &mut self,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .app
            .attachments
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let attachment = self.app.attachments.attach(&mut sessions, request)?;
        drop(sessions);

        // Create default agent if session has no agents (e.g., after session was ended and reattached).
        // Parked/active sessions that were never ended will retain their existing agents.
        let session_agents = self.app.agents.get_session_agents(&session_id);
        if session_agents.is_empty() {
            let worktree_id = self
                .app
                .sessions()
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request =
                CreateAgentRequest::new(&session_id, "default").with_worktree(worktree_id);
            let session_store = self.app.session_state_store();
            let mut sessions = session_store.write();
            let _agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
            drop(sessions);
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.app.sync_focused_provider_run_if_idle(&session_id)?;

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    pub(crate) fn spawn_agent(
        &mut self,
        mut request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        if let Some(kernel_ref) = request.kernel_ref.clone() {
            if self.app.kernel_ref_is_local(&kernel_ref) {
                request.kernel_ref = None;
            } else {
                let agent = self.app.spawn_worker_agent(request, &kernel_ref)?;
                self.app.durable_state_store().append_event(
                    "agent.created",
                    Some(agent.id().to_string()),
                    serde_json::json!({
                        "agent": &agent,
                    }),
                )?;
                return Ok(agent);
            }
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(request, &mut sessions)?;
        drop(sessions);
        self.app.durable_state_store().append_event(
            "agent.created",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
            }),
        )?;
        Ok(agent)
    }

    pub(crate) fn apply_workflow_code_definition(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        self.apply_workflow_code_definition_with_rebindings(
            session_id,
            definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
        )
    }

    pub(crate) fn apply_workflow_code_definition_with_rebindings(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        let validation = definition.validate_with_limits(limits);
        if !validation.ok {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: format!(
                    "workflow-code definition is invalid: {}",
                    validation
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        let mut definition = definition.clone();
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        let mut target_validation = WorkflowCodeValidationReport {
            ok: true,
            diagnostics: Vec::new(),
        };
        self.append_workflow_code_target_validation(
            session_id,
            &definition,
            &mut target_validation,
            controlled_by_metaagent_id.as_deref(),
        )?;
        if !target_validation.ok {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: workflow_code_validation_error_message(
                    "workflow-code target validation failed",
                    &target_validation,
                ),
            });
        }

        let mut node_agent_ids = BTreeMap::new();
        for node in &definition.nodes {
            let agent_id = match &node.agent {
                WorkflowCodeAgentBinding::Create(agent) => {
                    let mut request = CreateAgentRequest::new(session_id, agent.provider.clone())
                        .with_owner_user_id(created_by_user_id.clone());
                    if let Some(alias) = agent.alias.as_deref() {
                        request = request.with_alias(alias.to_string());
                    }
                    if let Some(model) = agent.model.as_deref() {
                        request = request.with_model(model.to_string());
                    }
                    if let Some(effort) = agent.effort.as_deref() {
                        request = request.with_effort(effort.to_string());
                    }
                    if let Some(account_profile) = agent.account_profile.as_deref() {
                        request = request.with_account_profile(account_profile.to_string());
                    }
                    if let Some(metaagent_id) = controlled_by_metaagent_id.as_deref() {
                        request = request.with_controlled_by_metaagent_id(metaagent_id.to_string());
                    }
                    let created =
                        self.spawn_workflow_code_generated_agent(request, agent.alias.as_deref())?;
                    self.grant_workflow_code_node_extensions(created.id(), &node.extensions)?;
                    created.id().to_string()
                }
                WorkflowCodeAgentBinding::Existing(existing) => {
                    let agent = self.app.agents.get_agent(&existing.agent_ref)?;
                    if agent.session_id() != session_id {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: format!(
                                "existing agent `{}` belongs to session `{}` instead of `{session_id}`",
                                existing.agent_ref,
                                agent.session_id()
                            ),
                        });
                    }
                    if agent.is_metaagent() {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: format!(
                                "invalid_existing_agent_binding: existing agent `{}` is a metaagent and cannot be bound to workflow node `{}`",
                                existing.agent_ref, node.handle
                            ),
                        });
                    }
                    if let Some(metaagent_id) = controlled_by_metaagent_id.as_deref() {
                        if agent.controlled_by_metaagent_id() != Some(metaagent_id) {
                            return Err(DaemonError::LocalTransport {
                                operation: "workflow_code.apply",
                                message: format!(
                                    "metaagent `{metaagent_id}` is not authorized to bind existing agent `{}`",
                                    existing.agent_ref
                                ),
                            });
                        }
                    }
                    self.grant_workflow_code_node_extensions(agent.id(), &node.extensions)?;
                    agent.id().to_string()
                }
            };
            node_agent_ids.insert(node.handle.clone(), agent_id);
        }

        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let report = sessions.apply_workflow_code_definition(
            session_id,
            &definition,
            &node_agent_ids,
            limits,
            created_by_user_id.clone(),
            controlled_by_metaagent_id.clone(),
        )?;
        drop(sessions);

        self.app.durable_state_store().append_event(
            "workflow_code.applied",
            Some(report.workflow_id.clone()),
            serde_json::json!({
                "session_id": session_id,
                "created_by_user_id": created_by_user_id,
                "controlled_by_metaagent_id": controlled_by_metaagent_id,
                "report": &report,
            }),
        )?;
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(report)
    }

    fn validate_workflow_code_extension_requirement(
        &self,
        workspace_id: &str,
        node_handle: &str,
        grant: &ExtensionGrant,
    ) -> Result<(), DaemonError> {
        let result = match &grant.kind {
            ExtensionKind::Mcp => crate::runtime::capability_registry::ensure_mcp_exists(
                Some(workspace_id),
                &grant.name,
            ),
            ExtensionKind::Skill => crate::runtime::capability_registry::ensure_skill_exists(
                Some(workspace_id),
                &grant.name,
            ),
            ExtensionKind::Script => {
                let environment =
                    grant
                        .environment
                        .as_deref()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: "script extension requirements must include environment"
                                .to_string(),
                        })?;
                crate::runtime::capability_registry::ensure_script_exists(
                    Some(workspace_id),
                    &grant.name,
                )?;
                crate::runtime::capability_registry::ensure_environment_exists(
                    Some(workspace_id),
                    environment,
                )
            }
            ExtensionKind::Connector => {
                crate::runtime::capability_registry::ensure_connector_exists(&grant.name)?;
                if let Some(credential) = grant.credential.as_deref() {
                    crate::runtime::capability_registry::ensure_credential_exists(credential)?;
                }
                crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref()).map(|_| ())
            }
        };

        result.map_err(|error| DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "node `{node_handle}` extension requirement `{}:{}` cannot be satisfied: {error}",
                grant.kind.as_str(),
                grant.name
            ),
        })
    }

    fn spawn_workflow_code_generated_agent(
        &mut self,
        request: CreateAgentRequest,
        requested_alias: Option<&str>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let Some(alias) = requested_alias else {
            return self.spawn_agent(request);
        };
        let trimmed_alias = alias.trim();
        if trimmed_alias.is_empty() {
            return self.spawn_agent(request);
        }

        for attempt in 0..1000 {
            let candidate_alias = if attempt == 0 {
                trimmed_alias.to_string()
            } else {
                format!("{trimmed_alias}-{}", attempt + 1)
            };
            let candidate = request.clone().with_alias(candidate_alias);
            match self.spawn_agent(candidate) {
                Ok(agent) => return Ok(agent),
                Err(DaemonError::AgentAliasConflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "could not allocate a unique generated agent alias for `{trimmed_alias}`"
            ),
        })
    }

    fn grant_workflow_code_node_extensions(
        &mut self,
        agent_id: &str,
        grants: &[ExtensionGrant],
    ) -> Result<(), DaemonError> {
        for grant in grants {
            let agent = self.app.agents.grant_extension(agent_id, grant.clone())?;
            self.app.durable_state_store().append_event(
                "agent.extension_granted",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "grant": grant,
                    "source": "workflow_code",
                }),
            )?;
        }
        Ok(())
    }

    pub(crate) fn compile_and_apply_workflow_code_javascript(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        self.compile_and_apply_workflow_code_javascript_with_rebindings(
            session_id,
            node_path,
            source,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
        )
    }

    pub(crate) fn compile_and_validate_workflow_code_source_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        language: WorkflowCodeLanguage,
        limits: &WorkflowCodeLimitsConfig,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        caller_metaagent_id: Option<&str>,
    ) -> Result<WorkflowCodeCompileResult, DaemonError> {
        let schema_import_root = self.workflow_code_schema_import_root(session_id)?;
        let mut compile = compile_workflow_code_source_with_schema_import_root(
            node_path,
            source,
            language,
            limits,
            schema_import_root.as_deref(),
        )?;
        let mut definition = compile.definition.clone();
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        if compile.validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &definition,
                &mut compile.validation,
                caller_metaagent_id,
            )?;
            crate::workflow_code::attach_workflow_code_diagnostic_spans(
                &mut compile.validation,
                &compile.source_spans,
            );
        }
        compile.definition = definition;
        Ok(compile)
    }

    pub(crate) fn validate_workflow_code_definition_with_rebindings(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        caller_metaagent_id: Option<&str>,
    ) -> Result<(WorkflowCodeDefinition, WorkflowCodeValidationReport), DaemonError> {
        let mut definition = definition.clone();
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        let mut validation = definition.validate_with_limits(limits);
        if validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &definition,
                &mut validation,
                caller_metaagent_id,
            )?;
        }
        Ok((definition, validation))
    }

    pub(crate) fn compile_and_apply_workflow_code_javascript_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        self.compile_and_apply_workflow_code_source_with_rebindings(
            session_id,
            node_path,
            source,
            WorkflowCodeLanguage::JavaScript,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            provider_rebindings,
        )
    }

    pub(crate) fn compile_and_apply_workflow_code_source_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        language: WorkflowCodeLanguage,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        let schema_import_root = self.workflow_code_schema_import_root(session_id)?;
        let compile = compile_workflow_code_source_with_schema_import_root(
            node_path,
            source,
            language,
            limits,
            schema_import_root.as_deref(),
        )?;
        let mut rebound_definition = compile.definition.clone();
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut rebound_definition,
            provider_rebindings,
        )?;
        let mut validation = rebound_definition.validate_with_limits(limits);
        if validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &rebound_definition,
                &mut validation,
                controlled_by_metaagent_id.as_deref(),
            )?;
            crate::workflow_code::attach_workflow_code_diagnostic_spans(
                &mut validation,
                &compile.source_spans,
            );
        }
        let apply = self.apply_workflow_code_definition_with_rebindings(
            session_id,
            &rebound_definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
        )?;
        Ok(WorkflowCodeCompileAndApplyResult {
            compile: crate::workflow_code::WorkflowCodeCompileResult {
                definition: rebound_definition,
                validation,
                logs: compile.logs,
                source_spans: compile.source_spans,
            },
            apply,
        })
    }

    fn workflow_code_schema_import_root(
        &self,
        session_id: &str,
    ) -> Result<Option<std::path::PathBuf>, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let workspace = std::path::PathBuf::from(session.workspace_id());
        if workspace.is_absolute() {
            Ok(Some(workspace))
        } else {
            Ok(None)
        }
    }

    fn append_workflow_code_target_validation(
        &self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        validation: &mut WorkflowCodeValidationReport,
        caller_metaagent_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let session = self.app.sessions().get_session(session_id)?;
        let registry = self.app.providers.registry();
        let generated_agent_count = definition
            .nodes
            .iter()
            .filter(|node| matches!(&node.agent, WorkflowCodeAgentBinding::Create(_)))
            .count();
        let current_agent_count = self.app.agents.get_session_agents(session_id).len();
        if current_agent_count.saturating_add(generated_agent_count) > session.max_agents() as usize
        {
            push_workflow_code_target_validation_error(
                validation,
                "session_agent_limit_exceeded",
                format!(
                    "workflow-code would create {generated_agent_count} agents but session `{session_id}` has {current_agent_count}/{} agents",
                    session.max_agents()
                ),
                None,
            );
        }
        let runtime_queue_limit = self.app.config().max_workflow_queues_per_workflow();
        let materialized_queue_count =
            crate::workflow_code::workflow_code_materialized_queue_count(definition);
        if materialized_queue_count > runtime_queue_limit {
            push_workflow_code_target_validation_error(
                validation,
                "limit_exceeded",
                format!(
                    "queues count {materialized_queue_count} exceeds configured runtime workflow queue limit {runtime_queue_limit}"
                ),
                None,
            );
        }
        if !workflow_code_alias_can_allocate(&session, definition.workflow.alias.as_deref()) {
            let alias = definition
                .workflow
                .alias
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            push_workflow_code_target_validation_error(
                validation,
                "workflow_alias_unavailable",
                format!(
                    "workflow-code alias `{alias}` cannot allocate a unique workflow alias after {} attempts",
                    crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS
                ),
                None,
            );
        }
        for node in &definition.nodes {
            match &node.agent {
                WorkflowCodeAgentBinding::Create(agent) => {
                    let provider = agent.provider.trim();
                    let adapter_key = adapter_key_for_provider(provider);
                    if registry.resolve(adapter_key).is_none() {
                        push_workflow_code_target_validation_error(
                            validation,
                            "unavailable_provider",
                            format!(
                                "node `{}` requests unavailable provider `{provider}`; available providers: {}",
                                node.handle,
                                registry.advertised_provider_ids().join(", ")
                            ),
                            Some(node.handle.clone()),
                        );
                    } else if let Some(model) = agent.model.as_deref() {
                        if let Some(catalog) = self.app.cached_provider_catalog() {
                            let model = model.trim();
                            if model != "default" && !model.is_empty() {
                                if let Some(provider_info) =
                                    catalog.all.iter().find(|item| item.id == provider)
                                {
                                    if !provider_info.models.is_empty()
                                        && !provider_info.models.contains_key(model)
                                    {
                                        push_workflow_code_target_validation_error(
                                            validation,
                                            "unavailable_model",
                                            format!(
                                                "node `{}` requests unavailable model `{model}` for provider `{provider}`; available models: {}",
                                                node.handle,
                                                provider_info
                                                    .models
                                                    .keys()
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ),
                                            Some(node.handle.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                WorkflowCodeAgentBinding::Existing(existing) => {
                    match self.app.agents.get_agent(&existing.agent_ref) {
                        Ok(agent) if agent.session_id() == session_id => {
                            if agent.is_metaagent() {
                                push_workflow_code_target_validation_error(
                                    validation,
                                    "invalid_existing_agent_binding",
                                    format!(
                                        "existing agent `{}` is a metaagent and cannot be bound to workflow node `{}`",
                                        existing.agent_ref, node.handle
                                    ),
                                    Some(node.handle.clone()),
                                );
                            } else if caller_metaagent_id.is_some_and(|metaagent_id| {
                                agent.controlled_by_metaagent_id() != Some(metaagent_id)
                            }) {
                                push_workflow_code_target_validation_error(
                                    validation,
                                    "unauthorized_existing_agent_binding",
                                    format!(
                                        "metaagent is not authorized to bind existing agent `{}`",
                                        existing.agent_ref
                                    ),
                                    Some(node.handle.clone()),
                                );
                            }
                        }
                        Ok(agent) => push_workflow_code_target_validation_error(
                            validation,
                            "invalid_existing_agent_binding",
                            format!(
                            "existing agent `{}` belongs to session `{}` instead of `{session_id}`",
                            existing.agent_ref,
                            agent.session_id()
                        ),
                            Some(node.handle.clone()),
                        ),
                        Err(error) => push_workflow_code_target_validation_error(
                            validation,
                            "invalid_existing_agent_binding",
                            format!(
                                "existing agent `{}` cannot be resolved: {error}",
                                existing.agent_ref
                            ),
                            Some(node.handle.clone()),
                        ),
                    }
                }
            }
            for grant in &node.extensions {
                if let Err(error) = self.validate_workflow_code_extension_requirement(
                    session.workspace_id(),
                    &node.handle,
                    grant,
                ) {
                    push_workflow_code_target_validation_error(
                        validation,
                        "unavailable_extension",
                        error.to_string(),
                        Some(node.handle.clone()),
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn destroy_agent(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote) = agent.remote_execution().cloned() {
            let target = arroba_relay::protocol::ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target.clone(),
                    crate::transport::relay_peer::RelayPeerRequest::DestroyLeasedAgent {
                        leased_agent_id: remote.leased_agent_id.clone(),
                    },
                ),
            )?;
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target,
                    crate::transport::relay_peer::RelayPeerRequest::DestroyExecutionLease {
                        lease_id: remote.execution_lease_id.clone(),
                    },
                ),
            )?;
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        self.app.agents.destroy_agent(agent_id, &mut sessions)
    }

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let (attachment, effect) = self
            .app
            .attachments
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);
        let owner_removed_queued_prompt_count =
            self.app.prompt_owner_remove_queued_prompts_by_attachment(
                attachment.session_id(),
                attachment_id,
            )?;
        let removed_queued_prompt_count = effect
            .removed_queued_prompt_count
            .max(owner_removed_queued_prompt_count);
        let session_after_detach = SessionStateReader::new(self.app.session_state_store())
            .get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self
                    .app
                    .advance_next_queued_prompt(attachment.session_id(), agent_id)?;
            }
        }

        let remaining_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(attachment.session_id());
        let has_active_prompt = self
            .app
            .prompt_owner_has_any_active_prompt(attachment.session_id())?;
        if remaining_attachment_ids.is_empty() && !has_active_prompt {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                let run = self.app.providers.get_run(&active_provider_run_id)?;
                if run.state() != ProviderRunState::Ended {
                    let outcome = self
                        .app
                        .providers
                        .park_run_provider_only(attachment.session_id(), &active_provider_run_id)?;
                    if SessionStateReader::new(self.app.session_state_store())
                        .get_session(attachment.session_id())?
                        .active_provider_run_id()
                        == Some(outcome.run().id())
                    {
                        SessionStateOwner::new(self.app.session_state_store())
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    self.app.update_provider_run_projection(outcome.into_run());
                }
            }
            for run in self.app.providers.list_runs() {
                if run.session_id() == attachment.session_id() {
                    crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
                    crate::transport::flow_control::clear_active_turn(self.app, run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": effect.removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            self.app.prompt_owner_remove_session(session_id);
            self.app
                .external_provider_session_index_store()
                .detach_session(session_id);
            let ended =
                SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
            self.app.durable_state_store().append_event(
                "session.ended",
                Some(ended.id().to_string()),
                serde_json::json!({
                    "session": &ended,
                    "already_ended": true,
                }),
            )?;
            return Ok(ended);
        }

        let removed_attachments = self.app.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .app
            .providers
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if SessionStateReader::new(self.app.session_state_store())
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                SessionStateOwner::new(self.app.session_state_store())
                    .set_active_provider_run(session_id, None)?;
            }
            let run = outcome.into_run();
            super::provider_runtime::ProviderProcessTracker::new(self.app).remove_run(run.id())?;
        }

        let removed_agents = self.app.agents.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.app.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
                crate::transport::flow_control::clear_active_turn(self.app, run.id());
            }
        }
        self.app.prompt_owner_remove_session(session_id);
        self.app
            .external_provider_session_index_store()
            .detach_session(session_id);
        let mut ended =
            SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
        ended.set_agents(removed_agents);
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        self.app.durable_state_store().append_event(
            "session.ended",
            Some(ended.id().to_string()),
            serde_json::json!({
                "session": &ended,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        )?;
        Ok(ended)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self
            .app
            .agents
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self
            .app
            .should_defer_provider_run_sync_for_focus_change(session_id, agent_id)?
        {
            self.app
                .sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .app
            .sessions()
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.resize_provider_terminal(session_id, &provider_run_id, cols, rows)
    }

    pub(crate) fn resize_provider_terminal(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let _ = super::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }

        if provider_run.endpoint_mode() == AgentEndpointMode::External {
            return Ok(());
        }

        self.app.pty.resize(provider_run_id, cols, rows)
    }
}

fn workflow_code_alias_can_allocate(
    session: &crate::session::RuntimeSession,
    requested_alias: Option<&str>,
) -> bool {
    let Some(alias) = requested_alias else {
        return true;
    };
    let trimmed_alias = alias.trim().to_lowercase();
    if trimmed_alias.is_empty() {
        return true;
    }
    (0..crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS).any(|attempt| {
        let candidate_alias = if attempt == 0 {
            trimmed_alias.clone()
        } else {
            format!("{trimmed_alias}-{}", attempt + 1)
        };
        !session
            .workflows()
            .iter()
            .any(|workflow| workflow.alias() == Some(candidate_alias.as_str()))
    })
}

fn push_workflow_code_target_validation_error(
    validation: &mut WorkflowCodeValidationReport,
    code: &'static str,
    message: impl Into<String>,
    handle: Option<String>,
) {
    validation.ok = false;
    validation
        .diagnostics
        .push(WorkflowCodeValidationDiagnostic {
            severity: WorkflowCodeValidationSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            handle,
            source_span: None,
        });
}

fn workflow_code_validation_error_message(
    prefix: &'static str,
    validation: &WorkflowCodeValidationReport,
) -> String {
    let details = validation
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let handle = diagnostic
                .handle
                .as_deref()
                .map(|handle| format!(" handle `{handle}`"))
                .unwrap_or_default();
            format!("{}{}: {}", diagnostic.code, handle, diagnostic.message)
        })
        .collect::<Vec<_>>()
        .join("; ");
    if details.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {details}")
    }
}
