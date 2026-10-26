use super::*;

fn compile_schema_imports(count: usize) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    let root = crate::test_support::TestWorktree::new("schema-replay-limit");
    for index in 0..count {
        fs::write(
            root.path().join(format!("schema-{index}.json")),
            r#"{"type":"object"}"#,
        )
        .unwrap();
    }
    let source = r#"
workflow.define({ alias: "schema-replay-limit" });
for (let index = 0; index < IMPORT_COUNT; index++) {
    workflow.schemaFromFile({ handle: `schema_${index}`, path: `schema-${index}.json` });
}
const worker = workflow.node({ handle: "worker", agent: workflow.newAgent({ provider: "dev-stub" }), canCompleteWorkflowRun: true });
workflow.endpoint(worker, { handle: "entry" });
"#.replace("IMPORT_COUNT", &count.to_string());
    compile_workflow_code_javascript_with_schema_import_root(
        "/ignored-caller-node",
        &source,
        &WorkflowCodeLimitsConfig::default(),
        Some(root.path()),
    )
}

#[test]
fn javascript_compiler_finishes_after_32_sequential_schema_imports() {
    let result = compile_schema_imports(32)
        .expect("32 permitted schema imports must get a final compilation");
    assert!(result.validation.ok);
    assert_eq!(result.definition.schemas.len(), 32);
}

#[test]
fn javascript_compiler_retains_schema_resolution_round_limit() {
    let error = compile_schema_imports(33).unwrap_err();
    assert!(error.to_string().contains("resolution limit"));
}
