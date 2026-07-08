use super::*;

pub fn compile_workflow_code_javascript(
    node_path: impl AsRef<Path>,
    source: &str,
    limits: &WorkflowCodeLimitsConfig,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_schema_import_root(
        node_path,
        source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        None,
    )
}

pub fn compile_workflow_code_javascript_with_parameters(
    node_path: impl AsRef<Path>,
    source: &str,
    limits: &WorkflowCodeLimitsConfig,
    parameters: &BTreeMap<String, Value>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_parameters_and_schema_import_root(
        node_path,
        source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        parameters,
        None,
    )
}

pub fn compile_workflow_code_javascript_with_schema_import_root(
    node_path: impl AsRef<Path>,
    source: &str,
    limits: &WorkflowCodeLimitsConfig,
    schema_import_root: Option<&Path>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_schema_import_root(
        node_path,
        source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        schema_import_root,
    )
}

pub fn compile_workflow_code_source_with_schema_import_root(
    node_path: impl AsRef<Path>,
    source: &str,
    language: WorkflowCodeLanguage,
    limits: &WorkflowCodeLimitsConfig,
    schema_import_root: Option<&Path>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_parameters_and_schema_import_root(
        node_path,
        source,
        language,
        limits,
        &BTreeMap::new(),
        schema_import_root,
    )
}

pub fn compile_workflow_code_source_with_parameters_and_schema_import_root(
    node_path: impl AsRef<Path>,
    source: &str,
    language: WorkflowCodeLanguage,
    limits: &WorkflowCodeLimitsConfig,
    parameters: &BTreeMap<String, Value>,
    schema_import_root: Option<&Path>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    let max_old_space_mb = u64::max(16, limits.script_memory_bytes.div_ceil(1024 * 1024));
    let mut child = Command::new(node_path.as_ref())
        .arg(format!("--max-old-space-size={max_old_space_mb}"))
        .arg("--input-type=module")
        .arg("-e")
        .arg(NODE_WORKFLOW_CODE_COMPILER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("failed to start Node workflow-code compiler: {error}"),
        })?;

    let input = serde_json::to_vec(&WorkflowCodeCompilerInput {
        source,
        language: language.compiler_name(),
        timeout_ms: limits.script_timeout_ms,
        max_schema_bytes: limits.max_schema_bytes,
        parameters,
        schema_import_root,
    })
    .map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: format!("failed to serialize workflow-code compiler input: {error}"),
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: "failed to open Node workflow-code compiler stdin".to_string(),
        })?;
    stdin
        .write_all(&input)
        .map_err(io_error("workflow_code.compile"))?;
    drop(stdin);

    let timeout = Duration::from_millis(limits.script_timeout_ms);
    match child
        .wait_timeout(timeout)
        .map_err(io_error("workflow_code.compile"))?
    {
        Some(_) => {}
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: format!(
                    "workflow-code script exceeded configured timeout of {} ms",
                    limits.script_timeout_ms
                ),
            });
        }
    }

    let output = child
        .wait_with_output()
        .map_err(io_error("workflow_code.compile"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!(
                "Node workflow-code compiler failed with status {}: {}{}",
                output.status,
                stderr.trim(),
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!("\nstdout: {}", stdout.trim())
                }
            ),
        });
    }

    let compiler_output = serde_json::from_str::<WorkflowCodeCompilerOutput>(stdout.trim())
        .map_err(|error| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("failed to parse Node workflow-code compiler output: {error}"),
        })?;
    let logs = compiler_output.logs.unwrap_or_default();
    if !compiler_output.ok {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: compiler_output
                .error
                .unwrap_or_else(|| "workflow-code script failed".to_string()),
        });
    }
    let definition =
        compiler_output
            .definition
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: "Node workflow-code compiler did not return a definition".to_string(),
            })?;
    let source_spans = compiler_output.source_spans;
    let mut validation = definition.validate_with_limits(limits);
    attach_workflow_code_diagnostic_spans(&mut validation, &source_spans);
    Ok(WorkflowCodeCompileResult {
        definition,
        validation,
        logs,
        source_spans,
    })
}

pub fn discover_workflow_code_node_path() -> Result<PathBuf, crate::DaemonError> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("NODE") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ]);
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            candidates.push(dir.join("node"));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message:
                "could not find Node.js for workflow-code compilation; pass node_path or set NODE"
                    .to_string(),
        })
}
