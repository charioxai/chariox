use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, TerminalCommandCatalog, TerminalCommandCatalogExecutionTarget,
    TerminalCommandCatalogNode, TerminalCommandCatalogNodeKind, TerminalCommandCatalogSurface,
    TerminalOperationContract, TerminalOperationRegistry,
};

const CATALOG_JSON_FRAGMENTS: &[(&str, &str)] = &[
    (
        "core",
        include_str!("terminal_command_catalog/catalog/core.json"),
    ),
    (
        "extensions",
        include_str!("terminal_command_catalog/catalog/extensions.json"),
    ),
    (
        "workflow",
        include_str!("terminal_command_catalog/catalog/workflow.json"),
    ),
    (
        "notifications",
        include_str!("terminal_command_catalog/catalog/notifications.json"),
    ),
    (
        "workspace",
        include_str!("terminal_command_catalog/catalog/workspace.json"),
    ),
    (
        "provider",
        include_str!("terminal_command_catalog/catalog/provider.json"),
    ),
];

const PARITY_MANIFEST_JSON: &str = include_str!("terminal_operation_registry/parity_manifest.json");
const AGENT_TERMINAL_CONTRACTS_JSON: &str =
    include_str!("terminal_operation_registry/contracts.json");

#[derive(Debug, Deserialize)]
struct ParityManifest {
    schema_version: u32,
    source: String,
    requests: Vec<ParityManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ParityManifestEntry {
    variant: String,
    classification: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct AgentTerminalContracts {
    schema_version: u32,
    source: String,
    contracts: std::collections::HashMap<String, AgentTerminalContract>,
}

#[derive(Debug, Deserialize)]
struct AgentTerminalContract {
    required_targets: Vec<String>,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RawCommandNode {
    id: String,
    label: String,
    description: String,
    value: String,
    #[serde(default, alias = "searchAliases")]
    search_aliases: Vec<String>,
    #[serde(default)]
    intents: Vec<String>,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    kind: Option<TerminalCommandCatalogNodeKind>,
    #[serde(default)]
    execution_target: Option<TerminalCommandCatalogExecutionTarget>,
    #[serde(default)]
    surfaces: Vec<TerminalCommandCatalogSurface>,
    #[serde(default)]
    dynamic_source: Option<String>,
    #[serde(default)]
    children: Vec<RawCommandNode>,
}

pub(crate) fn terminal_command_catalog_response() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::TerminalCommandCatalog {
        catalog: terminal_command_catalog()?,
    })
}

pub(crate) fn terminal_operation_registry_response() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::TerminalOperationRegistry {
        registry: terminal_operation_registry()?,
    })
}

pub(crate) fn terminal_operation_registry() -> Result<TerminalOperationRegistry, DaemonError> {
    let manifest =
        serde_json::from_str::<ParityManifest>(PARITY_MANIFEST_JSON).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "terminal_operation_registry.parse",
                message: error.to_string(),
            }
        })?;
    if manifest.schema_version != 1 || manifest.source != "LocalDaemonRequest" {
        return Err(DaemonError::LocalTransport {
            operation: "terminal_operation_registry.schema",
            message: "unsupported terminal parity manifest".to_string(),
        });
    }
    let mut operations = terminal_command_catalog()?
        .nodes
        .iter()
        .flat_map(flatten_catalog_operations)
        .collect::<Vec<_>>();
    let catalog_ids = operations
        .iter()
        .map(|operation| operation.id.clone())
        .collect::<std::collections::HashSet<_>>();
    operations.extend(
        manifest
            .requests
            .iter()
            .map(operation_from_manifest_entry)
            .filter(|operation| !catalog_ids.contains(&operation.id))
            .collect::<Vec<_>>(),
    );
    let revision = operation_registry_revision(&operations)?;
    Ok(TerminalOperationRegistry {
        revision,
        operations,
    })
}

fn flatten_catalog_operations(node: &TerminalCommandCatalogNode) -> Vec<TerminalOperationContract> {
    let mut operations = vec![operation_from_catalog_node(node)];
    for child in &node.children {
        operations.extend(flatten_catalog_operations(child));
    }
    operations
}

fn operation_from_catalog_node(node: &TerminalCommandCatalogNode) -> TerminalOperationContract {
    let action = node.id.rsplit('-').next().unwrap_or(node.id.as_str());
    let mutation = ![
        "get",
        "list",
        "show",
        "read",
        "search",
        "inspect",
        "open",
        "status",
        "health",
        "members",
        "collaborators",
        "invites",
        "kernels",
        "ls",
        "runs",
        "logs",
        "trace",
        "preview",
        "validate",
    ]
    .iter()
    .any(|verb| action == *verb);
    let mut search_aliases = node.search_aliases.clone();
    if search_aliases.is_empty() {
        search_aliases.push(node.id.replace('-', " "));
        let label = node.label.trim_start_matches('/').trim();
        if !label.is_empty() {
            search_aliases.push(label.to_string());
        }
    }
    search_aliases.sort();
    search_aliases.dedup();
    let intents = if node.intents.is_empty() {
        vec![if mutation {
            "mutate".to_string()
        } else {
            "inspect".to_string()
        }]
    } else {
        node.intents.clone()
    };
    let examples = if node.examples.is_empty() {
        vec![node.value.trim().to_string()]
    } else {
        node.examples.clone()
    };
    let presentation_only = node.execution_target != TerminalCommandCatalogExecutionTarget::Kernel;
    let supported_surfaces = node
        .surfaces
        .iter()
        .map(|surface| {
            match surface {
                TerminalCommandCatalogSurface::Session => "session",
                TerminalCommandCatalogSurface::WaitingRoom => "waiting_room",
                TerminalCommandCatalogSurface::WorkflowScreen => "workflow_screen",
            }
            .to_string()
        })
        .chain(
            agent_terminal_supported_for_catalog_node(node).then_some("agent_terminal".to_string()),
        )
        .collect();
    TerminalOperationContract {
        id: node.id.clone(),
        command: Some(node.value.trim_start_matches('/').to_string()),
        description: node.description.clone(),
        search_aliases,
        intents,
        required_context: vec!["workspace".to_string(), "worktree".to_string()],
        required_targets: required_targets_for(&node.id),
        input_schema: serde_json::json!({
            "type": "string",
            "description": "Arguments appended to the native Chariox shell command"
        }),
        result_kind: "terminal_command".to_string(),
        mutation,
        expected_projections: vec![
            "session_snapshot".to_string(),
            "terminal_events".to_string(),
        ],
        supported_surfaces,
        examples,
        parity_variants: Vec::new(),
        presentation_only,
    }
}

fn agent_terminal_supported_for_catalog_node(node: &TerminalCommandCatalogNode) -> bool {
    if node.execution_target != TerminalCommandCatalogExecutionTarget::Kernel {
        return false;
    }
    !matches!(
        node.id.as_str(),
        "agent-focus"
            | "agent-cycle"
            | "view"
            | "view-individual"
            | "view-split"
            | "waiting"
            | "exit"
            | "quit"
            | "credential-set"
            | "credential-register"
    )
}

fn required_targets_for(variant: &str) -> Vec<String> {
    let mut targets = Vec::new();
    if variant.contains("session")
        || variant.contains("prompt")
        || variant.contains("agent")
        || variant.contains("workflow")
        || variant.contains("interaction")
        || variant.contains("provider")
        || variant.contains("workspace")
    {
        targets.push("session_id".to_string());
    }
    if variant.contains("agent") || variant.contains("prompt") {
        targets.push("agent_id".to_string());
    }
    if variant.contains("workflow") {
        targets.push("workflow_ref".to_string());
    }
    targets
}

fn operation_from_manifest_entry(entry: &ParityManifestEntry) -> TerminalOperationContract {
    let snake = snake_case(&entry.variant);
    // The parity manifest is an inventory of kernel requests. A request is
    // only exposed to stateless agent terminals once it has a concrete input
    // contract and target adapter; generic serde forwarding is not a contract.
    let explicit_contract = explicit_agent_terminal_contract(&entry.variant);
    let supported =
        entry.classification == "agent_terminal_supported" && explicit_contract.is_some();
    let mutation = !matches!(
        entry.variant.as_str(),
        value if value.starts_with("Get")
            || value.starts_with("List")
            || value.starts_with("Resolve")
            || value.starts_with("Search")
            || value.starts_with("Read")
            || value.starts_with("Inspect")
            || value.starts_with("Poll")
            || value.starts_with("Query")
            || value.starts_with("Semantic")
            || value.starts_with("Show")
            || value.starts_with("Browse")
            || value.starts_with("Observe")
    );
    let required_targets = explicit_contract
        .as_ref()
        .map(|contract| contract.required_targets.clone())
        .unwrap_or_else(|| required_targets_for(&entry.variant.to_lowercase()));
    let input_schema = explicit_contract
        .as_ref()
        .map(|contract| contract.input_schema.clone())
        .unwrap_or_else(|| serde_json::json!({
            "type": "object",
            "description": "Kernel request contract pending an agent-terminal adapter; not executable on the agent-terminal surface.",
            "additionalProperties": false
        }));
    let description = if supported {
        format!("Execute or inspect {snake} through the Chariox kernel")
    } else {
        entry.reason.clone()
    };
    TerminalOperationContract {
        id: format!("terminal.{snake}"),
        command: None,
        description,
        search_aliases: vec![entry.variant.clone(), snake.replace('_', " ")],
        intents: vec![if mutation {
            "mutate".to_string()
        } else {
            "inspect".to_string()
        }],
        required_context: vec!["workspace".to_string(), "worktree".to_string()],
        required_targets,
        input_schema,
        result_kind: "kernel_response".to_string(),
        mutation,
        expected_projections: if mutation {
            vec![
                "session_snapshot".to_string(),
                "terminal_events".to_string(),
            ]
        } else {
            vec!["response".to_string()]
        },
        supported_surfaces: if supported {
            vec![
                "agent_terminal".to_string(),
                "tui".to_string(),
                "web".to_string(),
            ]
        } else {
            vec!["tui".to_string()]
        },
        examples: if supported {
            vec![format!("terminal.{snake}")]
        } else {
            Vec::new()
        },
        parity_variants: vec![entry.variant.clone()],
        presentation_only: entry.classification == "presentation_only",
    }
}

struct ExplicitAgentTerminalContract {
    required_targets: Vec<String>,
    input_schema: serde_json::Value,
}

fn explicit_agent_terminal_contract(variant: &str) -> Option<ExplicitAgentTerminalContract> {
    static CONTRACTS: std::sync::OnceLock<Option<AgentTerminalContracts>> =
        std::sync::OnceLock::new();
    let contracts = CONTRACTS
        .get_or_init(|| {
            serde_json::from_str::<AgentTerminalContracts>(AGENT_TERMINAL_CONTRACTS_JSON)
                .ok()
                .filter(|contracts| {
                    contracts.schema_version == 1 && contracts.source == "LocalDaemonRequest"
                })
        })
        .as_ref()?;
    let contract = contracts.contracts.get(variant)?;
    Some(ExplicitAgentTerminalContract {
        required_targets: contract.required_targets.clone(),
        input_schema: contract.input_schema.clone(),
    })
}

fn operation_registry_revision(
    operations: &[TerminalOperationContract],
) -> Result<String, DaemonError> {
    let serialized =
        serde_json::to_vec(operations).map_err(|error| DaemonError::LocalTransport {
            operation: "terminal_operation_registry.revision",
            message: error.to_string(),
        })?;
    let hash = Sha256::digest(serialized);
    Ok(format!("sha256:{hash:x}"))
}

fn snake_case(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .fold(String::new(), |mut output, (index, character)| {
            if character.is_uppercase() && index > 0 {
                output.push('_');
            }
            output.extend(character.to_lowercase());
            output
        })
}

pub(crate) fn terminal_command_catalog() -> Result<TerminalCommandCatalog, DaemonError> {
    let mut raw = Vec::new();
    for (name, source) in CATALOG_JSON_FRAGMENTS {
        let mut fragment =
            serde_json::from_str::<Vec<RawCommandNode>>(source).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "terminal_command_catalog.parse",
                    message: format!("{name}: {error}"),
                }
            })?;
        raw.append(&mut fragment);
    }
    let nodes = raw.into_iter().map(enrich_node).collect::<Vec<_>>();
    let revision = catalog_revision(&nodes)?;
    Ok(TerminalCommandCatalog { revision, nodes })
}

fn enrich_node(raw: RawCommandNode) -> TerminalCommandCatalogNode {
    let children = raw
        .children
        .into_iter()
        .map(enrich_node)
        .collect::<Vec<_>>();
    let kind = raw
        .kind
        .unwrap_or_else(|| infer_kind(&raw.id, &raw.value, &children));
    let execution_target = raw
        .execution_target
        .unwrap_or_else(|| infer_execution_target(&kind, &raw.id));
    let surfaces = if raw.surfaces.is_empty() {
        vec![TerminalCommandCatalogSurface::Session]
    } else {
        raw.surfaces
    };
    let dynamic_source = raw.dynamic_source.or_else(|| infer_dynamic_source(&raw.id));
    TerminalCommandCatalogNode {
        id: raw.id,
        label: raw.label,
        description: raw.description,
        value: raw.value,
        kind,
        execution_target,
        surfaces,
        search_aliases: raw.search_aliases,
        intents: raw.intents,
        examples: raw.examples,
        dynamic_source,
        children,
    }
}

fn infer_kind(
    id: &str,
    value: &str,
    children: &[TerminalCommandCatalogNode],
) -> TerminalCommandCatalogNodeKind {
    if id == "meta" {
        return TerminalCommandCatalogNodeKind::PromptPrefix;
    }
    if matches!(
        id,
        "provider" | "model" | "variant" | "mode" | "permissions" | "view"
    ) {
        return TerminalCommandCatalogNodeKind::Dynamic;
    }
    if !children.is_empty() || value.ends_with(' ') {
        return TerminalCommandCatalogNodeKind::Group;
    }
    TerminalCommandCatalogNodeKind::Command
}

fn infer_execution_target(
    kind: &TerminalCommandCatalogNodeKind,
    id: &str,
) -> TerminalCommandCatalogExecutionTarget {
    if *kind == TerminalCommandCatalogNodeKind::PromptPrefix {
        return TerminalCommandCatalogExecutionTarget::PromptPrefix;
    }
    if matches!(
        id,
        "view"
            | "view-split"
            | "view-individual"
            | "exit"
            | "quit"
            | "model"
            | "variant"
            | "mode"
            | "permissions"
            | "misc"
            | "attach"
    ) {
        return TerminalCommandCatalogExecutionTarget::TerminalLocal;
    }
    TerminalCommandCatalogExecutionTarget::Kernel
}

fn infer_dynamic_source(id: &str) -> Option<String> {
    match id {
        "provider" => Some("provider_catalog.providers".to_string()),
        "model" => Some("provider_catalog.models".to_string()),
        "variant" => Some("provider_catalog.variants".to_string()),
        "mode" => Some("session_config.modes".to_string()),
        "permissions" => Some("session_config.permissions".to_string()),
        "view" => Some("terminal.views".to_string()),
        _ => None,
    }
}

fn catalog_revision(nodes: &[TerminalCommandCatalogNode]) -> Result<String, DaemonError> {
    let serialized = serde_json::to_vec(nodes).map_err(|error| DaemonError::LocalTransport {
        operation: "terminal_command_catalog.revision",
        message: error.to_string(),
    })?;
    let hash = Sha256::digest(serialized);
    Ok(format!("sha256:{hash:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect<'a>(
        nodes: &'a [TerminalCommandCatalogNode],
        out: &mut Vec<&'a TerminalCommandCatalogNode>,
    ) {
        for node in nodes {
            out.push(node);
            collect(&node.children, out);
        }
    }

    #[test]
    fn terminal_command_catalog_includes_meta_prompt_prefix() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let mut nodes = Vec::new();
        collect(&catalog.nodes, &mut nodes);

        let meta = nodes
            .into_iter()
            .find(|node| node.id == "meta")
            .expect("meta command should be present");
        assert_eq!(meta.value, "/meta ");
        assert_eq!(meta.kind, TerminalCommandCatalogNodeKind::PromptPrefix);
        assert_eq!(
            meta.execution_target,
            TerminalCommandCatalogExecutionTarget::PromptPrefix
        );
        assert!(catalog.revision.starts_with("sha256:"));
    }

    #[test]
    fn terminal_command_catalog_explains_scheduled_prompt_syntax() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let wait_in = catalog
            .nodes
            .iter()
            .find(|node| node.id == "wait-in")
            .expect("wait-in command should be present");
        let wait_every = catalog
            .nodes
            .iter()
            .find(|node| node.id == "wait-every")
            .expect("wait-every command should be present");

        assert!(wait_in.description.contains("/wait-in <minutes> <prompt>"));
        assert_eq!(wait_in.examples, vec!["/wait-in 15 Check the latest build"]);
        assert!(wait_every
            .description
            .contains("/wait-every <minutes> <prompt>"));
        assert_eq!(wait_every.examples, vec!["/wait-every 30 Report progress"]);
    }

    #[test]
    fn terminal_command_catalog_covers_routed_workflow_actions() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let mut nodes = Vec::new();
        collect(&catalog.nodes, &mut nodes);

        for (id, value) in [
            ("workflow-open", "/workflow"),
            ("workflow-list", "/workflow list"),
            ("workflow-show", "/workflow show "),
            ("workflow-new", "/workflow new "),
            ("workflow-delete", "/workflow delete "),
            ("workflow-run", "/workflow run "),
            ("workflow-start", "/workflow start "),
            ("workflow-flush-context", "/workflow flush-context "),
            ("workflow-run-output-schema", "/workflow run-output-schema "),
            ("workflow-max-turns", "/workflow max-turns "),
            ("workflow-runs", "/workflow runs "),
            ("workflow-run-show", "/workflow run-show "),
            ("workflow-run-get", "/workflow run-get "),
            ("workflow-add-node-all", "/workflow add node all"),
            ("workflow-queue-list", "/workflow queue"),
            ("workflow-queue-create", "/workflow queue create "),
            ("workflow-queue-rename", "/workflow queue rename "),
            ("workflow-queue-priority", "/workflow queue priority "),
            ("workflow-queue-enable", "/workflow queue enable "),
            ("workflow-queue-disable", "/workflow queue disable "),
            ("workflow-queue-delete", "/workflow queue delete "),
            ("workflow-queue-edit", "/workflow queue edit "),
            ("workflow-queue-move", "/workflow queue move "),
            ("workflow-queue-clear", "/workflow queue clear "),
            ("workflow-queue-flush", "/workflow queue flush "),
            ("workflow-queue-remove", "/workflow queue remove "),
            ("workflow-cancel", "/workflow cancel "),
            ("workflow-pause", "/workflow pause "),
            ("workflow-resume", "/workflow resume "),
            ("workflow-terminal", "/workflow terminal "),
            ("workflow-pane-logs", "/workflow pane logs "),
            ("workflow-pane-trace", "/workflow pane trace "),
            ("workflow-pane-edit", "/workflow pane edit "),
            ("workflow-schedule-add", "/workflow schedule add "),
            ("workflow-schedule-list", "/workflow schedule list "),
            ("workflow-schedule-enable", "/workflow schedule enable "),
            ("workflow-schedule-disable", "/workflow schedule disable "),
            ("workflow-schedule-remove", "/workflow schedule remove "),
            ("workflow-schedule-preview", "/workflow schedule preview "),
            ("workflow-watchdog-add", "/workflow watchdog add "),
            ("workflow-watchdog-list", "/workflow watchdog list "),
            ("workflow-watchdog-enable", "/workflow watchdog enable "),
            ("workflow-watchdog-disable", "/workflow watchdog disable "),
            ("workflow-watchdog-remove", "/workflow watchdog remove "),
            ("workflow-alias", "/workflow "),
            ("workflow-node-add", "/workflow node add "),
            ("workflow-node-add-all", "/workflow node add all"),
            ("workflow-node-remove", "/workflow node remove "),
            (
                "workflow-node-can-complete-run",
                "/workflow node can-complete-run ",
            ),
            (
                "workflow-node-can-emit-intermediate-output",
                "/workflow node can-emit-intermediate-output ",
            ),
            (
                "workflow-node-wait-for-all-inputs",
                "/workflow node wait-for-all-inputs ",
            ),
            ("workflow-node-max-turns", "/workflow node max-turns "),
            (
                "workflow-node-intermediate-output-schema",
                "/workflow node intermediate-output-schema ",
            ),
            ("workflow-node-extensions", "/workflow node extensions "),
            (
                "workflow-node-extension-grant",
                "/workflow node extension grant ",
            ),
            (
                "workflow-node-extension-revoke",
                "/workflow node extension revoke ",
            ),
            (
                "workflow-node-instructions-show",
                "/workflow node instructions show ",
            ),
            (
                "workflow-node-instructions-set",
                "/workflow node instructions set ",
            ),
            (
                "workflow-node-instructions-save",
                "/workflow node instructions save",
            ),
            (
                "workflow-node-instructions-close",
                "/workflow node instructions close",
            ),
            ("workflow-edge-shorthand", "/workflow "),
            ("workflow-edge-add", "/workflow edge add "),
            ("workflow-edge-remove", "/workflow edge remove "),
            ("workflow-endpoint-new", "/workflow endpoint new "),
            ("workflow-endpoint-alias", "/workflow endpoint alias "),
            ("workflow-endpoint-bind", "/workflow endpoint bind "),
            ("workflow-endpoint-rebind", "/workflow endpoint rebind "),
            ("workflow-endpoint-remove", "/workflow endpoint remove "),
            ("workflow-code-validate", "/workflow code validate "),
            ("workflow-code-apply", "/workflow code apply "),
            ("workflow-code-run", "/workflow code run "),
            ("workflow-code-save", "/workflow code save "),
            (
                "workflow-code-artifact-list",
                "/workflow code artifact list",
            ),
            ("workflow-code-artifact-get", "/workflow code artifact get "),
            (
                "workflow-code-artifact-delete",
                "/workflow code artifact delete ",
            ),
            (
                "workflow-code-artifact-apply",
                "/workflow code artifact apply ",
            ),
            ("workflow-code-artifact-run", "/workflow code artifact run "),
            (
                "workflow-code-package-export",
                "/workflow code package export ",
            ),
            (
                "workflow-code-package-import",
                "/workflow code package import ",
            ),
            (
                "workflow-code-source-export",
                "/workflow code source export ",
            ),
            (
                "workflow-code-source-export-directory",
                "/workflow code source export-directory ",
            ),
            ("workflow-trigger-list", "/workflow trigger list"),
            ("workflow-trigger-create", "/workflow trigger create "),
            ("workflow-trigger-show", "/workflow trigger show "),
            ("workflow-trigger-export", "/workflow trigger export "),
            (
                "workflow-trigger-config-show",
                "/workflow trigger config show ",
            ),
            (
                "workflow-trigger-config-set",
                "/workflow trigger config set ",
            ),
            (
                "workflow-trigger-config-clear",
                "/workflow trigger config clear ",
            ),
            ("workflow-trigger-disable", "/workflow trigger disable "),
            ("workflow-registry-list", "/workflow registry list"),
            ("workflow-registry-get", "/workflow registry get "),
            ("workflow-registry-add", "/workflow registry add "),
            (
                "workflow-registry-add-from-workflow",
                "/workflow registry add-from-workflow ",
            ),
            ("workflow-registry-load", "/workflow load "),
            ("workflow-registry-run", "/workflow run "),
            ("workflow-registry-delete", "/workflow registry delete "),
        ] {
            let node = nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap_or_else(|| panic!("routed workflow command {id} should be discoverable"));
            assert_eq!(node.value, value, "catalog value drifted for {id}");
        }
    }

    #[test]
    fn terminal_command_catalog_marks_dynamic_roots() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let provider = catalog
            .nodes
            .iter()
            .find(|node| node.id == "provider")
            .expect("provider command should be present");

        assert_eq!(provider.kind, TerminalCommandCatalogNodeKind::Dynamic);
        assert_eq!(
            provider.dynamic_source.as_deref(),
            Some("provider_catalog.providers")
        );
        for (id, source) in [
            ("mode", "session_config.modes"),
            ("permissions", "session_config.permissions"),
        ] {
            let node = catalog
                .nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap_or_else(|| panic!("{id} command should be present"));
            assert_eq!(node.kind, TerminalCommandCatalogNodeKind::Dynamic);
            assert_eq!(node.dynamic_source.as_deref(), Some(source));
            assert_eq!(
                node.surfaces,
                vec![
                    TerminalCommandCatalogSurface::Session,
                    TerminalCommandCatalogSurface::WaitingRoom,
                ]
            );
        }
    }

    #[test]
    fn terminal_operation_registry_classifies_namespaced_reads_by_action() {
        let registry = terminal_operation_registry().expect("registry should load");
        for id in [
            "session-list",
            "session-status",
            "cloud-status",
            "cloud-members",
            "cloud-collaborators",
            "collab-invites",
            "kernel-health",
            "kernel-status",
            "machine-kernels",
            "machine-ls",
        ] {
            let operation = registry
                .operations
                .iter()
                .find(|operation| operation.id == id)
                .unwrap_or_else(|| panic!("{id} should be registered"));
            assert!(!operation.mutation, "{id} should be read-only");
            assert!(
                operation.intents.iter().any(|intent| intent == "inspect"),
                "{id} should inspect"
            );
        }
    }

    #[test]
    fn terminal_operation_registry_covers_catalog_and_parity_manifest() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let registry = terminal_operation_registry().expect("operation registry should load");
        let mut nodes = Vec::new();
        collect(&catalog.nodes, &mut nodes);

        for node in nodes {
            let operation = registry
                .operations
                .iter()
                .find(|operation| operation.id == node.id)
                .unwrap_or_else(|| {
                    panic!(
                        "catalog command {} is missing from operation registry",
                        node.id
                    )
                });
            assert_eq!(
                operation.command.as_deref(),
                Some(node.value.trim_start_matches('/'))
            );
            assert_eq!(
                operation.presentation_only,
                node.execution_target != TerminalCommandCatalogExecutionTarget::Kernel
            );
        }

        let manifest = serde_json::from_str::<ParityManifest>(PARITY_MANIFEST_JSON)
            .expect("parity manifest should remain valid");
        for entry in manifest.requests {
            assert!(
                registry
                    .operations
                    .iter()
                    .any(|operation| operation.parity_variants.contains(&entry.variant)),
                "parity variant {} is missing from operation registry",
                entry.variant
            );
        }
        assert!(registry.revision.starts_with("sha256:"));

        for id in [
            "agent-focus",
            "agent-cycle",
            "waiting",
            "credential-set",
            "meta",
            "loop",
            "goal",
            "wait-in",
            "wait-every",
            "model",
            "variant",
            "mode",
            "permissions",
            "misc",
            "attach",
        ] {
            let operation = registry
                .operations
                .iter()
                .find(|operation| operation.id == id)
                .unwrap_or_else(|| panic!("{id} command should be present"));
            assert!(
                !operation
                    .supported_surfaces
                    .iter()
                    .any(|surface| surface == "agent_terminal"),
                "{id} must not be advertised to agent terminals"
            );
        }
    }

    #[test]
    fn terminal_operation_registry_publishes_real_contracts_for_supported_requests() {
        let registry = terminal_operation_registry().expect("operation registry should load");
        let supported = registry
            .operations
            .iter()
            .filter(|operation| {
                operation.parity_variants.first().is_some_and(|variant| {
                    operation
                        .supported_surfaces
                        .iter()
                        .any(|surface| surface == "agent_terminal")
                        && variant != "AppendNativeProviderOutput"
                        && variant != "AppendNativeProviderOutputBatch"
                        && variant != "CompletePrompt"
                })
            })
            .collect::<Vec<_>>();
        let manifest_supported = serde_json::from_str::<ParityManifest>(PARITY_MANIFEST_JSON)
            .expect("parity manifest should remain valid")
            .requests
            .into_iter()
            .filter(|entry| entry.classification == "agent_terminal_supported")
            .count();
        assert_eq!(
            supported.len(),
            manifest_supported,
            "all user-facing request variants need agent-terminal contracts"
        );
        assert!(supported
            .iter()
            .all(|operation| operation.input_schema.is_object()
                || operation.input_schema.get("type")
                    == Some(&serde_json::Value::String("null".to_string()))));
        let advertised = registry
            .operations
            .iter()
            .filter(|operation| {
                operation
                    .supported_surfaces
                    .iter()
                    .any(|surface| surface == "agent_terminal")
            })
            .collect::<Vec<_>>();
        assert!(advertised.iter().all(|operation| {
            !operation.description.trim().is_empty()
                && !operation.search_aliases.is_empty()
                && !operation.intents.is_empty()
                && !operation.expected_projections.is_empty()
                && !operation.supported_surfaces.is_empty()
                && !operation.examples.is_empty()
        }));

        let submit_prompts = registry
            .operations
            .iter()
            .find(|operation| operation.parity_variants == vec!["SubmitPrompts".to_string()])
            .expect("SubmitPrompts should be discoverable");
        assert!(submit_prompts
            .supported_surfaces
            .iter()
            .any(|surface| surface == "agent_terminal"));
        assert_eq!(
            submit_prompts.input_schema["required"],
            serde_json::json!(["session_id", "attachment_id", "prompts"])
        );
        assert_eq!(
            submit_prompts.input_schema["properties"]["prompts"]["items"]["required"],
            serde_json::json!(["target_agent_id", "prompt"])
        );
    }

    #[test]
    fn terminal_command_catalog_misc_group_has_scoped_value() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let misc = catalog
            .nodes
            .iter()
            .find(|node| node.id == "misc")
            .expect("misc command should be present");

        assert_eq!(misc.label, "/misc");
        assert_eq!(misc.value, "/misc ");
        assert_eq!(misc.kind, TerminalCommandCatalogNodeKind::Group);
        assert_eq!(
            misc.children
                .iter()
                .map(|node| node.value.as_str())
                .collect::<Vec<_>>(),
            vec!["/attach ", "/stop", "/waiting", "/exit"]
        );
    }

    #[test]
    fn terminal_command_catalog_marks_every_detached_safe_command_for_the_waiting_room() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let mut nodes = Vec::new();
        collect(&catalog.nodes, &mut nodes);
        let waiting_room_nodes = nodes
            .into_iter()
            .filter(|node| {
                node.surfaces
                    .contains(&TerminalCommandCatalogSurface::WaitingRoom)
            })
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            waiting_room_nodes,
            vec![
                "session",
                "session-new",
                "session-attach",
                "session-list",
                "session-delete",
                "cloud",
                "cloud-open",
                "cloud-link",
                "cloud-status",
                "cloud-invite-accept",
                "cloud-collaborators",
                "relay",
                "relay-status",
                "relay-use",
                "relay-disable",
                "relay-invite-accept",
                "collab",
                "collab-invite-accept",
                "kernel",
                "kernel-health",
                "kernel-remote-runtime",
                "kernel-delete",
                "kernel-runtime",
                "kernel-status",
                "machine",
                "machine-list",
                "machine-kernels",
                "machine-approve",
                "machine-forget",
                "machine-rename",
                "mcp",
                "mcp-list",
                "mcp-install",
                "mcp-import",
                "mcp-show",
                "skill",
                "skill-list",
                "skill-install",
                "skill-import",
                "skill-show",
                "env",
                "env-list",
                "env-register",
                "env-remove",
                "env-show",
                "script",
                "script-list",
                "script-validate",
                "script-register",
                "script-show",
                "credential",
                "credential-list",
                "credential-set",
                "credential-register",
                "credential-remove",
                "credential-show",
                "connector",
                "connector-list",
                "connector-register",
                "connector-doctor",
                "connector-test",
                "connector-show",
                "notifications",
                "notifications-catalog",
                "notifications-search",
                "notifications-category",
                "notifications-show",
                "notifications-connections",
                "notifications-connect",
                "notifications-authorization",
                "notifications-connection-show",
                "notifications-connection-resources",
                "notifications-connection-refresh",
                "notifications-connection-reconnect",
                "notifications-connection-dependencies",
                "notifications-connection-remove",
                "slice",
                "slice-list",
                "slice-create",
                "slice-status",
                "slice-doctor",
                "slice-logs",
                "slice-audit",
                "slice-audit-limit",
                "slice-state",
                "slice-save-state",
                "slice-save-state-restart-agents",
                "slice-save-state-shutdown",
                "slice-save-state-this-slice",
                "slice-save-state-future-slices",
                "slice-backup",
                "slice-backup-name",
                "slice-reset-state",
                "slice-start",
                "slice-stop",
                "slice-delete",
                "slice-screen",
                "slice-ls",
                "slice-show",
                "slice-auth",
                "slice-auth-login",
                "slice-auth-import",
                "slice-auth-remove",
                "slice-auth-alias",
                "workspace",
                "workspace-set",
                "workspace-sync-default",
                "workspace-sync-default-off",
                "workspace-sync-default-managed",
                "workspace-sync-default-tracked",
                "worktree",
                "worktree-set",
                "worktree-create",
                "worktree-name",
                "provider",
                "provider-select",
                "provider-status",
                "provider-login",
                "provider-logout",
                "provider-reauth",
                "provider-processes",
                "provider-processes-teardown",
                "config",
                "config-show",
                "config-path",
                "config-keys",
                "config-schema",
                "config-set",
                "config-unset",
                "config-workspace-live-sync",
                "config-workspace-live-sync-off",
                "config-workspace-live-sync-managed",
                "config-workspace-live-sync-tracked",
                "model",
                "variant",
                "mode",
                "permissions",
                "view",
                "view-individual",
                "view-split",
                "exit",
                "waiting",
            ]
        );
    }

    #[test]
    fn terminal_command_catalog_registers_workflow_trigger_lifecycle() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let workflow = catalog
            .nodes
            .iter()
            .find(|node| node.id == "workflow")
            .expect("workflow command should be present");
        let trigger = workflow
            .children
            .iter()
            .find(|node| node.id == "workflow-trigger")
            .expect("workflow trigger commands should be present");

        assert_eq!(trigger.value, "/workflow trigger ");
        assert_eq!(
            trigger
                .children
                .iter()
                .map(|node| node.value.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/workflow trigger list",
                "/workflow trigger create ",
                "/workflow trigger show ",
                "/workflow trigger export ",
                "/workflow trigger config ",
                "/workflow trigger event ",
                "/workflow trigger disable ",
            ]
        );
    }

    #[test]
    fn terminal_command_catalog_registers_event_generator_management() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let event = catalog
            .nodes
            .iter()
            .find(|node| node.id == "workflow")
            .and_then(|node| {
                node.children
                    .iter()
                    .find(|child| child.id == "workflow-trigger")
            })
            .and_then(|node| {
                node.children
                    .iter()
                    .find(|child| child.id == "workflow-trigger-event")
            })
            .expect("workflow event publication commands should be present");

        assert!(event
            .children
            .iter()
            .any(|node| node.value == "/workflow trigger event install "));
        assert!(event
            .children
            .iter()
            .any(|node| node.value == "/workflow trigger event resources "));

        let notifications = catalog
            .nodes
            .iter()
            .find(|node| node.id == "notifications")
            .expect("notification center commands should be present");
        assert!(notifications
            .children
            .iter()
            .any(|node| node.value == "/notifications connection remove "));
    }
}
