use super::*;

/// MP-08/MP-11: amaro 1.1.5, byte-identical to official Node 22.22.1's type
/// stripper. It runs inside the isolated compiler for every Node build.
pub(super) const TYPESCRIPT_STRIPPER: &str = include_str!("../../vendor/amaro/index.js");

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
    #[cfg(test)]
    compile_gate_for_test::pause(source);
    let mut imports = super::compiler_isolation::SchemaImports::new(schema_import_root)?;
    let started = std::time::Instant::now();
    let total_timeout = Duration::from_millis(limits.script_timeout_ms);
    // Caller-selected paths are never executed on the home kernel.
    let _ = node_path;
    let node = discover_workflow_code_node_path()?;
    let max_old_space_mb = u64::max(16, limits.script_memory_bytes.div_ceil(1024 * 1024));
    let mut command = super::compiler_isolation::compiler_command(&node, limits)?;
    command
        .arg(format!("--max-old-space-size={max_old_space_mb}"))
        .arg("--disable-wasm-trap-handler")
        .arg("--input-type=module")
        .arg("-e")
        .arg(NODE_WORKFLOW_CODE_COMPILER);
    // Declarative evaluation has no external effects. Resolve serialized schema
    // requests and replay with that approved data, sharing one total deadline
    // and the prepared private runtime boundary.
    const MAX_SCHEMA_RESOLUTION_ROUNDS: usize = 32;
    for round in 0..=MAX_SCHEMA_RESOLUTION_ROUNDS {
        let remaining = total_timeout.saturating_sub(started.elapsed()).as_millis() as u64;
        if remaining == 0 {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: "workflow-code script exceeded configured timeout".into(),
            });
        }
        let run_limits = WorkflowCodeLimitsConfig {
            script_timeout_ms: remaining,
            ..limits.clone()
        };
        let input = serde_json::to_vec(&WorkflowCodeCompilerInput {
            source,
            language: language.compiler_name(),
            timeout_ms: run_limits.script_timeout_ms,
            parameters,
            schema_files: imports.files.as_ref(),
            schema_errors: &imports.errors,
            typescript_stripper: (language == WorkflowCodeLanguage::TypeScript)
                .then_some(TYPESCRIPT_STRIPPER),
        })
        .map_err(|error| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("failed to serialize workflow-code compiler input: {error}"),
        })?;
        let output = super::compiler_process::run(
            &mut command,
            input,
            &run_limits,
            started + total_timeout,
        )?;
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
        if !compiler_output.schema_requests.is_empty() {
            if round == MAX_SCHEMA_RESOLUTION_ROUNDS {
                break;
            }
            imports.load(compiler_output.schema_requests, limits)?;
            continue;
        }
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
        return Ok(WorkflowCodeCompileResult {
            definition,
            validation,
            logs,
            source_spans,
        });
    }
    Err(crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: "schema imports exceed configured resolution limit".into(),
    })
}

/// Holds a compilation whose source contains an installed marker until the test
/// releases it, so authority rechecks after a compiler wait are deterministic.
#[cfg(test)]
pub(crate) mod compile_gate_for_test {
    use std::sync::{mpsc, Mutex};
    use tokio::sync::oneshot;

    type Gate = (String, oneshot::Sender<()>, mpsc::Receiver<()>);
    static GATES: Mutex<Vec<Gate>> = Mutex::new(Vec::new());

    /// Returns a started signal and the release handle (dropping it also releases).
    pub(crate) fn install(marker: &str) -> (oneshot::Receiver<()>, mpsc::Sender<()>) {
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = mpsc::channel();
        GATES
            .lock()
            .unwrap()
            .push((marker.to_string(), started, release_rx));
        (started_rx, release)
    }

    pub(super) fn pause(source: &str) {
        let gate = {
            let mut gates = GATES.lock().unwrap();
            let index = gates
                .iter()
                .position(|(marker, ..)| source.contains(marker.as_str()));
            index.map(|index| gates.remove(index))
        };
        if let Some((_, started, release)) = gate {
            let _ = started.send(());
            let _ = release.recv();
        }
    }
}

pub fn discover_workflow_code_node_path() -> Result<PathBuf, crate::DaemonError> {
    discover_node_path(env::var_os("NODE"))
}

fn discover_node_path(
    configured: Option<std::ffi::OsString>,
) -> Result<PathBuf, crate::DaemonError> {
    let mut candidates = Vec::new();
    if let Some(path) = configured {
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
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.clone()));
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    for candidate in candidates {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(mut child) = Command::new(&candidate)
            .env_clear()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if child
            .wait_timeout(remaining.min(Duration::from_millis(500)))
            .is_ok_and(|status| status.is_some_and(|status| status.success()))
        {
            return Ok(candidate);
        }
        if child.id() > 1 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    Err(crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: "could not find responsive Node.js for workflow-code compilation; set the kernel NODE environment variable".into(),
    })
}

#[cfg(unix)]
#[test]
fn node_discovery_preserves_the_explicit_runtime_override() {
    use std::os::unix::fs::PermissionsExt;
    // Never set the process-wide NODE: parallel compiler tests discover Node too.
    let worktree = crate::test_support::TestWorktree::new("node-discovery-override");
    let node = worktree.path().join("configured-node");
    std::fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(discover_node_path(Some(node.clone().into())).unwrap(), node);
}
