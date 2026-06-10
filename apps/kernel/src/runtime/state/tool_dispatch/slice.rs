use base64::Engine;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

mod slice_browser;
use slice_browser::*;

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
                message: "slice runtime tools are only available inside Arroba slices".to_string(),
            });
        };
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch_slice_runtime_tool_call",
                    message: "provider run is not bound to an agent".to_string(),
                })?;
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
                    .unwrap_or_else(|| "/tmp/arroba-slice-screenshot.png".to_string());
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
                let status_output =
                    run_slice_screen_command(vec!["browser-status".to_string()]).await?;
                let browser_status = slice_browser_json(&status_output)?;
                let browser_url = browser_status_url(&browser_status)?;
                ensure_browser_target_matches_expectations(&browser_status, &args)?;
                let selector = browser_selector(args.selector.as_deref(), args.field_id.as_deref());
                ensure_browser_fill_target(&browser_status, selector.as_deref())?;
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
                let mut command_args = vec![if args.submit {
                    "secret-paste-submit-stdin".to_string()
                } else {
                    "secret-paste-stdin".to_string()
                }];
                if let Some(selector) = selector.clone() {
                    command_args.push(selector);
                }
                let output = run_slice_screen_command_with_stdin(command_args, secret).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["credential_id"] = serde_json::Value::String(args.credential_id.clone());
                payload["submitted"] = serde_json::Value::Bool(args.submit && output.success);
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
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
            crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL => {
                let output = run_slice_screen_command(vec!["browser-status".to_string()]).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_FIND_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserFindArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_find",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let output = run_slice_screen_command(vec![
                    "browser-find".to_string(),
                    args.query,
                    args.kind.unwrap_or_else(|| "any".to_string()),
                ])
                .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserFillArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_fill",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let selector = required_browser_selector(
                    args.selector.as_deref(),
                    args.field_id.as_deref(),
                    "runtime_tool_slice_browser_fill",
                )?;
                let output =
                    run_slice_screen_command(vec!["browser-fill".to_string(), selector, args.text])
                        .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_CLICK_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserClickArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_click",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let selector = required_browser_selector(
                    args.selector.as_deref(),
                    args.field_id.as_deref(),
                    "runtime_tool_slice_browser_click",
                )?;
                let output =
                    run_slice_screen_command(vec!["browser-click".to_string(), selector]).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_SUBMIT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserSubmitArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_submit",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut command_args = vec!["browser-submit".to_string()];
                if let Some(selector) =
                    browser_selector(args.selector.as_deref(), args.field_id.as_deref())
                {
                    command_args.push(selector);
                }
                let output = run_slice_screen_command(command_args).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL => {
                let output = run_slice_screen_command(vec!["browser-text".to_string()]).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["text"] = serde_json::Value::String(output.stdout.clone());
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
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

#[derive(Debug)]
struct SliceScreenCommandOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_slice_screen_command(
    args: Vec<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner(args, None).await
}

async fn run_slice_screen_command_with_stdin(
    args: Vec<String>,
    stdin: String,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner(args, Some(stdin)).await
}

async fn run_slice_screen_command_inner(
    args: Vec<String>,
    stdin: Option<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    let tool_path = std::env::var("ARROBA_SLICE_SCREEN_TOOL")
        .unwrap_or_else(|_| "/opt/arroba-slice/slice-screen.sh".to_string());
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
            child_stdin.write_all(stdin.as_bytes()).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!("failed to write slice screen stdin: {error}"),
                }
            })?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to wait for `{tool_path}`: {error}"),
            })?;
        Ok(SliceScreenCommandOutput {
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "run_slice_screen_command",
        message: error.to_string(),
    })?
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
