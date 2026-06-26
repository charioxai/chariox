use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaagentGuide {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub tags: &'static [&'static str],
    pub commands: &'static [&'static str],
    pub body: &'static str,
}

pub(crate) const METAAGENT_GUIDES: &[MetaagentGuide] = &[
    MetaagentGuide {
        id: "workflows/basic-components",
        title: "Workflow basic components",
        summary: "Defines workflow nodes, edges, endpoints, and runs; use this before building any workflow.",
        tags: &["workflow", "tutorial", "components"],
        commands: &[
            "workflow new",
            "workflow node add",
            "workflow edge add",
            "workflow endpoint new",
            "workflow run",
            "workflow runs",
        ],
        body: include_str!("metaagent_guides/workflows/basic-components.md"),
    },
    MetaagentGuide {
        id: "workflows/basic-dag",
        title: "Minimal directed workflow",
        summary: "Shows the smallest implementation-to-review DAG and how to make it runnable.",
        tags: &["workflow", "tutorial", "dag", "handoff"],
        commands: &[
            "agent spawn",
            "workflow new",
            "workflow node add",
            "workflow edge add",
            "workflow endpoint new",
            "workflow run",
        ],
        body: include_str!("metaagent_guides/workflows/basic-dag.md"),
    },
    MetaagentGuide {
        id: "workflows/edges-and-handoffs",
        title: "Workflow edges and handoffs",
        summary: "Explains how downstream nodes receive work and how missing edges leave workflows incomplete.",
        tags: &["workflow", "edges", "handoff", "debugging"],
        commands: &[
            "workflow edge add",
            "workflow edge remove",
            "workflow resolve",
            "workflow runs",
        ],
        body: include_str!("metaagent_guides/workflows/edges-and-handoffs.md"),
    },
    MetaagentGuide {
        id: "workflows/partial-and-final-outputs",
        title: "Partial and final workflow outputs",
        summary: "Explains intermediate outputs, final outputs, and when the metaagent should inspect run state.",
        tags: &["workflow", "outputs", "supervision", "completion"],
        commands: &[
            "workflow node intermediate-output",
            "workflow runs",
            "workflow get-run",
        ],
        body: include_str!("metaagent_guides/workflows/partial-and-final-outputs.md"),
    },
    MetaagentGuide {
        id: "workflows/events-and-supervision",
        title: "Workflow events and supervision",
        summary: "Lists workflow event subscriptions and the supervision loop for sleeping until updates arrive.",
        tags: &["workflow", "events", "supervision"],
        commands: &[
            "workflow runs",
            "workflow get-run",
            "arroba.meta.subscribe_events",
            "arroba.meta.list_events",
        ],
        body: include_str!("metaagent_guides/workflows/events-and-supervision.md"),
    },
    MetaagentGuide {
        id: "workflows/common-failures",
        title: "Common workflow failures",
        summary: "Covers missing endpoints, missing edges, invalid events, queued prompts, and absent workflow output.",
        tags: &["workflow", "debugging", "failures", "recovery"],
        commands: &[
            "workflow resolve",
            "workflow runs",
            "workflow get-run",
            "workflow edge add",
            "workflow endpoint new",
            "workflow resume",
        ],
        body: include_str!("metaagent_guides/workflows/common-failures.md"),
    },
    MetaagentGuide {
        id: "workflows/workflow-code-authoring",
        title: "Workflow-code authoring",
        summary: "Exact JavaScript builder API for creating, validating, applying, running, exporting, and importing workflow-code artifacts.",
        tags: &["workflow", "workflow-code", "script", "metaagent"],
        commands: &[
            "arroba.meta.workflow_code.create",
            "arroba.meta.workflow_code.read",
            "arroba.meta.workflow_code.list",
            "arroba.meta.workflow_code.update",
            "arroba.meta.workflow_code.delete",
            "arroba.meta.workflow_code.validate",
            "arroba.meta.workflow_code.apply",
            "arroba.meta.workflow_code.run",
            "arroba.meta.workflow_code.export",
            "arroba.meta.workflow_code.import",
            "arroba.meta.workflow_code.package_export",
            "arroba.meta.workflow_code.package_import",
            "arroba.meta.workflow_code.source_export",
            "arroba.meta.workflow_code.source_export_directory",
            "arroba.meta.workflow_registry.list",
            "arroba.meta.workflow_registry.get",
            "arroba.meta.workflow_registry.add",
            "arroba.meta.workflow_registry.add_from_workflow",
            "arroba.meta.workflow_registry.load",
            "arroba.meta.workflow_registry.run",
        ],
        body: include_str!("metaagent_guides/workflows/workflow-code-authoring.md"),
    },
    MetaagentGuide {
        id: "workflows/workflow-code-patterns",
        title: "Workflow-code pattern examples",
        summary: "Canonical, kernel-compiled workflow-code scripts for the dynamic workflow pattern suite.",
        tags: &["workflow", "workflow-code", "examples", "patterns"],
        commands: &[
            "arroba.meta.workflow_code.validate",
            "arroba.meta.workflow_code.apply",
            "arroba.meta.workflow_code.run",
            "arroba.meta.workflow_code.export",
            "arroba.meta.workflow_code.import",
            "arroba.meta.workflow_code.package_export",
            "arroba.meta.workflow_code.package_import",
            "arroba.meta.workflow_code.source_export",
            "arroba.meta.workflow_code.source_export_directory",
        ],
        body: include_str!("metaagent_guides/workflows/workflow-code-patterns.md"),
    },
    MetaagentGuide {
        id: "agent-apps/generate-app",
        title: "Generate an agent app",
        summary: "Recipe for building an app by planning, delegating to regular agents, and validating through workers.",
        tags: &["agent-app", "webapp", "delegation", "workflow"],
        commands: &[
            "agent spawn",
            "prompt",
            "workflow new",
            "workflow run",
            "workflow runs",
            "workflow get-run",
        ],
        body: include_str!("metaagent_guides/agent-apps/generate-app.md"),
    },
];

const METAAGENT_GUIDE_DIR: &str = "metaagent-guides";
const BUILTIN_GUIDE_DIR: &str = "builtin";

#[derive(Debug, Clone, Default)]
pub(crate) struct MetaagentGuideSearchArgs {
    pub query: Option<String>,
    pub tag: Option<String>,
    pub command: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaagentGuideSourceScope {
    Workspace,
    User,
    BuiltinCopy,
    Embedded,
}

impl MetaagentGuideSourceScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::User => "user",
            Self::BuiltinCopy => "builtin_copy",
            Self::Embedded => "embedded",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MetaagentGuideContext {
    workspace: Option<PathBuf>,
    user_root: Option<PathBuf>,
    seed_builtin_copies: bool,
}

impl MetaagentGuideContext {
    pub(crate) fn for_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: Some(workspace.into()),
            user_root: arroba_home().map(|home| home.join(METAAGENT_GUIDE_DIR)),
            seed_builtin_copies: true,
        }
    }

    fn embedded_only() -> Self {
        Self {
            workspace: None,
            user_root: None,
            seed_builtin_copies: false,
        }
    }

    #[cfg(test)]
    fn for_test(workspace: PathBuf, user_root: PathBuf, seed_builtin_copies: bool) -> Self {
        Self {
            workspace: Some(workspace),
            user_root: Some(user_root),
            seed_builtin_copies,
        }
    }
}

#[derive(Debug, Clone)]
struct EffectiveMetaagentGuide {
    id: String,
    title: String,
    summary: String,
    tags: Vec<String>,
    commands: Vec<String>,
    body: String,
    source_scope: MetaagentGuideSourceScope,
    source_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetaagentGuideFrontmatter {
    id: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    tags: Vec<String>,
    commands: Vec<String>,
}

pub(crate) fn list_guides(args: MetaagentGuideSearchArgs) -> Vec<serde_json::Value> {
    search_guides(args).into_iter().collect()
}

pub(crate) fn search_guides(args: MetaagentGuideSearchArgs) -> Vec<serde_json::Value> {
    search_guides_with_context(args, &MetaagentGuideContext::embedded_only())
}

pub(crate) fn list_guides_with_context(
    args: MetaagentGuideSearchArgs,
    context: &MetaagentGuideContext,
) -> Vec<serde_json::Value> {
    search_guides_with_context(args, context)
        .into_iter()
        .collect()
}

pub(crate) fn search_guides_with_context(
    args: MetaagentGuideSearchArgs,
    context: &MetaagentGuideContext,
) -> Vec<serde_json::Value> {
    let query = args.query.as_deref().map(normalize_search_text);
    let tag = args.tag.as_deref().map(str::to_ascii_lowercase);
    let command = args.command.as_deref().map(normalize_search_text);
    let mut matches = effective_guides(context)
        .into_iter()
        .filter_map(|guide| {
            if let Some(tag) = tag.as_deref() {
                if !guide
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                {
                    return None;
                }
            }
            if let Some(command) = command.as_deref() {
                if !guide
                    .commands
                    .iter()
                    .any(|candidate| normalize_search_text(candidate).contains(command))
                {
                    return None;
                }
            }
            let score = query
                .as_deref()
                .map(|query| score_guide(query, &guide))
                .unwrap_or(1);
            (score > 0).then_some((score, guide))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.title.cmp(&right.title))
    });
    matches
        .into_iter()
        .take(args.limit.unwrap_or(20).clamp(1, 100))
        .map(|(_, guide)| guide_summary(&guide, false))
        .collect()
}

pub(crate) fn read_guide(guide_ref: &str) -> Option<serde_json::Value> {
    read_guide_with_context(guide_ref, &MetaagentGuideContext::embedded_only())
}

pub(crate) fn read_guide_with_context(
    guide_ref: &str,
    context: &MetaagentGuideContext,
) -> Option<serde_json::Value> {
    let needle = normalize_search_text(guide_ref);
    effective_guides(context)
        .into_iter()
        .find(|guide| {
            normalize_search_text(&guide.id) == needle
                || normalize_search_text(&guide.title) == needle
        })
        .map(|guide| guide_summary(&guide, true))
}

fn guide_summary(guide: &EffectiveMetaagentGuide, include_body: bool) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "id": guide.id,
        "title": guide.title,
        "summary": guide.summary,
        "tags": guide.tags,
        "commands": guide.commands,
        "source_scope": guide.source_scope.as_str(),
    });
    if let Some(path) = &guide.source_path {
        payload["source_path"] = serde_json::Value::String(path.display().to_string());
    }
    if include_body {
        payload["body"] = serde_json::Value::String(guide.body.clone());
    }
    payload
}

fn effective_guides(context: &MetaagentGuideContext) -> Vec<EffectiveMetaagentGuide> {
    if context.seed_builtin_copies {
        if let Some(user_root) = &context.user_root {
            let _ = seed_builtin_guide_copies(user_root);
        }
    }

    let mut by_id = BTreeMap::<String, EffectiveMetaagentGuide>::new();
    for guide in embedded_guides() {
        by_id.insert(guide.id.clone(), guide);
    }
    if let Some(user_root) = &context.user_root {
        for guide in read_guides_from_root(
            &user_root.join(BUILTIN_GUIDE_DIR),
            MetaagentGuideSourceScope::BuiltinCopy,
            false,
        ) {
            by_id.insert(guide.id.clone(), guide);
        }
        for guide in read_guides_from_root(user_root, MetaagentGuideSourceScope::User, true) {
            by_id.insert(guide.id.clone(), guide);
        }
    }
    if let Some(workspace) = &context.workspace {
        for guide in read_guides_from_root(
            &workspace.join(".arroba").join(METAAGENT_GUIDE_DIR),
            MetaagentGuideSourceScope::Workspace,
            true,
        ) {
            by_id.insert(guide.id.clone(), guide);
        }
    }
    by_id.into_values().collect()
}

fn embedded_guides() -> Vec<EffectiveMetaagentGuide> {
    METAAGENT_GUIDES
        .iter()
        .map(|guide| EffectiveMetaagentGuide {
            id: guide.id.to_string(),
            title: guide.title.to_string(),
            summary: guide.summary.to_string(),
            tags: guide.tags.iter().map(|tag| (*tag).to_string()).collect(),
            commands: guide
                .commands
                .iter()
                .map(|command| (*command).to_string())
                .collect(),
            body: guide_body(guide),
            source_scope: MetaagentGuideSourceScope::Embedded,
            source_path: None,
        })
        .collect()
}

fn guide_body(guide: &MetaagentGuide) -> String {
    if guide.id == "workflows/workflow-code-patterns" {
        let mut body = guide.body.to_string();
        for example in crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES {
            body.push_str("\n\n## ");
            body.push_str(example.title);
            body.push('\n');
            body.push_str(example.summary);
            body.push_str("\n\nSource: `");
            body.push_str(example.path);
            body.push_str("`\n\n```js\n");
            body.push_str(example.source.trim());
            body.push_str("\n```");
        }
        body
    } else {
        guide.body.to_string()
    }
}

fn score_guide(query: &str, guide: &EffectiveMetaagentGuide) -> usize {
    let haystack = normalize_search_text(&format!(
        "{} {} {} {} {}",
        guide.id,
        guide.title,
        guide.summary,
        guide.tags.join(" "),
        guide.commands.join(" ")
    ));
    let body = normalize_search_text(&guide.body);
    query
        .split_whitespace()
        .map(|term| {
            let mut score = 0;
            if normalize_search_text(&guide.id).contains(term) {
                score += 8;
            }
            if normalize_search_text(&guide.title).contains(term) {
                score += 6;
            }
            if haystack.contains(term) {
                score += 3;
            }
            if body.contains(term) {
                score += 1;
            }
            score
        })
        .sum()
}

fn read_guides_from_root(
    root: &Path,
    source_scope: MetaagentGuideSourceScope,
    skip_builtin_dir: bool,
) -> Vec<EffectiveMetaagentGuide> {
    let mut guides = Vec::new();
    collect_guide_files(root, root, source_scope, skip_builtin_dir, &mut guides);
    guides
}

fn collect_guide_files(
    root: &Path,
    current: &Path,
    source_scope: MetaagentGuideSourceScope,
    skip_builtin_dir: bool,
    guides: &mut Vec<EffectiveMetaagentGuide>,
) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if skip_builtin_dir
                && path.file_name().and_then(|name| name.to_str()) == Some(BUILTIN_GUIDE_DIR)
            {
                continue;
            }
            collect_guide_files(root, &path, source_scope, skip_builtin_dir, guides);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if let Some(guide) = read_file_guide(root, &path, source_scope) {
            guides.push(guide);
        }
    }
}

fn read_file_guide(
    root: &Path,
    path: &Path,
    source_scope: MetaagentGuideSourceScope,
) -> Option<EffectiveMetaagentGuide> {
    let source = std::fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&source);
    let metadata = frontmatter
        .and_then(|frontmatter| serde_yaml::from_str::<MetaagentGuideFrontmatter>(frontmatter).ok())
        .unwrap_or_default();
    let fallback_id = guide_id_from_path(root, path)?;
    let body = body.trim_start_matches('\n').to_string();
    let title = metadata
        .title
        .or_else(|| first_markdown_heading(&body))
        .unwrap_or_else(|| fallback_id.clone());
    Some(EffectiveMetaagentGuide {
        id: metadata.id.unwrap_or(fallback_id),
        title,
        summary: metadata.summary.unwrap_or_default(),
        tags: metadata.tags,
        commands: metadata.commands,
        body,
        source_scope,
        source_path: Some(path.to_path_buf()),
    })
}

fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, source);
    };
    let body_start = end + "\n---".len();
    (Some(&rest[..end]), &rest[body_start..])
}

fn guide_id_from_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut without_extension = relative.to_path_buf();
    without_extension.set_extension("");
    Some(
        without_extension
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn first_markdown_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

fn seed_builtin_guide_copies(user_root: &Path) -> std::io::Result<()> {
    let root = user_root.join(BUILTIN_GUIDE_DIR);
    for guide in METAAGENT_GUIDES {
        let path = root.join(format!("{}.md", guide.id));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = builtin_guide_file(guide);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(payload.as_str()) {
            continue;
        }
        std::fs::write(path, payload)?;
    }
    Ok(())
}

fn builtin_guide_file(guide: &MetaagentGuide) -> String {
    let mut payload = String::new();
    payload.push_str("---\n");
    payload.push_str("id: ");
    payload.push_str(guide.id);
    payload.push('\n');
    payload.push_str("title: ");
    payload.push_str(&yaml_string(guide.title));
    payload.push('\n');
    payload.push_str("summary: ");
    payload.push_str(&yaml_string(guide.summary));
    payload.push('\n');
    payload.push_str("tags:\n");
    for tag in guide.tags {
        payload.push_str("  - ");
        payload.push_str(&yaml_string(tag));
        payload.push('\n');
    }
    payload.push_str("commands:\n");
    for command in guide.commands {
        payload.push_str("  - ");
        payload.push_str(&yaml_string(command));
        payload.push('\n');
    }
    payload.push_str("---\n");
    let body = guide_body(guide);
    payload.push_str(body.trim_start_matches('\n'));
    payload
}

fn yaml_string(value: &str) -> String {
    serde_yaml::to_string(value)
        .unwrap_or_else(|_| format!("{value:?}"))
        .trim()
        .to_string()
}

fn arroba_home() -> Option<PathBuf> {
    std::env::var_os("ARROBA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".arroba")))
}

fn normalize_search_text(value: &str) -> String {
    value.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_guides_finds_workflow_construction_help() {
        let guides = search_guides(MetaagentGuideSearchArgs {
            query: Some("create endpoint run workflow".to_string()),
            tag: Some("workflow".to_string()),
            command: None,
            limit: Some(5),
        });
        assert!(guides.iter().any(|guide| {
            guide.get("id").and_then(serde_json::Value::as_str)
                == Some("workflows/basic-components")
        }));
    }

    #[test]
    fn read_guide_returns_markdown_body() {
        let guide = read_guide("agent-apps/generate-app").expect("guide should exist");
        assert!(guide
            .get("body")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|body| body.contains("Do not implement directly")));
    }

    #[test]
    fn workflow_code_guide_is_searchable_by_meta_tool_command() {
        for command in [
            "arroba.meta.workflow_code.create",
            "arroba.meta.workflow_code.read",
            "arroba.meta.workflow_code.list",
            "arroba.meta.workflow_code.update",
            "arroba.meta.workflow_code.delete",
            "arroba.meta.workflow_code.validate",
            "arroba.meta.workflow_code.apply",
            "arroba.meta.workflow_code.run",
            "arroba.meta.workflow_code.export",
            "arroba.meta.workflow_code.import",
        ] {
            let guides = search_guides(MetaagentGuideSearchArgs {
                query: Some("workflow code javascript builder".to_string()),
                tag: Some("workflow-code".to_string()),
                command: Some(command.to_string()),
                limit: Some(5),
            });
            assert!(
                guides.iter().any(|guide| {
                    guide.get("id").and_then(serde_json::Value::as_str)
                        == Some("workflows/workflow-code-authoring")
                }),
                "workflow-code authoring guide should be discoverable for `{command}`"
            );
        }
    }

    #[test]
    fn workflow_code_guide_documents_real_builder_api() {
        let guide = read_guide("workflows/workflow-code-authoring").expect("guide should exist");
        let body = guide
            .get("body")
            .and_then(serde_json::Value::as_str)
            .expect("guide body should be markdown");
        for expected in [
            "workflow.define",
            "workflow.schema",
            "workflow.schemaFromFile",
            "workflow.newAgent",
            "workflow.existingAgent",
            "workflow.node",
            "workflow.edge",
            "workflow.endpoint",
            "workflow.queue",
            "workflow.watchdog",
        ] {
            assert!(body.contains(expected), "missing `{expected}` from guide");
        }
        for unsupported in ["workflow.run", "workflow.enqueue"] {
            assert!(
                !body.contains(unsupported),
                "guide should not document unsupported builder method `{unsupported}`"
            );
        }
    }

    #[test]
    fn workflow_code_guide_documents_portability_and_extension_rules() {
        let guide = read_guide("workflows/workflow-code-authoring").expect("guide should exist");
        let body = guide
            .get("body")
            .and_then(serde_json::Value::as_str)
            .expect("guide body should be markdown");
        for expected in [
            "provider_rebindings",
            "Do not include runtime ids in provider rebindings",
            "Do not rebind existing-agent nodes",
            "Generated runtime ids are never authored in the script",
            "Supported `kind` values are `\"mcp\"`, `\"skill\"`, `\"script\"`, and `\"connector\"`",
            "Script grants must include `environment`",
            "If no queues are defined",
            "Queue aliases must be unique after normalization",
            "Use `queue: \"default\"`",
            "with `endpoint`, optional `queue`, and `prompt`",
            "`run` may pass `endpoint`, `queue`, and `prompt`",
            "arroba.meta.workflow_code.source_export_directory",
            "`workflow.js`, `schemas/*.json`, and `manifest.json`",
            "endpoint and queue values may be script handles",
            "`validationPolicy` (`\"warn\"` or `\"halt\"`)",
            "`policy` (`\"skip\"` or `\"queue\"`)",
        ] {
            assert!(body.contains(expected), "missing `{expected}` from guide");
        }
        assert!(
            !body.contains("optional `queue_ref`"),
            "metaagent guide should document the real workflow_code.run `queue` argument, not `queue_ref`"
        );
    }

    #[test]
    fn workflow_code_pattern_guide_embeds_canonical_examples() {
        let guide = read_guide("workflows/workflow-code-patterns").expect("guide should exist");
        let body = guide
            .get("body")
            .and_then(serde_json::Value::as_str)
            .expect("guide body should be markdown");
        for example in crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES {
            assert!(
                body.contains(example.path),
                "missing path `{}` from guide",
                example.path
            );
            assert!(
                body.contains(example.source.trim()),
                "missing source for `{}` from guide",
                example.slug
            );
        }
    }

    #[test]
    fn workflow_code_pattern_guide_is_searchable_by_embedded_pattern_names() {
        for query in [
            "workflow-code fan-out synthesize",
            "workflow-code adversarial verification",
            "workflow-code tournament",
            "workflow-code evaluator optimizer",
        ] {
            let guides = search_guides(MetaagentGuideSearchArgs {
                query: Some(query.to_string()),
                tag: Some("workflow-code".to_string()),
                command: Some("arroba.meta.workflow_code.validate".to_string()),
                limit: Some(5),
            });
            assert!(
                guides.iter().any(|guide| {
                    guide.get("id").and_then(serde_json::Value::as_str)
                        == Some("workflows/workflow-code-patterns")
                }),
                "workflow-code pattern guide should be discoverable for `{query}`"
            );
        }
    }

    #[test]
    fn contextual_guides_use_workspace_user_builtin_embedded_precedence() {
        let root = temp_guide_root("precedence");
        let workspace = root.join("workspace");
        let user_root = root.join("user-guides");
        let guide_rel = "workflows/basic-components.md";
        write_test_guide(
            &user_root.join(BUILTIN_GUIDE_DIR).join(guide_rel),
            "workflows/basic-components",
            "Built-in Copy Components",
            "builtin body",
        );
        write_test_guide(
            &user_root.join(guide_rel),
            "workflows/basic-components",
            "User Components",
            "user body",
        );
        write_test_guide(
            &workspace
                .join(".arroba")
                .join(METAAGENT_GUIDE_DIR)
                .join(guide_rel),
            "workflows/basic-components",
            "Workspace Components",
            "workspace body",
        );
        let context = MetaagentGuideContext::for_test(workspace.clone(), user_root.clone(), false);

        let guide =
            read_guide_with_context("workflows/basic-components", &context).expect("guide exists");
        assert_eq!(
            guide.get("title").and_then(serde_json::Value::as_str),
            Some("Workspace Components")
        );
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("workspace")
        );

        std::fs::remove_file(
            workspace
                .join(".arroba")
                .join(METAAGENT_GUIDE_DIR)
                .join(guide_rel),
        )
        .expect("workspace guide should remove");
        let guide =
            read_guide_with_context("workflows/basic-components", &context).expect("guide exists");
        assert_eq!(
            guide.get("title").and_then(serde_json::Value::as_str),
            Some("User Components")
        );
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("user")
        );

        std::fs::remove_file(user_root.join(guide_rel)).expect("user guide should remove");
        let guide =
            read_guide_with_context("workflows/basic-components", &context).expect("guide exists");
        assert_eq!(
            guide.get("title").and_then(serde_json::Value::as_str),
            Some("Built-in Copy Components")
        );
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("builtin_copy")
        );

        std::fs::remove_file(user_root.join(BUILTIN_GUIDE_DIR).join(guide_rel))
            .expect("builtin copy guide should remove");
        let guide =
            read_guide_with_context("workflows/basic-components", &context).expect("guide exists");
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("embedded")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn contextual_guide_search_reports_source_scope() {
        let root = temp_guide_root("search-scope");
        let workspace = root.join("workspace");
        let user_root = root.join("user-guides");
        write_test_guide(
            &user_root.join("workflows/custom-routing.md"),
            "workflows/custom-routing",
            "Custom Routing Guide",
            "routing body with specialist selection",
        );
        let context = MetaagentGuideContext::for_test(workspace, user_root, false);
        let guides = search_guides_with_context(
            MetaagentGuideSearchArgs {
                query: Some("specialist selection".to_string()),
                tag: Some("workflow".to_string()),
                command: Some("workflow run".to_string()),
                limit: Some(5),
            },
            &context,
        );
        let guide = guides
            .iter()
            .find(|guide| {
                guide.get("id").and_then(serde_json::Value::as_str)
                    == Some("workflows/custom-routing")
            })
            .expect("custom guide should be searchable");
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("user")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn contextual_guides_seed_builtin_copies_to_user_root() {
        let root = temp_guide_root("seed");
        let workspace = root.join("workspace");
        let user_root = root.join("user-guides");
        let context = MetaagentGuideContext::for_test(workspace, user_root.clone(), true);
        let guide = read_guide_with_context("workflows/workflow-code-patterns", &context)
            .expect("seeded guide should exist");
        assert_eq!(
            guide
                .get("source_scope")
                .and_then(serde_json::Value::as_str),
            Some("builtin_copy")
        );
        let seeded_path = user_root
            .join(BUILTIN_GUIDE_DIR)
            .join("workflows")
            .join("workflow-code-patterns.md");
        let seeded = std::fs::read_to_string(seeded_path).expect("builtin guide should be seeded");
        for example in crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES {
            assert!(
                seeded.contains(example.source.trim()),
                "seeded built-in copy should include source for `{}`",
                example.slug
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn write_test_guide(path: &std::path::Path, id: &str, title: &str, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("guide parent should create");
        }
        std::fs::write(
            path,
            format!(
                "---\nid: {id}\ntitle: {title:?}\nsummary: \"test guide\"\ntags:\n  - workflow\ncommands:\n  - workflow run\n---\n# {title}\n\n{body}\n"
            ),
        )
        .expect("guide should write");
    }

    fn temp_guide_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-metaagent-guide-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }
}
