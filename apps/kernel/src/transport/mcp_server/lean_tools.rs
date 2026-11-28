//! A provider that cannot defer MCP tools itself lists only its upfront tools (`agent_upfront_tools.json`) and
//! `chariox.tools.find`. Finding a tool adds it to that run's listing and the run is told with the standard
//! `notifications/tools/list_changed`. The set lives and dies with the run's token: a new run starts lean again.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::Value;

use super::tool_loading::upfront_names;
use crate::transport::runtime_tools::RuntimeToolSpec;

pub(super) const FIND_TOOL: &str = "chariox.tools.find";
const FIND_LIMIT: usize = 8;

/// How one lean run lists its tools.
#[derive(Debug, Clone)]
pub(super) struct LeanListing {
    provider: String,
    sub_agent: bool,
    loads: LeanLoads,
}

/// The tools each run of one kernel has loaded through `chariox.tools.find`, by run token.
#[derive(Clone, Default, Debug)]
pub(crate) struct LeanLoads(std::sync::Arc<Mutex<BTreeMap<String, BTreeSet<String>>>>);

impl LeanLoads {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, BTreeSet<String>>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Forgets the loaded tools of runs that ended.
    pub(crate) fn retain_runs(&self, running: &BTreeSet<String>) {
        self.lock().retain(|token, _| running.contains(token));
    }
}

impl LeanListing {
    /// `Some` for a run whose provider's upfront list names tools; `None` for Claude (it defers by itself) and for a
    /// provider whose list is `"*"`.
    pub(super) fn for_run(
        router: &crate::runtime::router::CommandRouter,
        auth_token: &str,
    ) -> Option<Self> {
        let run = router.runtime_mcp_provider_run(auth_token)?;
        let provider = run.adapter_key();
        if provider == "claude" {
            return None;
        }
        let lean = Self {
            provider: provider.to_owned(),
            sub_agent: run.is_sub_agent(),
            loads: router.lean_loads().clone(),
        };
        (!lean.upfront().is_empty()).then_some(lean)
    }

    fn upfront(&self) -> &'static [String] {
        upfront_names(
            &self.provider,
            if self.sub_agent { "sub_agent" } else { "agent" },
        )
    }

    fn lists(&self, token: &str, name: &str) -> bool {
        self.upfront().iter().any(|upfront| upfront == name)
            || upfront_names(&self.provider, "workflow_turn")
                .iter()
                .any(|upfront| upfront == name)
            || self
                .loads
                .lock()
                .get(token)
                .is_some_and(|names| names.contains(name))
    }

    /// The tools this run lists now, from the full authorised catalog.
    pub(super) fn view(&self, token: &str, tools: Vec<RuntimeToolSpec>) -> Vec<RuntimeToolSpec> {
        let mut view: Vec<RuntimeToolSpec> = tools
            .into_iter()
            .filter(|tool| self.lists(token, &tool.name))
            .collect();
        view.push(find_spec());
        view
    }

    /// The same view of a `(name, description, schema)` listing.
    pub(super) fn view_listing(
        &self,
        token: &str,
        listing: Option<Vec<(String, String, Value)>>,
    ) -> Option<Vec<(String, String, Value)>> {
        listing.map(|tools| {
            tools
                .into_iter()
                .filter(|tool| self.lists(token, &tool.0))
                .collect()
        })
    }

    /// Loads the tools `arguments` ask for out of `catalog` and says what happened.
    pub(super) fn find(
        &self,
        token: &str,
        arguments: &Value,
        catalog: &[RuntimeToolSpec],
    ) -> Value {
        let mut wanted: Vec<&RuntimeToolSpec> = Vec::new();
        for name in arguments["names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            wanted.extend(catalog.iter().filter(|tool| tool.name == name));
        }
        let words: Vec<String> = arguments["query"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.len() > 1)
            .map(str::to_owned)
            .collect();
        if !words.is_empty() {
            let mut scored: Vec<(usize, &RuntimeToolSpec)> = catalog
                .iter()
                .filter_map(|tool| {
                    let name = tool.name.to_lowercase();
                    let description = tool.description.to_lowercase();
                    let score: usize = words
                        .iter()
                        .map(|word| {
                            3 * usize::from(name.contains(word))
                                + usize::from(description.contains(word))
                        })
                        .sum();
                    (score > 0).then_some((score, tool))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
            wanted.extend(scored.into_iter().map(|(_, tool)| tool));
        }
        if wanted.is_empty() && words.is_empty() && arguments["names"].is_null() {
            let unloaded: Vec<&str> = catalog
                .iter()
                .filter(|tool| !self.lists(token, &tool.name))
                .map(|tool| tool.name.as_str())
                .collect();
            return serde_json::json!({ "available": unloaded });
        }
        let mut seen = BTreeSet::new();
        wanted.retain(|tool| seen.insert(tool.name.clone()));
        wanted.truncate(FIND_LIMIT);
        let mut set = self.loads.lock();
        let entry = set.entry(token.to_owned()).or_default();
        let found: Vec<Value> = wanted
            .iter()
            .map(|tool| {
                entry.insert(tool.name.clone());
                serde_json::json!({"name": tool.name, "description": tool.description})
            })
            .collect();
        serde_json::json!({ "loaded": found })
    }
}

pub(super) fn find_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: FIND_TOOL.to_owned(),
        description: "Load Chariox tools by name or topic; they become callable.".to_owned(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "names": {"type": "array", "items": {"type": "string"}}
            },
            "additionalProperties": false
        }),
    }
}
