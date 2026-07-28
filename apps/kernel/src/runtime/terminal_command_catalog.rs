use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, TerminalCommandCatalog, TerminalCommandCatalogExecutionTarget,
    TerminalCommandCatalogNode, TerminalCommandCatalogNodeKind, TerminalCommandCatalogSurface,
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
        "workspace",
        include_str!("terminal_command_catalog/catalog/workspace.json"),
    ),
    (
        "provider",
        include_str!("terminal_command_catalog/catalog/provider.json"),
    ),
];

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
        "view" | "view-split" | "view-individual" | "exit" | "quit"
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
    fn terminal_command_catalog_registers_workflow_publication_lifecycle() {
        let catalog = terminal_command_catalog().expect("catalog should load");
        let workflow = catalog
            .nodes
            .iter()
            .find(|node| node.id == "workflow")
            .expect("workflow command should be present");
        let publication = workflow
            .children
            .iter()
            .find(|node| node.id == "workflow-publication")
            .expect("workflow publication commands should be present");

        assert_eq!(publication.value, "/workflow publication ");
        assert_eq!(
            publication
                .children
                .iter()
                .map(|node| node.value.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/workflow publication list",
                "/workflow publication create ",
                "/workflow publication show ",
                "/workflow publication export ",
                "/workflow publication config ",
                "/workflow publication event ",
                "/workflow publication disable ",
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
                    .find(|child| child.id == "workflow-publication")
            })
            .and_then(|node| {
                node.children
                    .iter()
                    .find(|child| child.id == "workflow-publication-event")
            })
            .expect("workflow event publication commands should be present");

        assert!(event
            .children
            .iter()
            .any(|node| node.value == "/workflow publication event authorize "));
        assert!(event
            .children
            .iter()
            .any(|node| node.value == "/workflow publication event resources "));
    }
}
