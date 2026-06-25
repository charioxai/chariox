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
            "arroba.meta.workflow_code.update",
            "arroba.meta.workflow_code.validate",
            "arroba.meta.workflow_code.apply",
            "arroba.meta.workflow_code.run",
            "arroba.meta.workflow_code.export",
            "arroba.meta.workflow_code.import",
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

#[derive(Debug, Clone, Default)]
pub(crate) struct MetaagentGuideSearchArgs {
    pub query: Option<String>,
    pub tag: Option<String>,
    pub command: Option<String>,
    pub limit: Option<usize>,
}

pub(crate) fn list_guides(args: MetaagentGuideSearchArgs) -> Vec<serde_json::Value> {
    search_guides(args).into_iter().collect()
}

pub(crate) fn search_guides(args: MetaagentGuideSearchArgs) -> Vec<serde_json::Value> {
    let query = args.query.as_deref().map(normalize_search_text);
    let tag = args.tag.as_deref().map(str::to_ascii_lowercase);
    let command = args.command.as_deref().map(normalize_search_text);
    let mut matches = METAAGENT_GUIDES
        .iter()
        .filter_map(|guide| {
            if let Some(tag) = tag.as_deref() {
                if !guide.tags.iter().any(|candidate| *candidate == tag) {
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
                .map(|query| score_guide(query, guide))
                .unwrap_or(1);
            (score > 0).then_some((score, guide))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.title.cmp(right.title))
    });
    matches
        .into_iter()
        .take(args.limit.unwrap_or(20).clamp(1, 100))
        .map(|(_, guide)| guide_summary(guide, false))
        .collect()
}

pub(crate) fn read_guide(guide_ref: &str) -> Option<serde_json::Value> {
    let needle = normalize_search_text(guide_ref);
    METAAGENT_GUIDES
        .iter()
        .find(|guide| {
            normalize_search_text(guide.id) == needle
                || normalize_search_text(guide.title) == needle
        })
        .map(|guide| guide_summary(guide, true))
}

fn guide_summary(guide: &MetaagentGuide, include_body: bool) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "id": guide.id,
        "title": guide.title,
        "summary": guide.summary,
        "tags": guide.tags,
        "commands": guide.commands,
    });
    if include_body {
        payload["body"] = serde_json::Value::String(guide_body(guide));
    }
    payload
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

fn score_guide(query: &str, guide: &MetaagentGuide) -> usize {
    let haystack = normalize_search_text(&format!(
        "{} {} {} {} {}",
        guide.id,
        guide.title,
        guide.summary,
        guide.tags.join(" "),
        guide.commands.join(" ")
    ));
    let body = normalize_search_text(guide.body);
    query
        .split_whitespace()
        .map(|term| {
            let mut score = 0;
            if normalize_search_text(guide.id).contains(term) {
                score += 8;
            }
            if normalize_search_text(guide.title).contains(term) {
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
        let guides = search_guides(MetaagentGuideSearchArgs {
            query: Some("workflow code javascript builder".to_string()),
            tag: Some("workflow-code".to_string()),
            command: Some("arroba.meta.workflow_code.apply".to_string()),
            limit: Some(5),
        });
        assert!(guides.iter().any(|guide| {
            guide.get("id").and_then(serde_json::Value::as_str)
                == Some("workflows/workflow-code-authoring")
        }));
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
            "`validationPolicy` (`\"warn\"` or `\"halt\"`)",
            "`policy` (`\"skip\"` or `\"queue\"`)",
        ] {
            assert!(body.contains(expected), "missing `{expected}` from guide");
        }
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
}
