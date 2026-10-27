use super::*;

#[test]
fn compiles_javascript_builder_source() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
const finalSchema = workflow.schema({
  alias: "Final",
  schema: {
type: "object",
properties: { answer: { type: "string" } },
required: ["answer"],
additionalProperties: false
  }
})
workflow.define({
  alias: "compiled",
  prompt: "Run the compiled workflow.",
  maxConcurrent: 2,
  runOutputSchema: finalSchema
})
const planner = workflow.node({
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  publicLabel: "Planner",
  instructions: "Plan.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(planner, { alias: "entry" })
"#;

    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(
        result.definition.workflow.alias.as_deref(),
        Some("compiled")
    );
    assert_eq!(
        result.definition.workflow.prompt.as_deref(),
        Some("Run the compiled workflow.")
    );
    assert_eq!(result.definition.nodes.len(), 1);
    assert_eq!(result.definition.endpoints.len(), 1);
    assert_eq!(result.definition.schemas.len(), 1);
}

#[test]
fn javascript_compiler_resolves_parameter_defaults() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
const params = workflow.parameters({
  schema: {
type: "object",
properties: {
  worker_count: { type: "integer", minimum: 1, default: 2, title: "Worker count" }
},
additionalProperties: false
  }
})
workflow.define({ alias: "parameterized", maxConcurrent: params.worker_count })
let previous = null
for (let index = 0; index < params.worker_count; index += 1) {
  const worker = workflow.node({
handle: `worker_${index + 1}`,
agent: workflow.newAgent({ provider: "dev-stub" }),
publicLabel: `Worker ${index + 1}`
  })
  if (index === 0) workflow.endpoint(worker, { handle: "entry" })
  if (previous) workflow.edge(previous, worker)
  previous = worker
}
"#;

    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(result.definition.nodes.len(), 2);
    assert_eq!(result.definition.workflow.max_concurrent, Some(2));
    assert_eq!(
        result
            .definition
            .parameters_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/worker_count/type")),
        Some(&serde_json::json!("integer"))
    );
}

#[test]
fn javascript_compiler_applies_explicit_parameters() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
const params = workflow.parameters({
  schema: {
type: "object",
properties: {
  worker_count: { type: "integer", minimum: 1, default: 2 }
},
additionalProperties: false
  }
})
workflow.define({ alias: "parameterized", maxConcurrent: params.worker_count })
let previous = null
for (let index = 0; index < params.worker_count; index += 1) {
  const worker = workflow.node({
handle: `worker_${index + 1}`,
agent: workflow.newAgent({ provider: "dev-stub" }),
publicLabel: `Worker ${index + 1}`
  })
  if (index === 0) workflow.endpoint(worker, { handle: "entry" })
  if (previous) workflow.edge(previous, worker)
  previous = worker
}
"#;
    let parameters = BTreeMap::from([("worker_count".to_string(), serde_json::json!(4))]);

    let result = compile_workflow_code_javascript_with_parameters(
        node,
        source,
        &WorkflowCodeLimitsConfig::default(),
        &parameters,
    )
    .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(result.definition.nodes.len(), 4);
    assert_eq!(result.definition.workflow.max_concurrent, Some(4));
}

#[test]
fn javascript_compiler_rejects_non_power_of_two_parameter() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
const params = workflow.parameters({
  schema: {
type: "object",
properties: {
  bracket_size: { type: "integer", minimum: 2, xPowerOfTwo: true, default: 2 }
},
additionalProperties: false
  }
})
workflow.define({ alias: "tournament", maxConcurrent: params.bracket_size })
"#;
    let parameters = BTreeMap::from([("bracket_size".to_string(), serde_json::json!(3))]);

    let error = compile_workflow_code_javascript_with_parameters(
        node,
        source,
        &WorkflowCodeLimitsConfig::default(),
        &parameters,
    )
    .expect_err("non-power-of-two parameter should fail");

    assert!(format!("{error}").contains("power of two"));
}

#[test]
fn javascript_compiler_ignores_source_console_output() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
console.log("do not leak this into the compile result")
console.error("do not leak this either")
workflow.define({ alias: "silent_console" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(
        result.definition.workflow.alias.as_deref(),
        Some("silent_console")
    );
    assert!(
        result.logs.is_empty(),
        "workflow-code source console output must not be surfaced in compile results"
    );
}

#[test]
fn compiles_javascript_queues_and_schedules() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
const finalSchema = workflow.schema({
  handle: "final",
  schema: {
type: "object",
required: ["answer"],
properties: { answer: { type: "string" } },
additionalProperties: false
  }
})
workflow.define({ alias: "queued_schedule_flow", runOutputSchema: finalSchema })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
const entry = workflow.endpoint(worker, { handle: "entry", alias: "entry" })
const urgent = workflow.queue({ handle: "urgent", alias: "urgent", priority: 5, enabled: false })
workflow.schedule(entry, {
  handle: "schedule_entry",
  queue: urgent,
  enabled: false,
  cron: "15 30 14 * * *",
  timezone: "UTC",
  invocationPrompt: "Check for queued work.",
  overlap: "skip",
  maxRuns: 2
})
"#;

    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(result.definition.queues.len(), 1);
    assert_eq!(result.definition.queues[0].handle, "urgent");
    assert_eq!(result.definition.queues[0].priority, 5);
    assert!(!result.definition.queues[0].enabled);
    assert_eq!(result.definition.schedules.len(), 1);
    assert_eq!(result.definition.schedules[0].endpoint, "entry");
    assert_eq!(
        result.definition.schedules[0].trigger,
        WorkflowScheduleTrigger::cron("15 30 14 * * *", "UTC")
    );
    assert_eq!(
        result.definition.schedules[0].queue.as_deref(),
        Some("urgent")
    );
    assert_eq!(
        result.definition.schedules[0].overlap_policy,
        WorkflowScheduleOverlapPolicy::Skip
    );
    assert_eq!(result.definition.schedules[0].enabled, Some(false));
    assert_eq!(result.definition.schedules[0].max_runs, Some(2));
}

#[test]
fn compiles_javascript_connector_extension_with_js_safety_spelling() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let source = r#"
workflow.define({ alias: "connector_extension_flow" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true,
  extensions: [
{ kind: "connector", name: "linear", credential: "linear-api", maxSafety: "read" }
  ]
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("workflow-code JS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    let extension = result
        .definition
        .nodes
        .first()
        .and_then(|node| node.extensions.first())
        .expect("compiled worker should retain extension grant");
    assert_eq!(extension.kind, ExtensionKind::Connector);
    assert_eq!(extension.name, "linear");
    assert_eq!(extension.credential.as_deref(), Some("linear-api"));
    assert_eq!(extension.max_safety.as_deref(), Some("read"));
}

#[test]
fn workflow_code_language_serializes_canonical_typescript_name() {
    assert_eq!(
        serde_json::to_value(WorkflowCodeLanguage::TypeScript).expect("language should serialize"),
        serde_json::json!("typescript")
    );
    assert_eq!(
        serde_json::from_value::<WorkflowCodeLanguage>(serde_json::json!("javascript"))
            .expect("friendly JavaScript spelling should decode"),
        WorkflowCodeLanguage::JavaScript
    );
    assert_eq!(
        serde_json::from_value::<WorkflowCodeLanguage>(serde_json::json!("type_script"))
            .expect("legacy spelling should decode"),
        WorkflowCodeLanguage::TypeScript
    );
}

#[test]
fn compiles_typescript_builder_source() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code TS compiler test because node is not available");
        return;
    };

    // MP-08/MP-11: no skip for Node builds without native type stripping; an
    // enum also requires transform mode, as official Node's transform.
    let source = r#"
type ProviderName = "dev-stub";
interface FinalAnswer {
  answer: string;
}
enum Concurrency { One = 1, Two }
const provider: ProviderName = "dev-stub";
const finalSchema = workflow.schema({
  handle: "final",
  schema: {
type: "object",
required: ["answer"],
properties: { answer: { type: "string" } },
additionalProperties: false
  }
})
workflow.define({ alias: "compiled_ts", maxConcurrent: Concurrency.Two, runOutputSchema: finalSchema })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "ts-worker", provider, model: "default" }),
  instructions: "Return a FinalAnswer.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let result = compile_workflow_code_source_with_schema_import_root(
        node,
        source,
        WorkflowCodeLanguage::TypeScript,
        &WorkflowCodeLimitsConfig::default(),
        None,
    )
    .expect("workflow-code TS source should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(
        result.definition.workflow.alias.as_deref(),
        Some("compiled_ts")
    );
    assert_eq!(result.definition.workflow.max_concurrent, Some(2));
    assert_eq!(
        result.definition.workflow.run_output_schema.as_deref(),
        Some("final")
    );
    assert_eq!(
        result.definition.nodes[0].agent,
        WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
            alias: Some("ts-worker".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            account_profile: None,
        })
    );
}

#[test]
fn typescript_stripper_is_pinned_to_official_node_amaro() {
    // amaro 1.1.5 dist/index.js, as embedded by official Node.js 22.22.1.
    assert_eq!(
        sha256_hex(TYPESCRIPT_STRIPPER.as_bytes()),
        "5e05805ae1fa4461b5346c3aa7d8b4690b085e139ffc9fb3c97ae2e1b2219581"
    );
}

#[test]
fn canonical_pattern_examples_compile() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code pattern examples because node is not available");
        return;
    };

    for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
        let result = compile_workflow_code_javascript(
            &node,
            example.source,
            &WorkflowCodeLimitsConfig::default(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "workflow-code pattern example `{}` at `{}` should compile: {error}",
                example.slug, example.path
            )
        });

        assert!(
            result.validation.ok,
            "workflow-code pattern example `{}` should validate: {:?}",
            example.slug, result.validation.diagnostics
        );
        assert!(
            result.definition.workflow.alias.is_some(),
            "workflow-code pattern example `{}` should name the workflow",
            example.slug
        );
        assert!(
            result.definition.workflow.run_output_schema.is_some(),
            "workflow-code pattern example `{}` should define final output schema",
            example.slug
        );
        assert!(
            !result.definition.schemas.is_empty(),
            "workflow-code pattern example `{}` should define schemas",
            example.slug
        );
        assert!(
            !result.definition.nodes.is_empty(),
            "workflow-code pattern example `{}` should define nodes",
            example.slug
        );
        assert!(
            !result.definition.endpoints.is_empty(),
            "workflow-code pattern example `{}` should define endpoints",
            example.slug
        );
        assert!(
            result.definition.parameters_schema.is_some(),
            "workflow-code pattern example `{}` should define input parameters",
            example.slug
        );
    }
}

#[test]
fn tournament_pattern_generates_power_of_two_brackets() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code pattern examples because node is not available");
        return;
    };
    let tournament = WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "tournament")
        .expect("tournament example should be registered");

    let bracket4 = BTreeMap::from([("bracket_size".to_string(), serde_json::json!(4))]);
    let result4 = compile_workflow_code_javascript_with_parameters(
        &node,
        tournament.source,
        &WorkflowCodeLimitsConfig::default(),
        &bracket4,
    )
    .expect("tournament bracket_size=4 should compile");
    assert!(
        result4.validation.ok,
        "{:?}",
        result4.validation.diagnostics
    );
    assert_eq!(
        result4
            .definition
            .nodes
            .iter()
            .filter(|node| node.handle.starts_with("contestant_"))
            .count(),
        4
    );
    assert_eq!(
        result4
            .definition
            .nodes
            .iter()
            .filter(|node| node.handle.contains("judge"))
            .count(),
        3
    );
    assert!(result4
        .definition
        .nodes
        .iter()
        .any(|node| node.handle == "final_judge"));

    let bracket16 = BTreeMap::from([("bracket_size".to_string(), serde_json::json!(16))]);
    let result16 = compile_workflow_code_javascript_with_parameters(
        &node,
        tournament.source,
        &WorkflowCodeLimitsConfig::default(),
        &bracket16,
    )
    .expect("tournament bracket_size=16 should compile");
    assert!(
        result16.validation.ok,
        "{:?}",
        result16.validation.diagnostics
    );
    assert_eq!(
        result16
            .definition
            .nodes
            .iter()
            .filter(|node| node.handle.starts_with("contestant_"))
            .count(),
        16
    );
    assert_eq!(
        result16
            .definition
            .nodes
            .iter()
            .filter(|node| node.handle.contains("judge"))
            .count(),
        15
    );
    assert_eq!(result16.definition.endpoints.len(), 1);
}

#[test]
fn javascript_compiler_attaches_source_spans_to_validation_diagnostics() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let result = compile_workflow_code_javascript(
        node,
        r#"
const final = workflow.schema({
  handle: "final",
  schema: { type: "object", additionalProperties: false }
})
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ provider: "dev-stub" }),
  canCompleteWorkflowRun: true,
  maxTurns: 0
})
workflow.endpoint(worker, { handle: "entry" })
workflow.define({ alias: "bad", runOutputSchema: final })
"#,
        &WorkflowCodeLimitsConfig::default(),
    )
    .expect("workflow-code script should compile");

    let diagnostic = result
        .validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "invalid_max_turns")
        .expect("invalid max_turns diagnostic should exist");
    assert_eq!(diagnostic.handle.as_deref(), Some("worker"));
    let source_span = diagnostic
        .source_span
        .as_ref()
        .expect("diagnostic should carry a source span");
    assert!(source_span.start_line >= 1);
    assert!(source_span.start_column >= 1);
    assert_eq!(source_span.end_line, source_span.start_line);
    assert_eq!(source_span.end_column, source_span.start_column);
}

#[test]
fn javascript_compiler_reports_script_errors() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };

    let error = compile_workflow_code_javascript(
        node,
        r#"throw new Error("boom")"#,
        &WorkflowCodeLimitsConfig::default(),
    )
    .expect_err("script error should be returned");

    assert!(format!("{error}").contains("boom"));
}

#[test]
fn javascript_compiler_embeds_schema_from_approved_file() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-schema-import-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(root.join("schemas")).expect("schema directory should create");
    fs::write(
        root.join("schemas/final.json"),
        r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema file should write");

    let result = compile_workflow_code_javascript_with_schema_import_root(
        node,
        r#"
workflow.define({ alias: "imported_schema" })
const final = workflow.schemaFromFile({
  handle: "final",
  path: "schemas/final.json",
  alias: "Final output"
})
workflow.define({ runOutputSchema: final })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ provider: "dev-stub" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry" })
"#,
        &WorkflowCodeLimitsConfig::default(),
        Some(&root),
    )
    .expect("workflow-code schema import should compile");

    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(result.definition.schemas.len(), 1);
    assert_eq!(result.definition.schemas[0].handle, "final");
    assert_eq!(
        result.definition.schemas[0].schema["properties"]["answer"]["type"],
        "string"
    );
    assert_eq!(
        result.definition.workflow.run_output_schema.as_deref(),
        Some("final")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn javascript_compiler_rejects_schema_file_escape() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-schema-escape-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("schema root should create");

    let error = compile_workflow_code_javascript_with_schema_import_root(
        node,
        r#"
workflow.schemaFromFile({ handle: "final", path: "../outside.json" })
"#,
        &WorkflowCodeLimitsConfig::default(),
        Some(&root),
    )
    .expect_err("schema import should reject parent traversal");

    assert!(format!("{error}").contains("approved import root"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn javascript_compiler_rejects_schema_file_without_json_extension() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow-code JS compiler test because node is not available");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-schema-extension-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(root.join("schemas")).expect("schema root should create");
    fs::write(
        root.join("schemas/final.txt"),
        r#"{"type":"object","additionalProperties":false}"#,
    )
    .expect("schema fixture should write");

    let error = compile_workflow_code_javascript_with_schema_import_root(
        node,
        r#"
workflow.schemaFromFile({ handle: "final", path: "schemas/final.txt" })
"#,
        &WorkflowCodeLimitsConfig::default(),
        Some(&root),
    )
    .expect_err("schema import should reject non-json files");

    assert!(format!("{error}").contains("must end in .json"));
    let _ = fs::remove_dir_all(root);
}

// MP-08 / MP-11: declarative source receives only its own realm's values.
#[test]
fn compiler_isolation_has_no_host_objects_or_ambient_authority() {
    let node = find_node().expect("real Node is required for compiler isolation");
    let source = r#"
if (typeof process !== "undefined" || typeof require !== "undefined" ||
    typeof fetch !== "undefined" || typeof Buffer !== "undefined") {
  throw new Error("ambient host capability is visible")
}
if (Object.getPrototypeOf(workflow.define) !== Function.prototype ||
    Object.getPrototypeOf(console.log) !== Function.prototype) {
  throw new Error("builder received a host function")
}
if (globalThis.workflow !== workflow || globalThis.console !== console ||
    Object.getPrototypeOf(globalThis.console.log) !== Function.prototype) throw new Error("global builder received host values")
if (Object.getPrototypeOf(workflow) !== Object.prototype) throw new Error("builder received a host object")
let disabled = false
try { workflow.define.constructor("return 1")() } catch (error) { disabled = error instanceof EvalError }
if (!disabled) throw new Error("dynamic host functions are available")
const worker = workflow.node({agent: workflow.newAgent({provider: "dev-stub"})})
workflow.endpoint(worker)
"#;
    let result =
        compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
            .expect("compiler must isolate source from host objects");
    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
}

#[test]
fn compiler_isolation_ignores_caller_selected_executable() {
    let source = "const n = workflow.node({agent: workflow.newAgent({provider: 'dev-stub'})}); workflow.endpoint(n)";
    let result = compile_workflow_code_javascript(
        "/caller-selected-executable-must-not-run",
        source,
        &WorkflowCodeLimitsConfig::default(),
    )
    .expect("compiler uses the kernel-selected Node");
    assert!(result.validation.ok);
}

#[test]
fn compiler_isolation_keeps_promise_callbacks_in_the_source_realm() {
    let source = r#"
const originalThen = Promise.prototype.then
Promise.prototype.then = function (fulfilled, rejected) {
  for (const callback of [fulfilled, rejected]) {
    if (callback && Object.getPrototypeOf(callback) !== Function.prototype) {
      throw new Error("promise received a host callback")
    }
  }
  return originalThen.call(this, fulfilled, rejected)
}
await Promise.resolve()
const n = workflow.node({agent: workflow.newAgent({provider: "dev-stub"})})
workflow.endpoint(n)
"#;
    let result =
        compile_workflow_code_javascript("ignored", source, &WorkflowCodeLimitsConfig::default())
            .expect("promise settlement must not inject host callbacks");
    assert!(result.validation.ok);
}

// MP-08 / MP-11: a real workspace contains unrelated large JSON files.
#[test]
fn compiler_isolation_serializes_only_requested_schema_data() {
    let root = crate::test_support::TestWorktree::new("compiler-requested-schema");
    let limits = WorkflowCodeLimitsConfig::default();
    fs::write(
        root.path().join("unrelated.json"),
        vec![b'x'; limits.max_schema_bytes as usize + 1],
    )
    .unwrap();
    let source = "const n = workflow.node({agent: workflow.newAgent({provider: 'dev-stub'})}); workflow.endpoint(n)";
    let result = compile_workflow_code_javascript_with_schema_import_root(
        "ignored",
        source,
        &limits,
        Some(root.path()),
    )
    .unwrap();
    assert!(result.validation.ok);
    fs::create_dir(root.path().join("schemas")).unwrap();
    fs::write(root.path().join("schemas/final.json"), r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}"#).unwrap();
    let source = r#"
try { workflow.schemaFromFile("absent.json") } catch (error) {}
const path = ["schemas", "final.json"].join("/")
const final = workflow.schemaFromFile({path, alias: "Final"})
workflow.define({runOutputSchema: final})
const n = workflow.node({agent: workflow.newAgent({provider: "dev-stub"}), canCompleteWorkflowRun: true})
workflow.endpoint(n)
"#;
    let result = compile_workflow_code_javascript_with_schema_import_root(
        "ignored",
        source,
        &limits,
        Some(root.path()),
    )
    .unwrap();
    assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
    assert_eq!(result.definition.schemas.len(), 1);
    assert_eq!(
        result.definition.schemas[0].schema["required"],
        serde_json::json!(["answer"])
    );
}
