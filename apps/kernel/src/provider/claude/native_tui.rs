//! Claude native TUI hook files and launch arguments.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::launch_args::{normalized_claude_model, request_uses_metaagent_tools_only};
use super::mcp_config::{
    create_claude_runtime_files_root, materialize_request_claude_mcp_config, ClaudeRuntimeFilesRoot,
};
use super::usage_capture::materialize_claude_usage_capture;

pub(crate) const CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS: usize = 8;
pub(crate) const CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES: usize = 6_000;
pub(crate) const CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES: usize =
    CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS * CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES;
/// The timeout of the kernel interaction a Claude permission request raises.
pub(crate) const CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS: u64 = 300;
/// How long the hook waits for the kernel's decision (`deadline` in the
/// handler): past the interaction timeout plus a forwarded interaction's relay
/// buffer, so the deny a timed-out interaction resolves to finds it waiting.
#[allow(
    dead_code,
    reason = "Retains the compile-time ordering check against the native hook timeout"
)]
pub(crate) const CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS: u64 = 330;
/// Claude's timeout for the permission hook, past the hook's own wait.
const CLAUDE_NATIVE_PERMISSION_HOOK_TIMEOUT_SECS: u64 = 360;
const _: () =
    assert!(CLAUDE_NATIVE_PERMISSION_HOOK_TIMEOUT_SECS > CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS);

/// The bytes one more section can add to `hidden_context`, separator included,
/// and still fit the chunked hook delivery. Every chunk but the last closes at
/// least 5,997 bytes in, because a UTF-8 scalar is at most four bytes.
pub(crate) fn claude_native_hidden_context_room(hidden_context: &str) -> usize {
    let used = hidden_context.trim().len();
    (CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS * (CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES - 3))
        .saturating_sub(used + if used == 0 { 0 } else { 2 })
}

pub(crate) fn ensure_claude_native_hidden_context_fits(
    provider_run_id: &str,
    hidden_context: &str,
) -> Result<(), DaemonError> {
    let mut chunk_count = usize::from(!hidden_context.is_empty());
    let mut chunk_bytes = 0usize;
    for scalar in hidden_context.chars() {
        let scalar_bytes = scalar.len_utf8();
        if chunk_bytes > 0 && chunk_bytes + scalar_bytes > CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES {
            chunk_count += 1;
            chunk_bytes = 0;
        }
        chunk_bytes += scalar_bytes;
    }
    if chunk_count <= CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS {
        return Ok(());
    }
    Err(DaemonError::ProviderProtocol {
        provider_run_id: provider_run_id.to_string(),
        operation: "claude_hidden_context_size",
        message: format!(
            "hidden context is {} bytes and requires {} UTF-8-safe chunks; Claude native delivery supports {} chunks of at most {} bytes ({} bytes theoretical maximum)",
            hidden_context.len(),
            chunk_count,
            CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS,
            CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES,
            CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES
        ),
    })
}

// MP-08 / MP-10 / MP-11: optional home replay is sized against the complete worker turn.
pub(crate) fn fit_claude_native_hidden_context(
    provider_run_id: &str,
    hidden_context: String,
) -> Result<String, DaemonError> {
    if ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context).is_ok() {
        return Ok(hidden_context);
    }
    let open = crate::prompt_assembly::RECREATED_LEASE_CONTEXT_OPEN;
    let close = crate::prompt_assembly::RECREATED_LEASE_CONTEXT_CLOSE;
    let Some(start) = hidden_context.find(open) else {
        ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context)?;
        return Ok(hidden_context);
    };
    let body_start = start + open.len();
    let Some(body_end) = hidden_context[body_start..]
        .find(close)
        .map(|end| body_start + end)
    else {
        ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context)?;
        return Ok(hidden_context);
    };
    let prefix = &hidden_context[..start];
    let suffix = &hidden_context[body_end + close.len()..];
    let required = format!("{prefix}{suffix}");
    // Mandatory origin, skill, attachment and prompt assembly instructions are never truncated.
    ensure_claude_native_hidden_context_fits(provider_run_id, &required)?;
    let capacity = CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS * (CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES - 3);
    let room = capacity.saturating_sub(required.len() + open.len() + close.len());
    if room == 0 {
        ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context)?;
        return Ok(hidden_context);
    }
    let body = &hidden_context[body_start..body_end];
    const TRUNCATED: &str = "[truncated]\n";
    // Latest turn is last in the home packet. Retain that turn before older
    // summaries, trimming the start at a UTF-8 boundary and disclosing the cut.
    let keep = room.saturating_sub(TRUNCATED.len());
    if keep == 0 {
        ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context)?;
        return Ok(hidden_context);
    }
    let latest_start = body
        .find("\nLatest turn:\n")
        .map(|index| index + 1)
        .or_else(|| body.starts_with("Latest turn:\n").then_some(0))
        .unwrap_or(body.len());
    let (prior, latest) = body.split_at(latest_start);
    let mut latest_end = latest.len().min(keep);
    while !latest.is_char_boundary(latest_end) {
        latest_end -= 1;
    }
    if !latest.is_empty() && latest_end <= "Latest turn:\n".len() {
        // A header without any latest-turn detail is not a usable replay.
        ensure_claude_native_hidden_context_fits(provider_run_id, &hidden_context)?;
        return Ok(hidden_context);
    }
    let prior_room = keep - latest_end;
    let mut begin = prior.len().saturating_sub(prior_room);
    while !prior.is_char_boundary(begin) {
        begin += 1;
    }
    let fitted = format!(
        "{prefix}{open}{TRUNCATED}{}{}{close}{suffix}",
        &prior[begin..],
        &latest[..latest_end]
    );
    ensure_claude_native_hidden_context_fits(provider_run_id, &fitted)?;
    Ok(fitted)
}

pub(super) struct ClaudeNativeTuiFiles {
    root: ClaudeRuntimeFilesRoot,
    pub(super) events_file: PathBuf,
    pub(super) context_file: PathBuf,
    pub(super) context_response_dir: PathBuf,
    pub(super) permission_response_dir: PathBuf,
    pub(super) settings_file: PathBuf,
    pub(super) usage_file: PathBuf,
    mcp_config_file: Option<PathBuf>,
}

impl ClaudeNativeTuiFiles {
    pub(super) fn materialize_mcp_config(
        &mut self,
        request: &LaunchProviderRequest,
    ) -> Result<(), DaemonError> {
        self.mcp_config_file = materialize_request_claude_mcp_config(request, &self.root)?;
        Ok(())
    }

    pub(super) fn mcp_config_file(&self) -> Option<&Path> {
        self.mcp_config_file.as_deref()
    }

    pub(super) fn persist_for_launch(&mut self) {
        self.root.persist_for_launch();
    }
}

pub(super) fn prepare_claude_native_tui_files(
    request: &LaunchProviderRequest,
) -> Result<ClaudeNativeTuiFiles, DaemonError> {
    let root = create_claude_runtime_files_root()?;
    let events_file = root.path().join("events.jsonl");
    let context_file = root.path().join("hidden-context.txt");
    let context_response_dir = root.path().join("hook-context-responses");
    let permission_response_dir = root.path().join("permission-responses");
    let settings_file = root.path().join("settings.json");
    let hook_handler_file = root.path().join("hook-handler.mjs");
    let usage_capture = materialize_claude_usage_capture(&root)?;
    let usage_file = usage_capture.usage_file().to_path_buf();
    fs::create_dir_all(&context_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context response dir",
        message: error.to_string(),
    })?;
    fs::create_dir_all(&permission_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native permission response dir",
        message: error.to_string(),
    })?;
    fs::write(&events_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native events file",
        message: error.to_string(),
    })?;
    fs::write(&context_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context file",
        message: error.to_string(),
    })?;
    fs::write(&hook_handler_file, claude_native_hook_handler()).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "prepare claude native hook handler",
            message: error.to_string(),
        }
    })?;
    let hook_command = claude_native_hook_command(
        &hook_handler_file,
        &events_file,
        &context_file,
        &context_response_dir,
        &permission_response_dir,
        None,
    );
    let prompt_hooks = (0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS)
        .map(|chunk_index| {
            serde_json::json!({
                "type": "command",
                "command": claude_native_hook_command(
                    &hook_handler_file,
                    &events_file,
                    &context_file,
                    &context_response_dir,
                    &permission_response_dir,
                    Some(chunk_index),
                )
            })
        })
        .collect::<Vec<_>>();
    let settings = serde_json::json!({
        "skipDangerousModePermissionPrompt": request.permission_level.unwrap_or_default()
            == AgentPermissionLevel::Yolo,
        "hooks": {
            "SessionStart": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "UserPromptSubmit": [{ "hooks": prompt_hooks }],
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "StopFailure": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "SessionEnd": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            // Claude must not cancel the hook before its own decision wait.
            "PermissionRequest": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_command, "timeout": CLAUDE_NATIVE_PERMISSION_HOOK_TIMEOUT_SECS }] }]
        },
        "statusLine": {
            "type": "command",
            "command": usage_capture.command()
        }
    });
    let settings =
        serde_json::to_string_pretty(&settings).map_err(|error| DaemonError::LocalTransport {
            operation: "prepare claude native settings",
            message: error.to_string(),
        })?;
    fs::write(&settings_file, settings).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native settings file",
        message: error.to_string(),
    })?;
    Ok(ClaudeNativeTuiFiles {
        root,
        events_file,
        context_file,
        context_response_dir,
        permission_response_dir,
        settings_file,
        usage_file,
        mcp_config_file: None,
    })
}

fn claude_native_hook_command(
    hook_handler_file: &Path,
    events_file: &Path,
    context_file: &Path,
    context_response_dir: &Path,
    permission_response_dir: &Path,
    context_chunk_index: Option<usize>,
) -> String {
    let quoted = |path: &Path| {
        serde_json::to_string(&path.display().to_string())
            .expect("serializing a filesystem path should not fail")
    };
    let context_chunk = context_chunk_index
        .map(|index| format!(" CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK={index}"))
        .unwrap_or_default();
    format!(
        "CHARIOX_CLAUDE_NATIVE_EVENTS={} CHARIOX_CLAUDE_NATIVE_CONTEXT={} CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES={} CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES={}{} node {}",
        quoted(events_file),
        quoted(context_file),
        quoted(context_response_dir),
        quoted(permission_response_dir),
        context_chunk,
        quoted(hook_handler_file),
    )
}

fn claude_native_hook_handler() -> &'static str {
    include_str!("native_tui_hook.mjs")
}

pub(super) fn claude_native_tui_args(
    request: &LaunchProviderRequest,
    settings_file: &Path,
    mcp_config_file: Option<&Path>,
) -> Result<Vec<String>, DaemonError> {
    let mut args = vec![
        "--settings".to_string(),
        settings_file.display().to_string(),
        "--permission-mode".to_string(),
        match (
            request.execution_mode.unwrap_or_default(),
            request.permission_level.unwrap_or_default(),
        ) {
            (AgentExecutionMode::Plan, _) => "plan".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Required) => "default".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
                "bypassPermissions".to_string()
            }
        },
    ];
    let model = normalized_claude_model(&request.model);
    if !model.is_empty() && model != "default" {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(variant) = request
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend(["--effort".to_string(), variant.to_string()]);
    }
    if let Some(session_id) = request
        .resume_state
        .as_ref()
        .and_then(|state| state.claude_session_id())
    {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    if request.permission_level.unwrap_or_default() == AgentPermissionLevel::Yolo {
        args.push("--allow-dangerously-skip-permissions".to_string());
    }
    if let Some(config_file) = mcp_config_file {
        args.extend([
            "--mcp-config".to_string(),
            config_file.display().to_string(),
        ]);
        args.push("--strict-mcp-config".to_string());
        if request.runtime_mcp_binding.is_some() {
            args.extend(["--allowedTools".to_string(), "mcp__chariox__*".to_string()]);
        }
    }
    if request_uses_metaagent_tools_only(request) {
        args.extend(["--tools".to_string(), String::new()]);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use crate::provider::{
        AgentPermissionLevel, LaunchProviderRequest, ProviderResumeState, RuntimeMcpBinding,
    };

    use super::{
        claude_native_hidden_context_room, claude_native_hook_handler, claude_native_tui_args,
        ensure_claude_native_hidden_context_fits, prepare_claude_native_tui_files,
        CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES, CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS,
        CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES,
    };
    use crate::error::DaemonError;

    #[test]
    fn claude_stop_failure_hook_preserves_error_fields() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let input = serde_json::json!({
            "hook_event_name": "StopFailure",
            "error": "rate_limit",
            "error_details": "429 Too Many Requests",
            "last_assistant_message": "You've hit your session limit · resets 4am (Europe/Madrid)"
        });
        let mut child = Command::new("node")
            .arg(
                native
                    .events_file
                    .parent()
                    .unwrap()
                    .join("hook-handler.mjs"),
            )
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&native.events_file).unwrap()).unwrap();
        for key in [
            "hook_event_name",
            "error",
            "error_details",
            "last_assistant_message",
        ] {
            assert_eq!(
                recorded[key], input[key],
                "{key} must reach the native failure bridge"
            );
        }
    }

    #[test]
    fn hook_auto_allows_only_bypass_permissions() {
        let handler = claude_native_hook_handler();

        assert!(handler.contains("if (input.permission_mode === \"bypassPermissions\")"));
        assert!(handler.contains("behavior: \"allow\""));
        assert!(!handler.contains("permissionDecision"));
        assert!(!handler.contains("toolName.startsWith"));
        assert!(handler.contains("process.exit(0)"));
        assert!(handler.contains(&format!(
            "const deadline = Date.now() + {}",
            super::CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS * 1000
        )));
    }

    #[test]
    fn status_line_captures_official_subscription_windows_atomically() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-usage",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let handler = native
            .usage_file
            .parent()
            .expect("usage file should have a root")
            .join("usage-handler.mjs");
        let mut child = Command::new("node")
            .arg(handler)
            .env("CHARIOX_CLAUDE_USAGE_FILE", &native.usage_file)
            .stdin(Stdio::piped())
            .spawn()
            .expect("usage handler should start");
        child
            .stdin
            .take()
            .expect("usage stdin should be piped")
            .write_all(
                br#"{"rate_limits":{"five_hour":{"used_percentage":21},"seven_day":{"used_percentage":34}}}"#,
            )
            .expect("usage input should write");
        assert!(child.wait().expect("usage handler should finish").success());

        let captured: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.usage_file).expect("usage capture should exist"),
        )
        .expect("usage capture should be valid JSON");
        assert_eq!(captured["rate_limits"]["five_hour"]["used_percentage"], 21);
        assert!(!native
            .usage_file
            .parent()
            .expect("usage root")
            .read_dir()
            .expect("usage root should list")
            .flatten()
            .any(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("tmp")));

        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.settings_file).expect("settings should exist"),
        )
        .expect("settings should be valid JSON");
        assert_eq!(settings["statusLine"]["type"], "command");
        assert!(settings["statusLine"]["command"]
            .as_str()
            .is_some_and(|command| command.contains("usage-handler.mjs")));
    }

    #[test]
    fn status_line_preserves_usage_when_a_later_tick_has_no_rate_limits() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-usage-sequence",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.settings_file).expect("settings should exist"),
        )
        .expect("settings should be valid JSON");
        let usage_command = settings["statusLine"]["command"]
            .as_str()
            .expect("usage command");
        for payload in [
            br#"{"rate_limits":{"five_hour":{"used_percentage":21}}}"#.as_slice(),
            br#"{"model":{"display_name":"Claude"}}"#.as_slice(),
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", usage_command])
                // The generated native command must override an inherited
                // internal probe flag instead of capturing ordinary ticks.
                .env("CHARIOX_CLAUDE_CAPTURE_ALL", "1")
                .stdin(Stdio::piped())
                .spawn()
                .expect("usage handler should start");
            child
                .stdin
                .take()
                .expect("usage stdin should be piped")
                .write_all(payload)
                .expect("usage input should write");
            assert!(child.wait().expect("usage handler should finish").success());
        }

        let captured: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.usage_file).expect("usage capture should exist"),
        )
        .expect("usage capture should be valid JSON");
        assert_eq!(captured["rate_limits"]["five_hour"]["used_percentage"], 21);
    }

    #[test]
    fn permission_request_hook_uses_permission_request_specific_decision_contract() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-hook-contract",
            "claude",
            "claude-headless",
            "default",
            "opus",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let contract: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../fixtures/claude-permission-hook-contract.json"
        ))
        .expect("shared Claude permission contract should be valid JSON");

        for contract_case in contract {
            let mut child = Command::new("node")
                .arg(&hook_handler)
                .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                    &native.context_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                    &native.permission_response_dir,
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("hook handler should start");
            child
                .stdin
                .take()
                .expect("hook stdin should be piped")
                .write_all(
                    serde_json::to_string(&contract_case["input"])
                        .expect("contract input should serialize")
                        .as_bytes(),
                )
                .expect("hook input should write");

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while child
                .try_wait()
                .expect("hook process status should be readable")
                .is_none()
            {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "PermissionRequest hook blocked instead of resolving contract case {}",
                        contract_case["name"]
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let output = child
                .wait_with_output()
                .expect("hook handler should finish");
            assert!(
                output.status.success(),
                "hook handler failed for {}: {}",
                contract_case["name"],
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("hook response should be JSON");
            assert_eq!(
                response["hookSpecificOutput"]["hookEventName"], "PermissionRequest",
                "{}",
                contract_case["name"]
            );
            assert_eq!(
                response["hookSpecificOutput"]["decision"]["behavior"], contract_case["behavior"],
                "{}",
                contract_case["name"]
            );
        }
    }

    #[test]
    fn ask_mode_permission_request_hook_returns_the_kernel_decision() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-hook-ask",
            "claude",
            "claude-headless",
            "default",
            "opus",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let spawn_hook = |input: serde_json::Value| {
            let mut child = Command::new("node")
                .arg(&hook_handler)
                .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                    &native.context_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                    &native.permission_response_dir,
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("hook handler should start");
            child
                .stdin
                .take()
                .expect("hook stdin should be piped")
                .write_all(input.to_string().as_bytes())
                .expect("hook input should write");
            child
        };
        let recorded_request_ids = || {
            fs::read_to_string(&native.events_file)
                .expect("events should be readable")
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter_map(|event| {
                    event["hook_context_request_id"]
                        .as_str()
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        };

        // The kernel's decisions, and its pass-through when no client can
        // answer (see write_claude_permission_response and _passthrough).
        for decision in [
            serde_json::json!({ "behavior": "deny", "message": "Denied in the Room." }),
            serde_json::json!({ "behavior": "allow", "message": "" }),
            serde_json::json!({}),
        ] {
            let known = recorded_request_ids();
            let child = spawn_hook(serde_json::json!({
                "hook_event_name": "PermissionRequest",
                "permission_mode": "default",
                "tool_name": "Bash",
                "tool_input": { "command": "true" }
            }));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let request_id = loop {
                if let Some(id) = recorded_request_ids()
                    .into_iter()
                    .find(|id| !known.contains(id))
                {
                    break id;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "hook never recorded its permission request"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            fs::write(
                native
                    .permission_response_dir
                    .join(format!("{request_id}.json")),
                decision.to_string(),
            )
            .expect("decision should write");
            let output = child
                .wait_with_output()
                .expect("hook handler should finish");
            assert!(
                output.status.success(),
                "hook handler failed for {decision}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if decision.get("behavior").is_none() {
                // No decision: Claude falls back to its own dialog.
                assert!(output.stdout.is_empty(), "{decision}");
                continue;
            }
            let response: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("hook response should be JSON");
            assert_eq!(
                response["hookSpecificOutput"]["hookEventName"],
                "PermissionRequest"
            );
            assert_eq!(
                response["hookSpecificOutput"]["decision"]["behavior"],
                decision["behavior"]
            );
            if decision["behavior"] == "deny" {
                assert_eq!(
                    response["hookSpecificOutput"]["decision"]["message"],
                    decision["message"]
                );
            }
        }

        // A request that names no tool is never bridged, so the hook must
        // return at once instead of waiting for a decision.
        for tool_name in [serde_json::json!(" "), serde_json::json!(7)] {
            let started = std::time::Instant::now();
            let output = spawn_hook(serde_json::json!({
                "hook_event_name": "PermissionRequest",
                "permission_mode": "default",
                "tool_name": tool_name
            }))
            .wait_with_output()
            .expect("hook handler should finish");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            assert!(started.elapsed() < std::time::Duration::from_secs(3));
        }
    }

    #[test]
    fn print_prompt_hooks_use_the_current_dispatched_context() {
        let request = LaunchProviderRequest::new(
            "session-print-context",
            "claude",
            "claude-p",
            "default",
            "sonnet",
        );
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let hook = native
            .events_file
            .parent()
            .unwrap()
            .join("hook-handler.mjs");
        // MP-08 / MP-10 / MP-11: print turns have already entered the kernel;
        // their dispatched envelope must not wait for a native prompt request.
        for context in [
            format!("{}🦀END", "current-envelope\n".repeat(2000)),
            String::new(),
            "next envelope".into(),
        ] {
            fs::write(&native.context_file, &context).unwrap();
            let children = (0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS).map(|index| {
                let mut child = Command::new("node").arg(&hook)
                    .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES", &native.context_response_dir)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK", index.to_string())
                    .env("CHARIOX_CLAUDE_PROMPT_CONTEXT_SOURCE", "dispatched")
                    .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                    .spawn().unwrap();
                child.stdin.take().unwrap().write_all(
                    br#"{"hook_event_name":"UserPromptSubmit","session_id":"print","prompt":"same prompt"}"#,
                ).unwrap();
                child
            }).collect::<Vec<_>>();
            let mut chunks = Vec::new();
            for child in children {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "dispatched print context must not require a native response: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                chunks.push(
                    value["hookSpecificOutput"]["additionalContext"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                );
            }
            assert_eq!(chunks.concat(), context);
        }
    }

    #[test]
    fn user_prompt_hooks_deliver_only_the_current_response_to_every_chunk() {
        let request = LaunchProviderRequest::new(
            "session-fresh-context",
            "claude",
            "claude-headless",
            "default",
            "opus",
        );
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let hook = native
            .events_file
            .parent()
            .unwrap()
            .join("hook-handler.mjs");
        let large = format!("{}🦀END", "exact-record\n".repeat(3000));
        for wanted in [Some(large.as_str()), Some(""), Some("next response"), None] {
            fs::write(&native.context_file, "STALE PREVIOUS TURN").unwrap();
            fs::write(&native.events_file, "").unwrap();
            let children = (0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS).rev().map(|index| {
                let mut child = Command::new("node").arg(&hook)
                    .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES", &native.context_response_dir)
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK", index.to_string())
                    .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS", "1")
                    .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                    .spawn().expect("official hook should start");
                child.stdin.take().unwrap().write_all(
                    br#"{"hook_event_name":"UserPromptSubmit","session_id":"native","prompt":"same prompt"}"#,
                ).unwrap();
                (index, child)
            }).collect::<Vec<_>>();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let event = loop {
                if let Some(event) =
                    fs::read_to_string(&native.events_file)
                        .ok()
                        .and_then(|events| {
                            events.lines().next().and_then(|line| {
                                serde_json::from_str::<serde_json::Value>(line).ok()
                            })
                        })
                {
                    break event;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "primary hook did not request fresh context"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            if let Some(context) = wanted {
                let id = event["hook_context_request_id"].as_str().unwrap();
                fs::write(
                    native.context_response_dir.join(format!("{id}.txt")),
                    context,
                )
                .unwrap();
            }
            let mut chunks = vec![String::new(); CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS];
            for (index, child) in children {
                let output = child.wait_with_output().unwrap();
                if wanted.is_none() {
                    assert_eq!(
                        output.status.code(),
                        Some(2),
                        "missing current context must block the prompt"
                    );
                    assert!(output.stdout.is_empty());
                } else {
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let response: serde_json::Value =
                        serde_json::from_slice(&output.stdout).unwrap();
                    let chunk = response["hookSpecificOutput"]["additionalContext"]
                        .as_str()
                        .unwrap();
                    assert!(chunk.len() <= CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES);
                    chunks[index] = chunk.to_string();
                }
            }
            if let Some(context) = wanted {
                assert_eq!(chunks.concat(), context);
            }
            let events = fs::read_to_string(&native.events_file)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["hook_event_name"] == "UserPromptSubmit")
                    .count(),
                1,
                "chunk hooks must record the actual native prompt once"
            );
            if wanted.is_none() {
                assert!(
                    events
                        .iter()
                        .any(|e| e["hook_event_name"] == "CharioxContextFailed"
                            && e["hook_context_request_id"] == event["hook_context_request_id"]),
                    "failed hook did not notify the kernel"
                );
            }
            let receipts = events
                .iter()
                .filter(|e| e["hook_event_name"] == "CharioxContextDelivered")
                .collect::<Vec<_>>();
            assert_eq!(
                receipts.len(),
                usize::from(wanted.is_some()),
                "failed delivery must not acknowledge context"
            );
            if let Some(receipt) = receipts.first() {
                assert_eq!(
                    receipt["hook_context_request_id"],
                    event["hook_context_request_id"]
                );
                assert_eq!(receipt["session_id"], "native");
            }
            assert!(fs::read_dir(native.events_file.parent().unwrap())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".sock")));
        }
    }

    #[test]
    fn hidden_context_room_always_fits_the_chunked_delivery() {
        let ascii = "x".repeat(20_000);
        let multibyte = "🦀".repeat(5_000);
        for base in ["", "x", ascii.as_str(), multibyte.as_str()] {
            let room = claude_native_hidden_context_room(base);
            for fill in ["x", "é", "🦀"] {
                let section = fill.repeat(room / fill.len());
                let joined = if base.is_empty() {
                    section
                } else {
                    format!("{base}\n\n{section}")
                };
                ensure_claude_native_hidden_context_fits("run-room", &joined)
                    .expect("a section within the room should fit");
            }
        }
        assert_eq!(claude_native_hidden_context_room(&"x".repeat(60_000)), 0);
    }

    #[test]
    fn oversized_hidden_context_fails_before_claude_receives_a_partial_instruction_set() {
        let five_chunk_reviewer_context = "x".repeat(CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES * 5);
        ensure_claude_native_hidden_context_fits(
            "run-five-chunk-reviewer-context",
            &five_chunk_reviewer_context,
        )
        .expect("the native bridge should carry a bounded five-chunk reviewer context");

        let exact = "x".repeat(CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES);
        ensure_claude_native_hidden_context_fits("run-exact", &exact)
            .expect("the exact transport ceiling should be accepted");

        let mut multibyte_boundary_spill = "x".repeat(5_999);
        for _ in 0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS - 1 {
            multibyte_boundary_spill.push('🦀');
            multibyte_boundary_spill.push_str(&"x".repeat(5_995));
        }
        multibyte_boundary_spill.push('🦀');
        assert!(multibyte_boundary_spill.len() <= CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES);
        let error = ensure_claude_native_hidden_context_fits(
            "run-multibyte-boundary-spill",
            &multibyte_boundary_spill,
        )
        .expect_err("UTF-8 boundary spill requiring a fifth chunk must fail closed");
        assert!(matches!(
            error,
            DaemonError::ProviderProtocol {
                provider_run_id,
                operation: "claude_hidden_context_size",
                ..
            } if provider_run_id == "run-multibyte-boundary-spill"
        ));

        let oversized = format!("{exact}x");
        let error = ensure_claude_native_hidden_context_fits("run-oversized", &oversized)
            .expect_err("oversized hidden context must fail closed");
        assert!(matches!(
            error,
            DaemonError::ProviderProtocol {
                provider_run_id,
                operation: "claude_hidden_context_size",
                ..
            } if provider_run_id == "run-oversized"
        ));
    }

    #[test]
    fn session_start_hook_removes_materialized_mcp_credentials() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "private-token",
        ));
        let mut native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        native
            .materialize_mcp_config(&request)
            .expect("MCP config should materialize");
        let config_path = native
            .mcp_config_file()
            .expect("MCP config path should exist")
            .to_path_buf();
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let mut child = Command::new("node")
            .arg(hook_handler)
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("hook handler should start");
        child
            .stdin
            .take()
            .expect("hook stdin should be piped")
            .write_all(br#"{"hook_event_name":"SessionStart"}"#)
            .expect("hook input should write");

        let output = child
            .wait_with_output()
            .expect("hook handler should finish");

        assert!(
            output.status.success(),
            "hook handler failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!config_path.exists());
        assert!(std::fs::read_to_string(&native.events_file)
            .expect("hook event should be recorded")
            .contains("SessionStart"));
    }

    #[test]
    fn native_session_start_preserves_session_identity_without_a_transcript() {
        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "haiku");
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let hook = native
            .events_file
            .parent()
            .unwrap()
            .join("hook-handler.mjs");
        let mut child = Command::new("node")
            .arg(hook)
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"SessionStart","source":"clear","session_id":"fresh-session-before-any-turn"}"#).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let event: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(native.events_file).unwrap()).unwrap();
        assert_eq!(event["session_id"], "fresh-session-before-any-turn");
    }

    #[test]
    fn native_session_start_writes_an_independent_identity_receipt() {
        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "haiku");
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let hook = native
            .events_file
            .parent()
            .unwrap()
            .join("hook-handler.mjs");
        let mut child = Command::new("node")
            .arg(hook)
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"SessionStart","source":"clear","session_id":"fresh-session-before-any-turn"}"#).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let event: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&native.events_file).unwrap()).unwrap();
        assert_eq!(event["session_id"], "fresh-session-before-any-turn");
        fs::write(&native.events_file, "").unwrap();
        assert_eq!(
            fs::read_to_string(format!(
                "{}.session-start-id",
                native.context_file.display()
            ))
            .unwrap(),
            "fresh-session-before-any-turn"
        );
    }

    #[test]
    fn native_tui_resumes_requested_claude_session() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_resume_state(ProviderResumeState::from_claude_session_id(
            "claude-session-1",
        ));

        let args = claude_native_tui_args(&request, Path::new("settings.json"), None)
            .expect("Claude native TUI args should resolve");

        assert!(args
            .windows(2)
            .any(|pair| pair == ["--resume", "claude-session-1"]));
    }

    #[test]
    fn native_settings_accept_dangerous_mode_only_for_yolo_agents() {
        let yolo = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_permission_level(AgentPermissionLevel::Yolo);
        let required = LaunchProviderRequest::new(
            "session-2",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_permission_level(AgentPermissionLevel::Required);

        let yolo_files = prepare_claude_native_tui_files(&yolo).unwrap();
        let required_files = prepare_claude_native_tui_files(&required).unwrap();
        let yolo_settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(yolo_files.settings_file).unwrap()).unwrap();
        let required_settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(required_files.settings_file).unwrap())
                .unwrap();

        assert_eq!(yolo_settings["skipDangerousModePermissionPrompt"], true);
        assert_eq!(
            required_settings["skipDangerousModePermissionPrompt"],
            false
        );
        let yolo_hook = yolo_settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_EVENTS="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_CONTEXT="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES="));
        assert_eq!(
            yolo_settings["hooks"]["PermissionRequest"][0]["matcher"],
            "*"
        );
        assert_eq!(
            yolo_settings["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            360
        );
    }
}
