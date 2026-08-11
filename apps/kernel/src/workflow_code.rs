use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

use crate::config::WorkflowCodeLimitsConfig;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::mcp::validate_registry_name;
use crate::session::{
    RuntimeSession, WorkflowEdgeEndpointSide, WorkflowHandoffValidationPolicy,
    WorkflowScheduleOverlapPolicy, WorkflowScheduleTrigger,
};

mod artifact_registry;
mod common;
mod compiler;
mod compiler_runtime;
mod definition;
mod model;
mod rebinding;
mod source_export;
#[cfg(test)]
mod tests;
mod validation;
mod workflow_registry;

pub use common::workflow_code_definition_sha256_hex;
pub use compiler::*;
pub use definition::*;
pub use model::*;
pub use rebinding::*;
pub use source_export::{
    export_workflow_code_package_from_session_workflow,
    export_workflow_code_source_from_session_workflow,
};
pub(crate) use validation::{
    attach_workflow_code_diagnostic_spans, workflow_code_materialized_queue_count,
};
pub use workflow_registry::{
    enrich_workflow_registry_entry_summary, workflow_registry_metadata_with_summary_failure,
};

use common::*;
use compiler_runtime::*;
use source_export::*;
use validation::*;
#[cfg(test)]
use workflow_registry::builtin_workflow_registry_metadata;

pub const WORKFLOW_CODE_SCHEMA_VERSION: u32 = 1;
pub const WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION: u32 = 2;
pub const WORKFLOW_CODE_SOURCE_EXPORT_MANIFEST_VERSION: u32 = 1;
pub const WORKFLOW_REGISTRY_MANIFEST_VERSION: u32 = 1;
pub const WORKFLOW_CODE_ARTIFACT_SOURCE_KIND: &str = "workflow_code";
pub const WORKFLOW_CODE_CANVAS_COORDINATE_SPACE: &str = "workflow-canvas-v1";
pub const WORKFLOW_CODE_CANVAS_NODE_WIDTH: i64 = 232;
pub const WORKFLOW_CODE_CANVAS_NODE_HEIGHT: i64 = 96;
pub const WORKFLOW_CODE_CANVAS_ENDPOINT_WIDTH: i64 = 180;
pub const WORKFLOW_CODE_CANVAS_ENDPOINT_HEIGHT: i64 = 78;
pub const WORKFLOW_CODE_CANVAS_EXIT_MARKER_WIDTH: i64 = 120;
pub const WORKFLOW_CODE_CANVAS_EXIT_MARKER_HEIGHT: i64 = 72;
pub const WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_X: i64 = 268;
pub const WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_Y: i64 = 28;
pub const WORKFLOW_CODE_CANVAS_MIN_GAP: i64 = 36;
pub const WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_X: i64 = 320;
pub const WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_Y: i64 = 160;
pub const WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_X: i64 = -220;
pub const WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_Y: i64 = 0;
pub(crate) const WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS: usize = 1000;
const WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy)]
pub struct WorkflowCodePatternExample {
    pub slug: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub path: &'static str,
    pub source: &'static str,
}

pub const WORKFLOW_CODE_PATTERN_EXAMPLES: &[WorkflowCodePatternExample] = &[
    WorkflowCodePatternExample {
        slug: "prompt-chaining",
        title: "Prompt chaining",
        summary: "Two nodes: a drafter hands a structured draft to a refiner.",
        path: "examples/workflow-code/prompt-chaining.js",
        source: include_str!("../../../examples/workflow-code/prompt-chaining.js"),
    },
    WorkflowCodePatternExample {
        slug: "routing",
        title: "Classify and act / routing",
        summary: "A classifier routes work to one of two specialist nodes.",
        path: "examples/workflow-code/routing.js",
        source: include_str!("../../../examples/workflow-code/routing.js"),
    },
    WorkflowCodePatternExample {
        slug: "fan-out-synthesize",
        title: "Fan-out and synthesize",
        summary: "A planner fans out to two workers, then a synthesizer waits for both inputs.",
        path: "examples/workflow-code/fan-out-synthesize.js",
        source: include_str!("../../../examples/workflow-code/fan-out-synthesize.js"),
    },
    WorkflowCodePatternExample {
        slug: "parallelization",
        title: "Parallelization",
        summary: "A dispatcher sends the same task to two reviewers, then an aggregator waits for both votes.",
        path: "examples/workflow-code/parallelization.js",
        source: include_str!("../../../examples/workflow-code/parallelization.js"),
    },
    WorkflowCodePatternExample {
        slug: "adversarial-verification",
        title: "Adversarial verification",
        summary: "A proposer, critic, and judge collaborate on adversarial verification.",
        path: "examples/workflow-code/adversarial-verification.js",
        source: include_str!("../../../examples/workflow-code/adversarial-verification.js"),
    },
    WorkflowCodePatternExample {
        slug: "generate-filter",
        title: "Generate and filter",
        summary: "A generator creates candidates, a filter selects them, and a finisher completes.",
        path: "examples/workflow-code/generate-filter.js",
        source: include_str!("../../../examples/workflow-code/generate-filter.js"),
    },
    WorkflowCodePatternExample {
        slug: "tournament",
        title: "Tournament",
        summary: "A seeder fans out to two contestants, then a judge selects a winner.",
        path: "examples/workflow-code/tournament.js",
        source: include_str!("../../../examples/workflow-code/tournament.js"),
    },
    WorkflowCodePatternExample {
        slug: "loop-until-done",
        title: "Loop until done",
        summary: "A worker and checker loop until the checker accepts final output.",
        path: "examples/workflow-code/loop-until-done.js",
        source: include_str!("../../../examples/workflow-code/loop-until-done.js"),
    },
    WorkflowCodePatternExample {
        slug: "planner-worker-reviewer",
        title: "Planner-worker-reviewer",
        summary: "A planner assigns work, a worker implements it, and a reviewer routes revisions or accepted steps back to the planner.",
        path: "examples/workflow-code/planner-worker-reviewer.js",
        source: include_str!("../../../examples/workflow-code/planner-worker-reviewer.js"),
    },
    WorkflowCodePatternExample {
        slug: "orchestrator-workers",
        title: "Orchestrator-workers",
        summary: "An orchestrator delegates to a worker and a synthesizer produces final output.",
        path: "examples/workflow-code/orchestrator-workers.js",
        source: include_str!("../../../examples/workflow-code/orchestrator-workers.js"),
    },
    WorkflowCodePatternExample {
        slug: "evaluator-optimizer",
        title: "Evaluator-optimizer",
        summary: "An optimizer produces candidates and an evaluator loops back or accepts.",
        path: "examples/workflow-code/evaluator-optimizer.js",
        source: include_str!("../../../examples/workflow-code/evaluator-optimizer.js"),
    },
];

pub fn workflow_code_canvas_contract() -> Value {
    serde_json::json!({
        "coordinate_space": WORKFLOW_CODE_CANVAS_COORDINATE_SPACE,
        "node": {
            "width": WORKFLOW_CODE_CANVAS_NODE_WIDTH,
            "height": WORKFLOW_CODE_CANVAS_NODE_HEIGHT,
        },
        "endpoint": {
            "width": WORKFLOW_CODE_CANVAS_ENDPOINT_WIDTH,
            "height": WORKFLOW_CODE_CANVAS_ENDPOINT_HEIGHT,
        },
        "exit_marker": {
            "width": WORKFLOW_CODE_CANVAS_EXIT_MARKER_WIDTH,
            "height": WORKFLOW_CODE_CANVAS_EXIT_MARKER_HEIGHT,
            "offset_from_node": {
                "x": WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_X,
                "y": WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_Y,
            },
        },
        "minimum_gap": WORKFLOW_CODE_CANVAS_MIN_GAP,
        "recommended_node_grid": {
            "x": WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_X,
            "y": WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_Y,
        },
        "default_endpoint_offset": {
            "x": WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_X,
            "y": WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_Y,
        },
        "validation": {
            "explicit_coordinates_only": true,
            "checks": ["nodes", "endpoints", "exit_markers"],
        },
    })
}
