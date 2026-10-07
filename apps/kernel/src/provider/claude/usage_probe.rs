use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::Path;
use std::process::{Command, Stdio};
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
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClaudeCredentialCheckError {
    /// Claude's API refused the credential: invalid, expired or revoked.
    Rejected,
    /// The check could not reach a verdict, e.g. network or launcher failure.
    Inconclusive(String),
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

fn verify_claude_account_credential_with_timeout(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(), ClaudeCredentialCheckError> {
    let (status, result) = run_claude_print(
        executable,
        environment,
        &["Reply with OK.", "--model", "haiku"],
        timeout,
        "Claude credential check",
    )
    .map_err(|error| ClaudeCredentialCheckError::Inconclusive(error.to_string()))?;
    let result = result.unwrap_or_default();
    if matches!(
        result
            .get("api_error_status")
            .and_then(serde_json::Value::as_u64),
        Some(401 | 403)
    ) {
        return Err(ClaudeCredentialCheckError::Rejected);
    }
    // Older official CLI results expose the API status only in their error
    // text. Restrict fallback matching to failed result envelopes; never treat
    // a successful model's quoted error as a rejected credential.
    let failed_result = result.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && result.get("is_error").and_then(serde_json::Value::as_bool) == Some(true);
    let rejected_text = |text: &str| {
        text.split_once("API Error: ")
            .and_then(|(_, status)| status.split_whitespace().next())
            .is_some_and(|status| matches!(status.trim_end_matches(':'), "401" | "403"))
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
    let succeeded = result.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && result.get("is_error").and_then(serde_json::Value::as_bool) == Some(false);
    if status.success() && succeeded {
        Ok(())
    } else {
        Err(ClaudeCredentialCheckError::Inconclusive(format!(
            "Claude did not complete the check ({status})"
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
    let mut command = Command::new(executable);
    command.arg("-p").arg(args[0]).args([
        "--output-format",
        "json",
        "--no-session-persistence",
        "--no-chrome",
        "--tools",
        "",
    ]);
    command.args(&args[1..]);
    command.current_dir(root.path());
    for (name, value) in environment {
        command.env(name, value);
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
    ] {
        command.env_remove(name);
    }
    command.env("DISABLE_AUTOUPDATER", "1");
    command
        .stdin(Stdio::null())
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
    Ok((status, serde_json::from_slice(&output).ok()))
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

    #[cfg(unix)]
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
  anthropicCustomHeaders: process.env.ANTHROPIC_CUSTOM_HEADERS
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
        ] {
            assert!(observed.get(key).is_none(), "{key} must be removed");
        }
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
                format!("#!/bin/sh\nprintf '%s\\n' '{}'\nexit {exit}\n", result),
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
                assert_eq!(checked, Ok(()));
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
