use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::account_profile::ProviderAccountUsageSnapshot;
use crate::error::DaemonError;

use super::mcp_config::create_claude_runtime_files_root;

const CLAUDE_USAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_USAGE_PROBE_OUTPUT_BYTES: u64 = 1024 * 1024;
const CLAUDE_CREDENTIAL_CHECK_TIMEOUT: Duration = Duration::from_secs(90);

pub(crate) fn probe_claude_account_usage(
    executable: &Path,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
    probe_claude_account_usage_with_timeout(
        executable,
        account_profile,
        environment,
        CLAUDE_USAGE_PROBE_TIMEOUT,
    )
}

fn probe_claude_account_usage_with_timeout(
    executable: &Path,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
    let (status, result) = run_claude_print(
        executable,
        environment,
        &["/usage"],
        timeout,
        "Claude usage probe",
    )?;
    if !status.success() {
        return Err(probe_error(format!(
            "Claude exited before reporting usage ({status})"
        )));
    }
    let result = result.ok_or_else(|| probe_error("Claude returned invalid usage JSON".into()))?;
    let text = validated_claude_usage_result(&result)?;
    let mut usage = claude_usage_snapshot_from_text(text).ok_or_else(|| {
        probe_error("Claude did not return subscription usage windows".to_string())
    })?;
    usage.profile_id = account_profile.to_string();
    usage.source = "claude.native_usage".to_string();
    for meter in &mut usage.meters {
        meter.source = "claude.native_usage".to_string();
    }
    Ok(usage)
}

/// Why Claude did not accept an account credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaudeCredentialCheckError {
    /// Claude's API refused the credential: invalid, expired or revoked.
    Rejected,
    /// The check could not reach a verdict, e.g. network or launcher failure.
    Inconclusive(String),
}

impl ClaudeCredentialCheckError {
    pub(crate) fn into_daemon_error(self) -> DaemonError {
        let message = match self {
            ClaudeCredentialCheckError::Rejected => "Claude rejected the setup token: invalid, expired or revoked. Choose Log in in Provider Accounts to authorize a replacement.".into(),
            ClaudeCredentialCheckError::Inconclusive(reason) => format!("Claude verification was inconclusive: {reason}. Check the network and Claude CLI, then retry Status or Refresh."),
        };
        DaemonError::LocalTransport {
            operation: "verify Claude setup token",
            message,
        }
    }
}

/// Verifies a Claude credential with one minimal model turn. Claude only
/// checks a `CLAUDE_CODE_OAUTH_TOKEN` when it calls the API: `auth status`
/// and local commands such as `/usage` report success for any token.
pub(crate) fn verify_claude_account_credential(
    executable: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(), ClaudeCredentialCheckError> {
    verify_claude_account_credential_with_timeout(
        executable,
        environment,
        CLAUDE_CREDENTIAL_CHECK_TIMEOUT,
    )
}

pub(super) fn verify_claude_account_credential_with_timeout(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(), ClaudeCredentialCheckError> {
    let (status, result) = run_claude_print(
        executable,
        environment,
        &[
            "Reply with exactly CHARIOX_AUTH_OK.",
            "--model",
            "haiku",
            "--max-turns",
            "1",
            "--system-prompt",
            "Reply only with the requested authentication marker.",
        ],
        timeout,
        "Claude credential check",
    )
    .map_err(|error| ClaudeCredentialCheckError::Inconclusive(error.to_string()))?;
    let result = result.unwrap_or_default();
    if matches!(
        result
            .get("api_error_status")
            .and_then(serde_json::Value::as_u64),
        Some(401)
    ) {
        return Err(ClaudeCredentialCheckError::Rejected);
    }
    // Older official CLI results expose the API status only in their error
    // text. Restrict fallback matching to failed result envelopes; never treat
    // a successful model's quoted error as a rejected credential.
    let failed_result = result.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && result.get("is_error").and_then(serde_json::Value::as_bool) == Some(true);
    let rejected_text = |text: &str| {
        let status = result
            .get("api_error_status")
            .and_then(serde_json::Value::as_u64);
        crate::provider::renewal_failure::claude_api_auth_failure(text)
            || (status == Some(403)
                && crate::provider::renewal_failure::claude_api_auth_failure(&format!(
                    "API Error: 403 {text}"
                )))
    };
    if failed_result
        && (result
            .get("result")
            .and_then(serde_json::Value::as_str)
            .is_some_and(rejected_text)
            || result
                .get("errors")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|errors| {
                    errors
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(rejected_text)
                }))
    {
        return Err(ClaudeCredentialCheckError::Rejected);
    }
    let single_turn = super::auth_probe::validate_auth_result(&result).is_ok();
    if status.success() && single_turn {
        Ok(())
    } else {
        // Only harness metadata is projected. Provider result/error text may
        // contain account details or secrets and never belongs in a diagnostic.
        let http = result
            .get("api_error_status")
            .and_then(serde_json::Value::as_u64)
            .filter(|code| (100..600).contains(code))
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unreported".into());
        let turns = result
            .get("num_turns")
            .and_then(serde_json::Value::as_u64)
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unreported".into());
        Err(ClaudeCredentialCheckError::Inconclusive(format!(
            "Claude did not complete verification ({status}; HTTP={http}; turns={turns}; valid_single_turn={single_turn})"
        )))
    }
}

/// Runs one bounded, tool-less `claude -p <args[0]> ... <args[1..]>` in a
/// private scratch directory and returns its exit status and JSON result.
fn run_claude_print(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    args: &[&str],
    timeout: Duration,
    label: &'static str,
) -> Result<(std::process::ExitStatus, Option<serde_json::Value>), DaemonError> {
    let mut command_args = vec![
        "-p",
        args[0],
        "--output-format",
        "json",
        "--no-session-persistence",
        "--no-chrome",
        "--tools",
        "",
    ];
    command_args.extend_from_slice(&args[1..]);
    let (status, output) =
        run_claude_capture(executable, environment, &command_args, None, timeout, label)?;
    Ok((status, serde_json::from_slice(&output).ok()))
}

/// Official stream-json initialization reports the native picker without a model turn.
pub(super) fn probe_claude_models(
    executable: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<serde_json::Value, DaemonError> {
    let input = b"{\"type\":\"control_request\",\"request_id\":\"chariox-catalog\",\"request\":{\"subtype\":\"initialize\"}}\n";
    let (status, output) = run_claude_capture(
        executable,
        environment,
        &[
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--no-chrome",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
        ],
        Some(input),
        Duration::from_secs(10),
        "Claude model catalog",
    )?;
    if status.success() {
        for line in output.split(|byte| *byte == b'\n') {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
                continue;
            };
            if value["type"] == "control_response"
                && value["response"]["request_id"] == "chariox-catalog"
                && value["response"]["subtype"] == "success"
            {
                if let Some(models) = value["response"]["response"]["models"].as_array() {
                    return Ok(serde_json::Value::Array(models.clone()));
                }
            }
        }
    }
    Err(probe_error(
        "Claude did not return native model metadata".into(),
    ))
}

fn run_claude_capture(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
    label: &'static str,
) -> Result<(std::process::ExitStatus, Vec<u8>), DaemonError> {
    let print_error = |message: String| DaemonError::LocalTransport {
        operation: label,
        message,
    };
    validate_claude_probe_environment(environment, linux_profile_home_is_supported())?;
    let root = create_claude_runtime_files_root()?;
    let stdout_path = root.path().join("print-result.json");
    let stderr_path = root.path().join("print-stderr.log");
    let stdout = private_probe_file(&stdout_path)
        .map_err(|error| print_error(format!("failed to prepare Claude stdout: {error}")))?;
    let stderr = private_probe_file(&stderr_path)
        .map_err(|error| print_error(format!("failed to prepare Claude stderr: {error}")))?;

    let mut stdout_metadata = stdout
        .try_clone()
        .map_err(|_| print_error("failed to pin Claude stdout".into()))?;
    let stderr_metadata = stderr
        .try_clone()
        .map_err(|_| print_error("failed to pin Claude stderr".into()))?;
    // Same managed boundary and ambient-credential scrub as the auth-status probe.
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        args.iter().map(|arg| (*arg).to_string()).collect(),
        environment.clone(),
        None,
        label,
    )?;
    command.current_dir(root.path());
    if !environment.contains_key(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
        command.env_remove(crate::provider::CLAUDE_OAUTH_TOKEN_ENV);
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
        // These select a different authentication source before OAuth. A
        // successful cloud-provider turn would not verify this setup token.
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        command.env_remove(name);
    }
    command.env("DISABLE_AUTOUPDATER", "1");
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A monitored spool alone can overshoot between polls. The child has a
        // hard per-file cap as well; only this owned probe inherits the limit.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: CLAUDE_USAGE_PROBE_OUTPUT_BYTES as libc::rlim_t,
                    rlim_max: CLAUDE_USAGE_PROBE_OUTPUT_BYTES as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = command
        .spawn()
        .map_err(|error| print_error(format!("failed to start Claude: {error}")))?;
    if let Some(input) = input {
        let result = child
            .stdin
            .take()
            .expect("piped catalog input")
            .write_all(input);
        if result.is_err() {
            stop_claude_probe(&mut child);
            return Err(print_error(format!("failed to initialize {label}")));
        }
    }
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        let overflow = [&stdout_metadata, &stderr_metadata]
            .into_iter()
            .any(|file| {
                file.metadata().map_or(true, |metadata| {
                    metadata.len() >= CLAUDE_USAGE_PROBE_OUTPUT_BYTES
                })
            });
        if overflow {
            stop_claude_probe(&mut child);
            return Err(print_error(format!("{label} output limit exceeded")));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            stop_claude_probe(&mut child);
            return Err(print_error(format!("{label} timed out")));
        }
        match child.wait_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                stop_claude_probe(&mut child);
                return Err(print_error(format!("failed to wait for {label}")));
            }
        }
    };
    if [&stdout_metadata, &stderr_metadata]
        .into_iter()
        .any(|file| {
            file.metadata().map_or(true, |metadata| {
                metadata.len() >= CLAUDE_USAGE_PROBE_OUTPUT_BYTES
            })
        })
    {
        return Err(print_error(format!("{label} output limit exceeded")));
    }
    let mut output = Vec::new();
    stdout_metadata
        .rewind()
        .map_err(|_| print_error(format!("failed to rewind {label} output")))?;
    stdout_metadata
        .take(CLAUDE_USAGE_PROBE_OUTPUT_BYTES + 1)
        .read_to_end(&mut output)
        .map_err(|_| print_error(format!("failed to read {label} output")))?;
    if output.len() as u64 > CLAUDE_USAGE_PROBE_OUTPUT_BYTES {
        return Err(print_error(format!("{label} output limit exceeded")));
    }
    Ok((status, output))
}

fn stop_claude_probe(child: &mut std::process::Child) {
    // Only this directly spawned child is owned; never form a group target.
    if child.id() > 1 {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn validated_claude_usage_result(value: &serde_json::Value) -> Result<&str, DaemonError> {
    let success = value.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && value.get("subtype").and_then(serde_json::Value::as_str) == Some("success")
        && value.get("is_error").and_then(serde_json::Value::as_bool) == Some(false);
    if !success {
        return Err(probe_error(
            "Claude returned an unsuccessful usage result".to_string(),
        ));
    }
    let no_model_activity = value
        .get("duration_api_ms")
        .and_then(serde_json::Value::as_u64)
        == Some(0)
        && value.get("num_turns").and_then(serde_json::Value::as_u64) == Some(0)
        && value
            .get("total_cost_usd")
            .and_then(serde_json::Value::as_f64)
            == Some(0.0)
        && [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "output_tokens",
        ]
        .into_iter()
        .all(|field| {
            value
                .get("usage")
                .and_then(|usage| usage.get(field))
                .and_then(serde_json::Value::as_u64)
                == Some(0)
        });
    if !no_model_activity {
        return Err(probe_error(
            "Claude /usage unexpectedly performed model activity".to_string(),
        ));
    }
    value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| probe_error("Claude usage result text is unavailable".to_string()))
}

fn claude_usage_snapshot_from_text(text: &str) -> Option<ProviderAccountUsageSnapshot> {
    let mut rate_limits = serde_json::Map::new();
    for line in text.lines().map(str::trim) {
        if let Some(percent) = usage_percent(line, "Current session:") {
            rate_limits.insert(
                "five_hour".to_string(),
                serde_json::json!({ "used_percentage": percent }),
            );
        } else if let Some(percent) = usage_percent(line, "Current week (all models):")
            .or_else(|| usage_percent(line, "Current week:"))
        {
            rate_limits.insert(
                "seven_day".to_string(),
                serde_json::json!({ "used_percentage": percent }),
            );
        }
    }
    crate::provider::claude_status_line_usage_snapshot(&serde_json::json!({
        "rate_limits": rate_limits,
    }))
}

fn usage_percent(line: &str, prefix: &str) -> Option<f64> {
    let percent = line
        .strip_prefix(prefix)?
        .trim_start()
        .split_once('%')?
        .0
        .trim()
        .parse::<f64>()
        .ok()?;
    (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent)
}

fn validate_claude_probe_environment(
    environment: &BTreeMap<String, String>,
    profile_home_is_supported: bool,
) -> Result<(), DaemonError> {
    if environment.contains_key("HOME") && !profile_home_is_supported {
        return Err(probe_error(
            "HOME-based Claude account profiles are supported only on Linux; refusing to override HOME on this platform"
                .to_string(),
        ));
    }
    Ok(())
}

fn linux_profile_home_is_supported() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(test)]
fn terminal_diagnostic(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let mut cleaned = String::with_capacity(text.len());
    let mut escape = false;
    for character in text.chars() {
        if escape {
            if character.is_ascii_alphabetic() || character == '\u{7}' {
                escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            escape = true;
        } else if character.is_control() {
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn probe_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "refresh Claude usage",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[cfg(unix)]
    #[test]
    fn model_probe_initializes_without_sending_a_user_turn() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile_fixture_root();
        let executable = root.join("claude");
        let request = root.join("request.json");
        let response = serde_json::json!({
            "type":"control_response", "response":{"subtype":"success", "request_id":"chariox-catalog",
                "response":{"models":[{"value":"haiku", "resolvedModel":"claude-haiku-5-5", "displayName":"Haiku"}]}}
        });
        fs::write(&executable, format!("#!/bin/sh\nread -r input\nprintf '%s' \"$input\" > '{}'\ncat >/dev/null\nprintf '%s\\n' '{}'\n", request.display(), response)).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        let models = probe_claude_models(&executable, &environment).unwrap();
        assert_eq!(models[0]["resolvedModel"], "claude-haiku-5-5");
        let input: serde_json::Value = serde_json::from_slice(&fs::read(request).unwrap()).unwrap();
        assert_eq!(input["type"], "control_request");
        assert_eq!(input["request"]["subtype"], "initialize");
        assert!(input.get("message").is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    fn catalog_probe_fixture(root: &Path, marker: &Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let executable = root.join("claude");
        let response = serde_json::json!({
            "type":"control_response", "response":{"subtype":"success", "request_id":"chariox-catalog",
                "response":{"models":[{"value":"haiku", "resolvedModel":"claude-haiku-5-5", "displayName":"Haiku"}]}}
        });
        fs::write(&executable, format!("#!/bin/sh\nprintf '%s' \"${{CLAUDE_CODE_OAUTH_TOKEN:-unset}}\" > '{}'\ncat >/dev/null\nprintf '%s\\n' '{}'\n", marker.display(), response)).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        executable
    }

    // MP-11 reviewer P1: the catalog probe uses the selected credential, never an inherited one.
    #[cfg(unix)]
    #[test]
    fn model_probe_drops_ambient_claude_credentials_unless_selected() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let previous = std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN");
        let isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "ambient");
        let root = tempfile_fixture_root();
        let marker = root.join("token");
        let executable = catalog_probe_fixture(&root, &marker);
        let mut environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        let ambient =
            probe_claude_models(&executable, &environment).map(|_| fs::read_to_string(&marker));
        environment.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), "selected".into());
        let selected =
            probe_claude_models(&executable, &environment).map(|_| fs::read_to_string(&marker));
        match previous {
            Some(value) => std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", value),
            None => std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN"),
        }
        if let Some(value) = isolation {
            std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value);
        }
        fs::remove_dir_all(root).unwrap();
        assert_eq!(ambient.unwrap().unwrap(), "unset");
        assert_eq!(selected.unwrap().unwrap(), "selected");
    }

    // MP-11 reviewer P1: a managed kernel never starts the catalog probe outside its provider boundary.
    #[cfg(unix)]
    #[test]
    fn model_probe_does_not_bypass_managed_isolation() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let root = tempfile_fixture_root();
        let marker = root.join("token");
        let executable = catalog_probe_fixture(&root, &marker);
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        let result = probe_claude_models(&executable, &environment);
        let ran_unconfined = marker.exists();
        match isolation {
            Some(value) => {
                std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
            }
            None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
        }
        fs::remove_dir_all(root).unwrap();
        assert!(
            !ran_unconfined,
            "catalog probe ran the provider outside managed isolation"
        );
        assert!(
            result.is_err(),
            "unconfigured managed isolation must fail closed"
        );
    }

    #[test]
    fn probes_both_usage_windows_without_replacing_home() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let fixture = std::env::temp_dir().join(format!(
            "chariox-claude-usage-probe-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&fixture).expect("fixture root");
        let executable = fixture.join("fake-claude.mjs");
        let observed_environment = fixture.join("environment.json");
        let claude_config_dir = fixture.join("claude-account");
        let source = fixture.join("fake-claude-source.mjs");
        fs::write(
            &source,
            br#"#!/usr/bin/env node
import { writeFileSync } from "node:fs"
const expected = ["-p", "/usage", "--output-format", "json", "--no-session-persistence", "--no-chrome", "--tools", ""]
if (JSON.stringify(process.argv.slice(2)) !== JSON.stringify(expected)) {
  throw new Error(`unexpected Claude usage arguments: ${JSON.stringify(process.argv.slice(2))}`)
}
writeFileSync(process.env.CHARIOX_PROBE_TEST_ENV_FILE, JSON.stringify({
  home: process.env.HOME,
  claudeConfigDir: process.env.CLAUDE_CONFIG_DIR,
  anthropicApiKey: process.env.ANTHROPIC_API_KEY,
  anthropicAuthToken: process.env.ANTHROPIC_AUTH_TOKEN,
  anthropicBaseUrl: process.env.ANTHROPIC_BASE_URL,
  anthropicCustomHeaders: process.env.ANTHROPIC_CUSTOM_HEADERS,
  bedrock: process.env.CLAUDE_CODE_USE_BEDROCK,
  vertex: process.env.CLAUDE_CODE_USE_VERTEX,
  foundry: process.env.CLAUDE_CODE_USE_FOUNDRY,
  oauthPresent: process.env.CLAUDE_CODE_OAUTH_TOKEN === "fixture-oauth"
}))
process.stdout.write(JSON.stringify({
  type: "result",
  subtype: "success",
  is_error: false,
  duration_api_ms: 0,
  num_turns: 0,
  total_cost_usd: 0,
  result: "Current session: 17% used\nCurrent week (all models): 41% used \\u00b7 resets Sep 2 at 11:59pm (Europe/Helsinki)",
  usage: { input_tokens: 0, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 0 }
}))
"#,
        )
        .expect("fake Claude source");
        // A concurrent fork can retain this process's writable descriptors.
        // Like TestTool, install from a child and wait for it to exit: the
        // parallel test process never holds the executable open for writing.
        let installed = Command::new("install")
            .args(["-m", "700"])
            .arg(&source)
            .arg(&executable)
            .status()
            .expect("install fake Claude executable");
        assert!(installed.success(), "install fake Claude: {installed}");
        let inherited_home = std::env::var("HOME").expect("test HOME");
        let environment = BTreeMap::from([
            ("CLAUDE_CODE_USE_BEDROCK".into(), "1".into()),
            ("CLAUDE_CODE_USE_VERTEX".into(), "1".into()),
            ("CLAUDE_CODE_USE_FOUNDRY".into(), "1".into()),
            ("CLAUDE_CODE_OAUTH_TOKEN".into(), "fixture-oauth".into()),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                claude_config_dir.display().to_string(),
            ),
            (
                "CHARIOX_PROBE_TEST_ENV_FILE".to_string(),
                observed_environment.display().to_string(),
            ),
            ("ANTHROPIC_API_KEY".to_string(), "wrong-api-key".to_string()),
            (
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                "wrong-auth-token".to_string(),
            ),
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://wrong.invalid".to_string(),
            ),
            (
                "ANTHROPIC_CUSTOM_HEADERS".to_string(),
                "x-wrong: yes".to_string(),
            ),
        ]);

        let usage = probe_claude_account_usage_with_timeout(
            &executable,
            "claude-2",
            &environment,
            Duration::from_secs(5),
        )
        .expect("usage probe");

        assert_eq!(usage.profile_id, "claude-2");
        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].used_percent, Some(17.0));
        assert_eq!(usage.meters[1].used_percent, Some(41.0));
        assert_eq!(usage.source, "claude.native_usage");
        let observed: serde_json::Value =
            serde_json::from_slice(&fs::read(&observed_environment).expect("observed environment"))
                .expect("environment JSON");
        assert_eq!(observed["home"], inherited_home);
        assert_eq!(
            observed["claudeConfigDir"],
            environment["CLAUDE_CONFIG_DIR"]
        );
        for key in [
            "anthropicApiKey",
            "anthropicAuthToken",
            "anthropicBaseUrl",
            "anthropicCustomHeaders",
            "bedrock",
            "vertex",
            "foundry",
        ] {
            assert!(observed.get(key).is_none(), "{key} must be removed");
        }
        assert_eq!(observed["oauthPresent"], true);
        assert!(
            !claude_config_dir.exists(),
            "the non-interactive probe must not mutate Claude profile state"
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[test]
    #[cfg(unix)]
    fn credential_check_classifies_legacy_api_rejections_and_transient_errors() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile_fixture_root();
        let executable = root.join("claude");
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        for (result, exit, rejected) in [
            (
                serde_json::json!({"type":"result", "is_error":true, "result":"API Error: 401 OAuth token is invalid"}),
                1,
                true,
            ),
            (
                serde_json::json!({"type":"result", "is_error":true, "errors":["API Error: 403 access denied"]}),
                1,
                false,
            ),
            (
                serde_json::json!({"type":"result", "is_error":true, "api_error_status":403, "result":"permission_error: model access denied"}),
                1,
                false,
            ),
            (
                serde_json::json!({"type":"result", "is_error":true, "api_error_status":403, "result":"Invalid authentication credentials"}),
                1,
                true,
            ),
            (
                serde_json::json!({"type":"result", "is_error":true, "result":"API Error: 503 service unavailable"}),
                1,
                false,
            ),
            (
                serde_json::json!({"type":"result", "is_error":true, "result":"connection timed out"}),
                1,
                false,
            ),
            (
                serde_json::json!({"type":"result", "is_error":false, "result":"API Error: 401 quoted by model"}),
                0,
                false,
            ),
        ] {
            fs::write(
                &executable,
                format!("#!/bin/sh\nprintf '%s\\n' '{result}'\nexit {exit}\n"),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let checked = verify_claude_account_credential_with_timeout(
                &executable,
                &environment,
                Duration::from_secs(2),
            );
            assert_eq!(
                checked == Err(ClaudeCredentialCheckError::Rejected),
                rejected,
                "{result}: {checked:?}"
            );
            if exit == 0 {
                assert!(matches!(checked, Err(ClaudeCredentialCheckError::Inconclusive(_))),
                    "MP-08/MP-11: a quoted 401 is not rejection, and a non-marker reply is not verification");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn credential_check_requires_verified_single_turn_and_redacts_failures() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile_fixture_root();
        let executable = root.join("claude");
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        for valid in [true, false] {
            let result = serde_json::json!({"type":"result", "subtype":"success", "is_error":false,
                "num_turns":1, "result": if valid { "CHARIOX_AUTH_OK" } else { "synthetic-private-provider-detail" },
                "usage":{"output_tokens":7}});
            fs::write(
                &executable,
                format!("#!/bin/sh\nprintf '%s\\n' '{result}'\n"),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let checked = verify_claude_account_credential_with_timeout(
                &executable,
                &environment,
                Duration::from_secs(2),
            );
            assert_eq!(checked.is_ok(), valid);
            if let Err(error) = checked {
                let message = error.into_daemon_error().to_string();
                assert!(message.contains("turns=1; valid_single_turn=false"));
                assert!(!message.contains("synthetic-private-provider-detail"));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn credential_check_timeout_names_the_check() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile_fixture_root();
        let executable = root.join("claude");
        fs::write(&executable, "#!/bin/sh\nexec sleep 10\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            root.join("profile").display().to_string(),
        )]);
        let error = verify_claude_account_credential_with_timeout(
            &executable,
            &environment,
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(
            matches!(error, ClaudeCredentialCheckError::Inconclusive(ref message) if message.contains("credential check timed out") && !message.contains("usage"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    fn tempfile_fixture_root() -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("claude-check-{:016x}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn rejects_model_activity_in_usage_result() {
        let value = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "duration_api_ms": 1,
            "num_turns": 1,
            "total_cost_usd": 0.01,
            "result": "Current session: 17% used",
            "usage": {
                "input_tokens": 1,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "output_tokens": 1
            }
        });

        let error = validated_claude_usage_result(&value)
            .expect_err("model activity must fail the usage probe");

        assert!(error.to_string().contains("performed model activity"));
    }

    #[test]
    fn parses_official_claude_usage_text() {
        let usage = claude_usage_snapshot_from_text(
            "You are currently using your subscription\n\nCurrent session: 12.5% used\nCurrent week (all models): 84% used · resets Sep 2 at 11:59pm (Europe/Helsinki)",
        )
        .expect("usage snapshot");

        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].used_percent, Some(12.5));
        assert_eq!(usage.meters[1].used_percent, Some(84.0));
    }

    #[test]
    fn rejects_legacy_command_summary_without_subscription_windows() {
        assert!(claude_usage_snapshot_from_text(
            "Total cost: $0.0000\nTotal duration (API): 0s\nUsage: 0 input, 0 output, 0 cache read, 0 cache write",
        )
        .is_none());
    }

    #[test]
    fn rejects_account_home_override_when_profile_home_is_unsupported() {
        let environment = BTreeMap::from([("HOME".to_string(), "/tmp/account".to_string())]);

        let error = validate_claude_probe_environment(&environment, false)
            .expect_err("unsupported HOME override must fail");

        assert!(error.to_string().contains("supported only on Linux"));
    }

    #[test]
    fn terminal_diagnostics_strip_control_sequences_and_collapse_whitespace() {
        assert_eq!(
            terminal_diagnostic(b"\x1b[31mLogin\x1b[0m\r\n  required\t now"),
            "Login required now"
        );
    }
}

#[cfg(all(test, unix))]
#[test]
fn mp11_usage_probe_rejects_stdout_and_stderr_bursts_before_allocating_them() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("mp11-usage-{:016x}", rand::random::<u64>()));
    fs::create_dir(&root).unwrap();
    for pipe in ["stdout", "stderr"] {
        let executable = root.join("fixture.mjs");
        fs::write(
            &executable,
            format!("#!/usr/bin/env node\nprocess.{pipe}.write(Buffer.alloc(8388608));\n"),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".to_string(),
            root.join("profile").display().to_string(),
        )]);
        let start = std::time::Instant::now();
        let error = probe_claude_account_usage_with_timeout(
            &executable,
            "synthetic",
            &environment,
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("output limit"),
            "burst was not rejected at the output boundary"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    fs::remove_dir_all(root).unwrap();
}

fn private_probe_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options.open(path)
}
