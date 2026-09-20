use base64::Engine;
use wait_timeout::ChildExt;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

mod slice_browser;
use slice_browser::*;
mod browser_runtime_controller;
use browser_runtime_controller::*;

const DEFAULT_SLICE_SCREEN_COMMAND_TIMEOUT_MS: u64 = 70_000;
const SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES: usize = 256 * 1024;

impl KernelRuntimeState {
    pub(super) async fn dispatch_slice_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let Some(slice_id) = self.slice_kernel_id() else {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch_slice_runtime_tool_call",
                message: "slice runtime tools are only available inside Chariox slices".to_string(),
            });
        };
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch_slice_runtime_tool_call",
                    message: "provider run is not bound to an agent".to_string(),
                })?;

        if is_browser_runtime_mcp_tool(tool_name) {
            return dispatch_browser_runtime_tool_call(&slice_id, agent_id, tool_name, arguments)
                .await;
        }

        let output = match tool_name {
            crate::transport::runtime_tools::SLICE_SCREEN_STATUS_TOOL => {
                run_slice_screen_command(vec!["status".to_string()]).await?
            }
            crate::transport::runtime_tools::SLICE_SCREENSHOT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceScreenshotArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_screenshot",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let image_path = args
                    .path
                    .unwrap_or_else(|| "/tmp/chariox-slice-screenshot.png".to_string());
                let output =
                    run_slice_screen_command(vec!["screenshot".to_string(), image_path.clone()])
                        .await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["image_path"] = serde_json::Value::String(image_path.clone());
                payload["mime_type"] = serde_json::Value::String("image/png".to_string());
                if output.success && args.return_image_base64 {
                    let image_path = std::path::PathBuf::from(&image_path);
                    let image_bytes = std::fs::read(&image_path).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_slice_screenshot",
                            message: format!(
                                "failed to read screenshot `{}`: {error}",
                                image_path.display()
                            ),
                        }
                    })?;
                    payload["image_base64"] = serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(image_bytes),
                    );
                }
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_OCR_TOOL => {
                let args = serde_json::from_value::<crate::transport::runtime_tools::SliceOcrArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_ocr",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut command_args = vec!["ocr".to_string()];
                if let Some(image_path) = args.image_path {
                    command_args.push(image_path);
                }
                let output = run_slice_screen_command(command_args).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["text"] = serde_json::Value::String(output.stdout.clone());
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_FIND_TEXT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceFindTextArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_find_text",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut command_args = vec!["find-text".to_string(), args.query];
                if let Some(image_path) = args.image_path {
                    command_args.push(image_path);
                }
                let output = run_slice_screen_command(command_args).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["match"] = output
                    .stdout
                    .lines()
                    .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .unwrap_or(serde_json::Value::Null);
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_MOUSE_TOOL => {
                let args =
                    serde_json::from_value::<crate::transport::runtime_tools::SliceMouseArgs>(
                        arguments,
                    )
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "runtime_tool_slice_mouse",
                        message: format!("invalid tool arguments: {error}"),
                    })?;
                run_slice_screen_command(slice_mouse_command_args(args)?).await?
            }
            crate::transport::runtime_tools::SLICE_KEYBOARD_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceKeyboardArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_keyboard",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                run_slice_screen_command(slice_keyboard_command_args(args)?).await?
            }
            crate::transport::runtime_tools::PASTE_SECRET_TO_SLICE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::PasteSecretToSliceArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_paste_secret_to_slice",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let status_output = run_browser_runtime_mcp_call(
                    crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL,
                    serde_json::json!({}),
                )
                .await?;
                let browser_status = slice_browser_json(&status_output)?;
                let browser_url = browser_status_url(&browser_status)?;
                ensure_browser_target_matches_expectations(&browser_status, &args)?;
                let target_arguments = browser_fill_target_arguments(
                    &browser_status,
                    args.selector.as_deref(),
                    args.field_id.as_deref(),
                )?;
                let secret = match self
                    .resolve_remote_home_credential_secret(
                        provider_run,
                        &args.credential_id,
                        crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                            target_url: browser_url.clone(),
                        },
                    )
                    .await?
                {
                    Some(secret) => secret,
                    None => {
                        let _vault_unlock = self
                            .ensure_vault_unlocked_for_provider_run(
                                provider_run,
                                "runtime_tool_paste_secret_to_slice",
                            )
                            .await?;
                        let user_config = self.owned.config_projection.snapshot().user_config;
                        let credentials = crate::credential::load_user_credentials()?;
                        let service = crate::secret::RuntimeSecretService::with_vault_config(
                            credentials,
                            &user_config.credential_vault,
                        )?;
                        service.browser_secret_input_for_target_url(
                            &args.credential_id,
                            &browser_url,
                        )?
                    }
                };
                let mut fill_arguments = target_arguments.clone();
                fill_arguments.insert("text".to_string(), serde_json::Value::String(secret));
                let mut output = run_browser_runtime_mcp_call(
                    crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL,
                    serde_json::Value::Object(fill_arguments),
                )
                .await?;
                if output.success && args.submit {
                    let submit_arguments = target_arguments;
                    output = run_browser_runtime_mcp_call(
                        crate::transport::runtime_tools::SLICE_BROWSER_SUBMIT_TOOL,
                        serde_json::Value::Object(submit_arguments),
                    )
                    .await?;
                }
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload: secret_paste_payload(
                        &slice_id,
                        agent_id,
                        &args.credential_id,
                        args.submit && output.success,
                        &output,
                    ),
                });
            }
            crate::transport::runtime_tools::SLICE_OPEN_URL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceOpenUrlArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_open_url",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                run_slice_screen_command(vec!["open-url".to_string(), args.url]).await?
            }
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_slice_runtime_tool_call",
                    message: format!("unknown slice runtime tool `{tool_name}`"),
                });
            }
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: output.success,
            payload: slice_tool_payload(&slice_id, agent_id, &output),
        })
    }
}

async fn dispatch_browser_runtime_tool_call(
    slice_id: &str,
    agent_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let output = run_browser_runtime_mcp_call(tool_name, arguments).await?;
    let text = (tool_name == crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL)
        .then(|| output.stdout.clone());
    let mut result = slice_browser_tool_result(slice_id, agent_id, output);
    if let Some(text) = text {
        result.payload["text"] = serde_json::Value::String(text);
    }
    Ok(result)
}

#[derive(Debug)]
struct SliceScreenCommandOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

async fn run_slice_screen_command(
    args: Vec<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner(args, None).await
}

async fn run_slice_screen_command_inner(
    args: Vec<String>,
    stdin: Option<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    let tool_path = std::env::var("CHARIOX_SLICE_SCREEN_TOOL")
        .unwrap_or_else(|_| "/opt/chariox-slice/slice-screen.sh".to_string());
    tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&tool_path);
        command
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if stdin.is_some() {
            command.stdin(std::process::Stdio::piped());
        }
        let mut child = command
            .spawn()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to run `{tool_path}`: {error}"),
            })?;
        if let Some(stdin) = stdin {
            use std::io::Write;
            let Some(mut child_stdin) = child.stdin.take() else {
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen command did not expose stdin".to_string(),
                });
            };
            if let Err(error) = child_stdin.write_all(stdin.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!("failed to write slice screen stdin: {error}"),
                });
            }
        }
        drop(child.stdin.take());
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: "slice screen command did not expose stdout".to_string(),
            })?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: "slice screen command did not expose stderr".to_string(),
            })?;
        let stdout_reader = std::thread::spawn(move || read_child_output(stdout));
        let stderr_reader = std::thread::spawn(move || read_child_output(stderr));
        let status = match child
            .wait_timeout(std::time::Duration::from_millis(
                slice_screen_command_timeout_ms(),
            ))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to wait for `{tool_path}`: {error}"),
            })? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!(
                        "slice screen command timed out after {}ms",
                        slice_screen_command_timeout_ms()
                    ),
                });
            }
        };
        let (stdout, stdout_truncated) =
            stdout_reader
                .join()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen stdout reader panicked".to_string(),
                })??;
        let (stderr, stderr_truncated) =
            stderr_reader
                .join()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen stderr reader panicked".to_string(),
                })??;
        Ok(SliceScreenCommandOutput {
            success: status.success(),
            status_code: status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "run_slice_screen_command",
        message: error.to_string(),
    })?
}

fn slice_screen_command_timeout_ms() -> u64 {
    std::env::var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SLICE_SCREEN_COMMAND_TIMEOUT_MS)
}

fn read_child_output<R: std::io::Read>(mut reader: R) -> Result<(String, bool), DaemonError> {
    let mut stored = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to read slice screen output: {error}"),
            })?;
        if read == 0 {
            break;
        }
        let remaining = SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES.saturating_sub(stored.len());
        if remaining > 0 {
            stored.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        if read > remaining {
            truncated = true;
        }
    }
    Ok((
        String::from_utf8_lossy(&stored).trim().to_string(),
        truncated,
    ))
}

fn slice_tool_payload(
    slice_id: &str,
    agent_id: &str,
    output: &SliceScreenCommandOutput,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "slice_id".to_string(),
        serde_json::Value::String(slice_id.to_string()),
    );
    payload.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    payload.insert(
        "status_code".to_string(),
        output
            .status_code
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    payload.insert(
        "stdout".to_string(),
        serde_json::Value::String(output.stdout.clone()),
    );
    payload.insert(
        "stderr".to_string(),
        serde_json::Value::String(output.stderr.clone()),
    );
    if output.stdout_truncated {
        payload.insert(
            "stdout_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if output.stderr_truncated {
        payload.insert(
            "stderr_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    for line in output.stdout.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            match key {
                "available" => {
                    payload.insert(key.to_string(), serde_json::Value::Bool(value == "true"));
                }
                "display" | "screen" | "viewer" | "mode" | "missing" | "message" => {
                    payload.insert(
                        key.to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
                _ => {}
            }
        }
    }
    serde_json::Value::Object(payload)
}

fn secret_paste_payload(
    slice_id: &str,
    agent_id: &str,
    credential_id: &str,
    submitted: bool,
    output: &SliceScreenCommandOutput,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "slice_id".to_string(),
        serde_json::Value::String(slice_id.to_string()),
    );
    payload.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    payload.insert(
        "credential_id".to_string(),
        serde_json::Value::String(credential_id.to_string()),
    );
    payload.insert("submitted".to_string(), serde_json::Value::Bool(submitted));
    payload.insert(
        "status_code".to_string(),
        output
            .status_code
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    if output.stdout_truncated {
        payload.insert(
            "stdout_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if output.stderr_truncated {
        payload.insert(
            "stderr_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    serde_json::Value::Object(payload)
}

fn slice_mouse_command_args(
    args: crate::transport::runtime_tools::SliceMouseArgs,
) -> Result<Vec<String>, DaemonError> {
    match args.action.as_str() {
        "move" => Ok(vec![
            "move".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "click" => Ok(vec![
            "click".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "double_click" => Ok(vec![
            "double-click".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "scroll" => Ok(vec![
            "scroll".to_string(),
            args.amount.unwrap_or(1).to_string(),
        ]),
        "drag" => Ok(vec![
            "drag".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.to_x, "to_x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.to_y, "to_y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_slice_mouse",
            message: format!("unsupported mouse action `{other}`"),
        }),
    }
}

fn slice_keyboard_command_args(
    args: crate::transport::runtime_tools::SliceKeyboardArgs,
) -> Result<Vec<String>, DaemonError> {
    match args.action.as_str() {
        "type" => Ok(vec![
            "type".to_string(),
            required_string(args.text, "text", "runtime_tool_slice_keyboard")?,
        ]),
        "key" => Ok(vec![
            "key".to_string(),
            required_string(args.key, "key", "runtime_tool_slice_keyboard")?,
        ]),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_slice_keyboard",
            message: format!("unsupported keyboard action `{other}`"),
        }),
    }
}

fn required_i64(
    value: Option<i64>,
    field: &str,
    operation: &'static str,
) -> Result<i64, DaemonError> {
    value.ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: format!("missing required `{field}`"),
    })
}

fn required_string(
    value: Option<String>,
    field: &str,
    operation: &'static str,
) -> Result<String, DaemonError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!("missing required `{field}`"),
        })
}

fn browser_fill_target_arguments(
    status: &serde_json::Value,
    selector: Option<&str>,
    field_id: Option<&str>,
) -> Result<serde_json::Map<String, serde_json::Value>, DaemonError> {
    if selector.is_some() {
        let selector = browser_selector(selector, None).ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_slice",
            message: "selector must not be empty".to_string(),
        })?;
        ensure_browser_fill_target(status, Some(&selector))?;
        let mut arguments = serde_json::Map::new();
        arguments.insert("selector".to_string(), serde_json::Value::String(selector));
        return Ok(arguments);
    }
    if field_id.is_some() {
        let field_id = browser_selector(None, field_id).ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_slice",
            message: "field_id must not be empty".to_string(),
        })?;
        ensure_browser_fill_target(status, Some(&field_id))?;
        let mut arguments = serde_json::Map::new();
        arguments.insert("field_id".to_string(), serde_json::Value::String(field_id));
        return Ok(arguments);
    }

    ensure_browser_fill_target(status, None)?;
    let focused = status.get("focusedElement");
    let focused_selector = focused
        .and_then(|value| value.get("selector"))
        .and_then(serde_json::Value::as_str);
    let focused_field_id = focused
        .and_then(|value| value.get("field_id"))
        .and_then(serde_json::Value::as_str);
    if let Some(selector) = browser_selector(focused_selector, None) {
        let mut arguments = serde_json::Map::new();
        arguments.insert("selector".to_string(), serde_json::Value::String(selector));
        return Ok(arguments);
    }
    if let Some(field_id) = browser_selector(None, focused_field_id) {
        let mut arguments = serde_json::Map::new();
        arguments.insert("field_id".to_string(), serde_json::Value::String(field_id));
        return Ok(arguments);
    }
    Err(DaemonError::LocalTransport {
        operation: "runtime_tool_paste_secret_to_slice",
        message: "focused browser field did not expose a selector or field_id".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::Mutex;
    use tokio::net::UnixListener;

    use super::*;

    fn browser_runtime_state_from_app(app: crate::app::DaemonApp) -> KernelRuntimeState {
        let config_projection = app.config_projection_store();
        let session_store = app.session_state_store();
        let agent_store = app.agents().clone();
        let attachment_store = app.attachments().clone();
        let provider_store = app.providers().clone();
        let provider_process_tracking = app.provider_process_tracking_store();
        let slice_store = app.slices();
        let session_projection = app.session_state_projection_store();
        let provider_run_projection = app.provider_run_projection_store();
        let operational_history_store = app.operational_history_store();
        let durable_state_store = app.durable_state_store();
        let prompt_state_owner = app.prompt_state_owner();
        let active_turns = app.active_turn_store();
        let prompt_activity = app.prompt_activity_store();
        let prompt_workspace_claims = app.prompt_workspace_claim_store();
        let structured_output_records = app.structured_output_record_store();
        let terminal_stream = app.terminal_stream_store();
        let workflow_design_events = app.workflow_design_event_store();
        let metaagent_events = app.metaagent_event_store();
        let workspace_coordinator = app.workspace_coordinator();
        KernelRuntimeState::new_with_owned_state(
            Arc::new(Mutex::new(app)),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[test]
    fn slice_mouse_args_map_to_screen_script_commands() {
        let args = crate::transport::runtime_tools::SliceMouseArgs {
            action: "drag".to_string(),
            x: Some(10),
            y: Some(20),
            to_x: Some(30),
            to_y: Some(40),
            amount: None,
        };

        assert_eq!(
            slice_mouse_command_args(args).expect("drag args should map"),
            vec![
                "drag".to_string(),
                "10".to_string(),
                "20".to_string(),
                "30".to_string(),
                "40".to_string()
            ]
        );
    }

    #[test]
    fn slice_keyboard_args_require_text_for_type() {
        let args = crate::transport::runtime_tools::SliceKeyboardArgs {
            action: "type".to_string(),
            text: None,
            key: None,
        };

        assert!(slice_keyboard_command_args(args).is_err());
    }

    #[test]
    fn slice_tool_payload_reports_screen_availability() {
        let output = SliceScreenCommandOutput {
            success: false,
            status_code: Some(1),
            stdout: [
                "display=:99",
                "screen=1280x800",
                "mode=headless",
                "available=false",
                "missing=xvfb,novnc",
                "message=slice screen is unavailable; missing xvfb,novnc",
            ]
            .join("\n"),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let payload = slice_tool_payload("slice-1", "agent-1", &output);

        assert_eq!(payload["mode"], "headless");
        assert_eq!(payload["available"], false);
        assert_eq!(payload["missing"], "xvfb,novnc");
        assert_eq!(
            payload["message"],
            "slice screen is unavailable; missing xvfb,novnc"
        );
        assert_eq!(payload.get("viewer"), None);
    }

    #[test]
    fn secret_paste_payload_does_not_reflect_helper_output() {
        let output = SliceScreenCommandOutput {
            success: true,
            status_code: Some(0),
            stdout: "typed super-secret-value".to_string(),
            stderr: "debug super-secret-value".to_string(),
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let payload = secret_paste_payload("slice-1", "agent-1", "gmail-password", true, &output);
        let serialized = serde_json::to_string(&payload).expect("payload should serialize");

        assert!(serialized.contains("gmail-password"));
        assert!(serialized.contains("\"submitted\":true"));
        assert!(!serialized.contains("super-secret-value"));
        assert!(payload.get("stdout").is_none());
        assert!(payload.get("stderr").is_none());
    }

    #[test]
    fn targetless_secret_paste_uses_the_focused_field_target() {
        let status = serde_json::json!({
            "focusedElement": {
                "kind": "field",
                "selector": "#password",
                "field_id": "field:password"
            },
            "fields": [{
                "selector": "#password",
                "field_id": "field:password"
            }]
        });

        let arguments = browser_fill_target_arguments(&status, None, None)
            .expect("focused field should provide a normal fill target");
        assert_eq!(arguments.get("selector").and_then(|value| value.as_str()), Some("#password"));
        assert!(arguments.get("field_id").is_none());
    }

    #[test]
    fn read_child_output_caps_stored_bytes_while_draining() {
        let input = vec![b'a'; SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES + 1024];

        let (output, truncated) =
            read_child_output(std::io::Cursor::new(input)).expect("output should read");

        assert!(truncated);
        assert_eq!(output.len(), SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES);
    }

    #[tokio::test]
    async fn slice_screen_command_times_out() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-slice-timeout-test-{}",
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::create_dir_all(&root);
        let script = root.join("slice-screen-timeout.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 1\n").expect("script should write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
                .expect("script should be executable");
        }
        std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
        std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "50");

        let error = run_slice_screen_command(vec!["status".to_string()])
            .await
            .expect_err("sleeping helper should time out");

        assert!(error.to_string().contains("timed out"));
        std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
        std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn browser_runtime_dispatch_uses_one_controller_endpoint() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-browser-runtime-dispatch-test-{}",
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test directory should be created");
        let socket = root.join("browser-runtime-mcp.sock");
        let auth_file = root.join("browser-runtime-mcp.auth");
        let one_shot_invoked = root.join("one-shot-invoked");
        let one_shot = root.join("slice-screen-marker.sh");
        std::fs::write(
            &one_shot,
            format!("#!/bin/sh\ntouch {}\n", one_shot_invoked.display()),
        )
        .expect("one-shot marker should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&one_shot, std::fs::Permissions::from_mode(0o700))
                .expect("one-shot marker should be executable");
        }
        std::fs::write(&auth_file, "local-test-token\n").expect("auth file should be written");
        let listener = UnixListener::bind(&socket).expect("controller socket should bind");
        let seen_tools = Arc::new(StdMutex::new(Vec::<String>::new()));
        let seen_tools_task = Arc::clone(&seen_tools);
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.expect("controller should accept");
                let seen_tools = Arc::clone(&seen_tools_task);
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.is_err() {
                        return;
                    }
                    let request: serde_json::Value = match serde_json::from_str(&line) {
                        Ok(request) => request,
                        Err(_) => return,
                    };
                    let request_id = request["request_id"].as_str().unwrap_or_default();
                    let tool_name = request["tool_name"].as_str().unwrap_or_default();
                    seen_tools
                        .lock()
                        .expect("seen tools lock should not be poisoned")
                        .push(tool_name.to_string());
                    let arguments = &request["arguments"];
                    if arguments.get("query").and_then(serde_json::Value::as_str)
                        == Some("malformed")
                    {
                        let _ = reader.get_mut().write_all(b"not-json\n").await;
                        return;
                    }
                    if arguments.get("text").and_then(serde_json::Value::as_str) == Some("timeout")
                    {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    let result = if tool_name.ends_with("slice_browser_status") {
                        serde_json::json!({
                            "url": "https://example.test/login",
                            "host": "example.test",
                            "title": "Login",
                            "focusedElement": {"kind": "field"},
                            "fields": [{"selector": "#email", "field_id": "field:1"}],
                            "buttons": [],
                            "links": []
                        })
                    } else if tool_name.ends_with("slice_browser_text") {
                        serde_json::Value::String("visible page text".to_string())
                    } else {
                        serde_json::json!({"ok": true, "selector": "#email"})
                    };
                    let response = serde_json::json!({
                        "type": "tool_result",
                        "request_id": request_id,
                        "ok": true,
                        "tool_name": tool_name,
                        "result": result,
                    });
                    let encoded = serde_json::to_vec(&response).expect("response should encode");
                    let _ = reader.get_mut().write_all(&encoded).await;
                    let _ = reader.get_mut().write_all(b"\n").await;
                });
            }
        });

        std::env::set_var("CHARIOX_BROWSER_RUNTIME_MCP_SOCKET", &socket);
        std::env::set_var("CHARIOX_BROWSER_RUNTIME_MCP_AUTH_FILE", &auth_file);
        std::env::set_var("CHARIOX_BROWSER_RUNTIME_MCP_TIMEOUT_MS", "120");
        std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &one_shot);

        let mut config = crate::config::DaemonConfig::for_tests();
        config.host_machine_id = "slice:slice-1".to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("daemon should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "browser-runtime-test-workspace",
                "browser-runtime-test-worktree",
            ))
            .expect("session should be created");
        let provider_run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        let runtime = browser_runtime_state_from_app(app);

        let status = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL,
                serde_json::json!({}),
            )
            .await
            .expect("status dispatch should complete");
        assert!(status.ok);
        assert_eq!(
            status.payload["browser"]["url"],
            "https://example.test/login"
        );

        let text = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL,
                serde_json::json!({}),
            )
            .await
            .expect("text dispatch should complete");
        assert!(text.ok);
        assert_eq!(text.payload["text"], "visible page text");

        let fill = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL,
                serde_json::json!({"selector": "#email", "text": "alice@example.test"}),
            )
            .await
            .expect("fill dispatch should complete");
        assert!(fill.ok);
        assert_eq!(fill.payload["browser"]["ok"], true);

        let malformed = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_FIND_TOOL,
                serde_json::json!({"query": "malformed"}),
            )
            .await
            .expect("malformed response should be represented");
        assert!(!malformed.ok);
        assert!(malformed.payload["stderr"]
            .as_str()
            .expect("malformed response should have stderr")
            .contains("malformed"));

        let timed_out = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL,
                serde_json::json!({"text": "timeout", "timeout_ms": 100}),
            )
            .await
            .expect("timeout should be represented");
        assert!(!timed_out.ok);
        assert!(timed_out.payload["stderr"]
            .as_str()
            .expect("timeout should have stderr")
            .contains("timed out"));

        server.abort();
        let _ = server.await;
        let unavailable = runtime
            .dispatch_slice_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL,
                serde_json::json!({}),
            )
            .await
            .expect("unavailable endpoint should be represented");
        assert!(!unavailable.ok);
        assert!(unavailable.payload["stderr"]
            .as_str()
            .expect("unavailable endpoint should have stderr")
            .contains("unavailable"));

        let seen_tools = seen_tools
            .lock()
            .expect("seen tools lock should not be poisoned")
            .clone();
        assert_eq!(
            seen_tools,
            vec![
                crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL,
                crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL,
                crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL,
                crate::transport::runtime_tools::SLICE_BROWSER_FIND_TOOL,
                crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL,
            ]
        );
        assert!(
            !one_shot_invoked.exists(),
            "slice-screen one-shot path was invoked"
        );

        std::env::remove_var("CHARIOX_BROWSER_RUNTIME_MCP_SOCKET");
        std::env::remove_var("CHARIOX_BROWSER_RUNTIME_MCP_AUTH_FILE");
        std::env::remove_var("CHARIOX_BROWSER_RUNTIME_MCP_TIMEOUT_MS");
        std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
        let _ = std::fs::remove_dir_all(&root);
    }
}
