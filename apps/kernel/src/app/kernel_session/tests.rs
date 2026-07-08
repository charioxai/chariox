use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::WorkflowCodeLimitsConfig;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
use crate::session::{
    CreateSessionRequest, SchedulerState, SessionStatus, WorkflowNodeRun, WorkflowNodeRunStatus,
    WorkflowRun, WorkflowRunStatus,
};
use crate::workflow_code::{
    WORKFLOW_CODE_PATTERN_EXAMPLES, WORKFLOW_CODE_SCHEMA_VERSION, WorkflowCodeAgentBinding,
    WorkflowCodeAgentCreate, WorkflowCodeAgentRebinding, WorkflowCodeDefinition,
    WorkflowCodeEndpointDefinition, WorkflowCodeExistingAgent, WorkflowCodeNodeDefinition,
    WorkflowCodeProviderRebinding, WorkflowCodeQueueDefinition, WorkflowCodeWorkflow,
};
use crate::{DaemonApp, DaemonConfig};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn generated_workflow_code_definition() -> WorkflowCodeDefinition {
    WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: WorkflowCodeWorkflow {
            alias: Some("generated_agents".to_string()),
            prompt: None,
            flush_agent_context_before_run: Some(true),
            max_concurrent: Some(2),
            run_output_schema: None,
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
        schedules: Vec::new(),
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
        format!("---\nname: {name}\ndescription: Workflow-code test skill\n---\nUse this skill.\n"),
    )
    .expect("skill should be written");
}

mod bootstrap;
mod workflow_code_apply;
mod workflow_code_preflight;
