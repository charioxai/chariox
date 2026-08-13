use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;
use wait_timeout::ChildExt;

use crate::error::DaemonError;

use super::{
    io_error, CharioxEnvironmentConfig, CharioxEnvironmentRuntime, CharioxScriptRuntime,
    ScriptExecutionResult,
};

const PYTHON_INSPECTOR: &str = r#"
import contextlib
import importlib.util
import inspect
import io
import json
import pathlib
import sys
import typing

path = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("chariox_user_script", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
run = getattr(module, "run", None)
test_run = getattr(module, "test_run", None)
if not callable(run):
    raise RuntimeError("script must define callable run")
if not callable(test_run):
    raise RuntimeError("script must define callable test_run")
doc = inspect.getdoc(run)
if not doc:
    raise RuntimeError("run must have a docstring")
signature = inspect.signature(run)
properties = {}
required = []
for name, parameter in signature.parameters.items():
    annotation = parameter.annotation
    if annotation is inspect._empty:
        raise RuntimeError(f"parameter {name} must have a type annotation")
    schema = {"type": "string"}
    origin = typing.get_origin(annotation)
    args = typing.get_args(annotation)
    if annotation is str:
        schema = {"type": "string"}
    elif annotation is int:
        schema = {"type": "integer"}
    elif annotation is float:
        schema = {"type": "number"}
    elif annotation is bool:
        schema = {"type": "boolean"}
    elif annotation is dict:
        schema = {"type": "object"}
    elif origin is list and args:
        item = args[0]
        if item is str:
            schema = {"type": "array", "items": {"type": "string"}}
        elif item is int:
            schema = {"type": "array", "items": {"type": "integer"}}
        elif item is float:
            schema = {"type": "array", "items": {"type": "number"}}
        elif item is bool:
            schema = {"type": "array", "items": {"type": "boolean"}}
        else:
            raise RuntimeError(f"unsupported list item type for parameter {name}")
    elif str(origin).endswith("Literal"):
        schema = {"enum": list(args)}
    else:
        raise RuntimeError(f"unsupported type annotation for parameter {name}: {annotation!r}")
    properties[name] = schema
    if parameter.default is inspect._empty:
        required.append(name)
def is_json_return_annotation(annotation):
    if annotation is inspect._empty:
        return False
    if annotation is typing.Any or annotation is object:
        return True
    if annotation in (str, int, float, bool, dict, list, type(None), None):
        return True
    origin = typing.get_origin(annotation)
    args = typing.get_args(annotation)
    if origin is dict:
        if not args:
            return True
        key_type, value_type = args
        return key_type in (str, typing.Any, object) and is_json_return_annotation(value_type)
    if origin is list:
        return not args or is_json_return_annotation(args[0])
    if str(origin).endswith("Union"):
        return all(is_json_return_annotation(arg) for arg in args)
    if "Dict" in str(annotation) or "List" in str(annotation):
        return True
    return False

if not is_json_return_annotation(signature.return_annotation):
    raise RuntimeError("run return type must be JSON-serializable")
captured_stdout = io.StringIO()
captured_stderr = io.StringIO()
with contextlib.redirect_stdout(captured_stdout), contextlib.redirect_stderr(captured_stderr):
    test_run()
print(json.dumps({
    "description": doc,
    "input_schema": {
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": False
    }
}))
"#;
const PYTHON_CALLER: &str = r#"
import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import traceback

path = pathlib.Path(sys.argv[1])
arguments = json.loads(sys.stdin.read() or "{}")
spec = importlib.util.spec_from_file_location("chariox_user_script", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
captured_stdout = io.StringIO()
captured_stderr = io.StringIO()
try:
    with contextlib.redirect_stdout(captured_stdout), contextlib.redirect_stderr(captured_stderr):
        result = module.run(**arguments)
    json.dumps(result)
    print(json.dumps({"ok": True, "payload": result, "logs": captured_stdout.getvalue() + captured_stderr.getvalue()}))
except Exception as error:
    print(json.dumps({
        "ok": False,
        "payload": {
            "error": {
                "kind": "script_exception",
                "message": str(error),
                "traceback": traceback.format_exc(),
            },
            "logs": captured_stdout.getvalue() + captured_stderr.getvalue()
        }
    }))
"#;

const NODE_TEST_RUNNER: &str = r#"
const scriptPath = process.argv[1]
const mod = await import(scriptPath.startsWith("file://") ? scriptPath : `file://${scriptPath}`)
if (typeof mod.run !== "function") throw new Error("script must export run")
if (typeof mod.test_run !== "function") throw new Error("script must export test_run")
await mod.test_run()
"#;

const NODE_CALLER: &str = r#"
const scriptPath = process.argv[1]
const parameterOrder = JSON.parse(process.argv[2] || "[]")
const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const args = JSON.parse(Buffer.concat(chunks).toString() || "{}")
const logs = []
const originalLog = console.log
const originalError = console.error
console.log = (...values) => logs.push(values.join(" "))
console.error = (...values) => logs.push(values.join(" "))
try {
  const mod = await import(scriptPath.startsWith("file://") ? scriptPath : `file://${scriptPath}`)
  const result = await mod.run(...parameterOrder.map((name) => args[name]))
  JSON.stringify(result)
  originalLog(JSON.stringify({ ok: true, payload: result, logs: logs.join("\n") }))
} catch (error) {
  originalLog(JSON.stringify({
    ok: false,
    payload: {
      error: {
        kind: "script_exception",
        message: String(error && error.message ? error.message : error),
        stack: String(error && error.stack ? error.stack : "")
      },
      logs: logs.join("\n")
    }
  }))
}
"#;
pub(super) fn inspect_script(
    source: &Path,
    runtime: &CharioxScriptRuntime,
    env: &CharioxEnvironmentConfig,
) -> Result<(String, Value), DaemonError> {
    match (runtime, &env.runtime) {
        (CharioxScriptRuntime::Python, CharioxEnvironmentRuntime::Python { python }) => {
            let output = Command::new(python)
                .arg("-c")
                .arg(PYTHON_INSPECTOR)
                .arg(source)
                .output()
                .map_err(io_error("script.inspect"))?;
            if !output.status.success() {
                return Err(DaemonError::LocalTransport {
                    operation: "script.inspect",
                    message: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            let value = serde_json::from_slice::<Value>(&output.stdout).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "script.inspect",
                    message: format!("failed to parse Python script metadata: {error}"),
                }
            })?;
            Ok((
                value
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                value
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            ))
        }
        (
            CharioxScriptRuntime::TypeScript,
            CharioxEnvironmentRuntime::Node { node, package_root },
        ) => inspect_typescript_script(source, node, package_root.as_deref()),
        _ => Err(DaemonError::InvalidConfig {
            field: "environment",
            message: "script runtime and environment runtime do not match",
        }),
    }
}

fn inspect_typescript_script(
    source: &Path,
    node: &Path,
    package_root: Option<&Path>,
) -> Result<(String, Value), DaemonError> {
    let contents = fs::read_to_string(source).map_err(io_error("script.inspect"))?;
    if !contents.contains("function run") {
        return Err(DaemonError::InvalidConfig {
            field: "script",
            message: "TypeScript script must export a run function",
        });
    }
    if !contents.contains("test_run") {
        return Err(DaemonError::InvalidConfig {
            field: "script",
            message: "TypeScript script must export test_run",
        });
    }
    let description = extract_jsdoc_before_run(&contents).ok_or(DaemonError::InvalidConfig {
        field: "script",
        message: "TypeScript run function must have JSDoc",
    })?;
    let signature = contents
        .split("function run")
        .nth(1)
        .and_then(|tail| tail.split(')').next())
        .and_then(|tail| tail.strip_prefix('('))
        .ok_or(DaemonError::InvalidConfig {
            field: "script",
            message: "failed to parse TypeScript run signature",
        })?;
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    let mut parameter_order = Vec::new();
    for raw in signature
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (left, defaulted) = raw
            .split_once('=')
            .map(|(left, _)| (left.trim(), true))
            .unwrap_or((raw, false));
        let (name, ty) = left.split_once(':').ok_or(DaemonError::InvalidConfig {
            field: "script",
            message: "all TypeScript run parameters must have type annotations",
        })?;
        let schema = match ty.trim() {
            "string" => serde_json::json!({"type": "string"}),
            "number" => serde_json::json!({"type": "number"}),
            "boolean" => serde_json::json!({"type": "boolean"}),
            other if other.ends_with("[]") => serde_json::json!({"type": "array"}),
            other if other.starts_with("Record<") => serde_json::json!({"type": "object"}),
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "script",
                    message: "unsupported TypeScript run parameter type",
                });
            }
        };
        let name = name.trim();
        let clean_name = name.trim_end_matches('?').to_string();
        properties.insert(clean_name.clone(), schema);
        parameter_order.push(Value::String(clean_name.clone()));
        if !defaulted && !name.ends_with('?') {
            required.push(Value::String(clean_name));
        }
    }
    run_node_test(node, package_root, source)?;
    Ok((
        description,
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "x-chariox-parameter-order": parameter_order,
            "additionalProperties": false
        }),
    ))
}

fn run_node_test(
    node: &Path,
    package_root: Option<&Path>,
    source: &Path,
) -> Result<(), DaemonError> {
    let mut command = Command::new(node);
    if source.extension().and_then(|ext| ext.to_str()) == Some("ts") {
        command.arg("--import").arg("tsx");
    }
    command.arg("-e").arg(NODE_TEST_RUNNER).arg(source);
    if let Some(package_root) = package_root {
        command.current_dir(package_root);
    }
    let output = command.output().map_err(io_error("script.inspect"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation: "script.inspect",
            message: format!(
                "node script test_run failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        })
    }
}

fn extract_jsdoc_before_run(contents: &str) -> Option<String> {
    let before = contents.split("function run").next()?;
    let start = before.rfind("/**")?;
    let end = before[start..].find("*/")?;
    let block = &before[start + 3..start + end];
    let text = block
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

pub(super) fn execute_python_script(
    python: &Path,
    script: &Path,
    arguments: Value,
    timeout_sec: u64,
) -> Result<ScriptExecutionResult, DaemonError> {
    let mut child = Command::new(python)
        .arg("-c")
        .arg(PYTHON_CALLER)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io_error("script.execute"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(arguments.to_string().as_bytes())
            .map_err(io_error("script.execute"))?;
    }
    let status = child
        .wait_timeout(std::time::Duration::from_secs(timeout_sec.max(1)))
        .map_err(io_error("script.execute"))?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(ScriptExecutionResult {
            ok: false,
            payload: serde_json::json!({
                "error": {
                    "kind": "timeout",
                    "message": "script call timed out"
                }
            }),
            logs: String::new(),
        });
    }
    let output = child
        .wait_with_output()
        .map_err(io_error("script.execute"))?;
    let value = serde_json::from_slice::<Value>(&output.stdout).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "script.execute",
            message: format!(
                "script runner returned invalid JSON: {error}; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    })?;
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let logs = value
        .get("logs")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("logs"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let payload = value.get("payload").cloned().unwrap_or(value);
    Ok(ScriptExecutionResult { ok, payload, logs })
}

pub(super) fn execute_node_script(
    node: &Path,
    package_root: Option<&Path>,
    script: &Path,
    input_schema: &Value,
    arguments: Value,
    timeout_sec: u64,
) -> Result<ScriptExecutionResult, DaemonError> {
    let mut command = Command::new(node);
    if script.extension().and_then(|ext| ext.to_str()) == Some("ts") {
        command.arg("--import").arg("tsx");
    }
    let parameter_order = input_schema
        .get("x-chariox-parameter-order")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    command
        .arg("-e")
        .arg(NODE_CALLER)
        .arg(script)
        .arg(parameter_order.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(package_root) = package_root {
        command.current_dir(package_root);
    }
    let mut child = command.spawn().map_err(io_error("script.execute"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(arguments.to_string().as_bytes())
            .map_err(io_error("script.execute"))?;
    }
    let status = child
        .wait_timeout(std::time::Duration::from_secs(timeout_sec.max(1)))
        .map_err(io_error("script.execute"))?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(ScriptExecutionResult {
            ok: false,
            payload: serde_json::json!({
                "error": {
                    "kind": "timeout",
                    "message": "script call timed out"
                }
            }),
            logs: String::new(),
        });
    }
    let output = child
        .wait_with_output()
        .map_err(io_error("script.execute"))?;
    let value = serde_json::from_slice::<Value>(&output.stdout).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "script.execute",
            message: format!(
                "node script runner returned invalid JSON: {error}; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    })?;
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let logs = value
        .get("logs")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("logs"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let payload = value.get("payload").cloned().unwrap_or(value);
    Ok(ScriptExecutionResult { ok, payload, logs })
}
