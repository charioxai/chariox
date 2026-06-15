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
        payload["body"] = serde_json::Value::String(guide.body.to_string());
    }
    payload
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
}
