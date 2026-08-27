use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::account_profile::ProviderAccountUsageSnapshot;
use crate::error::DaemonError;
use crate::provider::claude_runtime::new_claude_session_id;

use super::mcp_config::create_claude_runtime_files_root;
use super::usage_capture::materialize_claude_usage_capture;

// Claude Code can publish its first authenticated status-line rate-limit
// snapshot on the second refresh tick, about 15 seconds after a cold start.
const CLAUDE_USAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_USAGE_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CLAUDE_USAGE_MODAL_SETTLE_TIME: Duration = Duration::from_secs(10);
const CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES: usize = 4 * 1024;

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
    let profile_home_is_supported = linux_profile_home_is_supported();
    validate_claude_probe_environment(environment, profile_home_is_supported)?;
    let config_root = claude_config_root(
        environment,
        std::env::var_os("HOME").map(PathBuf::from),
        profile_home_is_supported,
    )
    .ok_or_else(|| probe_error("Claude config directory is unavailable".to_string()))?;
    super::ensure_claude_headless_onboarding_state_at(&config_root.join(".claude.json"))?;
    let root = create_claude_runtime_files_root()?;
    let capture = materialize_claude_usage_capture(&root)?;
    let settings_file = root.path().join("usage-probe-settings.json");
    fs::write(
        &settings_file,
        serde_json::to_vec(&serde_json::json!({
            "statusLine": {
                "type": "command",
                "command": capture.raw_command(),
            }
        }))
        .map_err(|error| probe_error(format!("failed to encode settings: {error}")))?,
    )
    .map_err(|error| probe_error(format!("failed to write settings: {error}")))?;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| probe_error(format!("failed to open PTY: {error}")))?;
    let mut command = CommandBuilder::new(executable);
    let session_id = new_claude_session_id();
    command.args([
        "--settings",
        settings_file.to_string_lossy().as_ref(),
        "--session-id",
        &session_id,
        "--no-chrome",
        // The private PTY has no terminal emulator attached. Screen-reader
        // mode prevents Claude's full-screen renderer from waiting on
        // terminal capability replies before starting the status line.
        "--ax-screen-reader",
    ]);
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
    command.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| probe_error(format!("failed to start Claude: {error}")))?;
    drop(pair.slave);
    let mut reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(probe_error(format!("failed to drain Claude PTY: {error}")));
        }
    };
    let mut writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(probe_error(format!(
                "failed to control Claude PTY: {error}"
            )));
        }
    };
    let (output_tx, output_rx) = mpsc::sync_channel(64);
    let reader_thread = thread::Builder::new()
        .name("chariox-claude-usage-probe-reader".to_string())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(size) = reader.read(&mut buffer) {
                if size == 0 {
                    break;
                }
                let _ = output_tx.try_send(buffer[..size].to_vec());
            }
        })
        .map_err(|error| {
            let _ = child.kill();
            let _ = child.wait();
            probe_error(format!("failed to start Claude PTY drain: {error}"))
        })?;

    let deadline = Instant::now() + timeout;
    let mut usage_command_sent_at = None;
    let mut usage_modal_closed = false;
    let mut terminal_output = Vec::new();
    let mut last_status_line_fields = Vec::new();
    let result = loop {
        drain_terminal_output(&output_rx, &mut terminal_output);
        if let Ok(contents) = fs::read_to_string(capture.usage_file()) {
            if !contents.trim().is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(mut usage) =
                        crate::provider::claude_status_line_usage_snapshot(&value)
                    {
                        usage.profile_id = account_profile.to_string();
                        break Ok(usage);
                    }
                    if let Some(object) = value.as_object() {
                        last_status_line_fields = object.keys().cloned().collect();
                    }
                    if usage_command_sent_at.is_none() {
                        if let Err(error) =
                            writer.write_all(b"/usage\r").and_then(|()| writer.flush())
                        {
                            break Err(probe_error(format!(
                                "failed to request Claude usage: {error}"
                            )));
                        }
                        usage_command_sent_at = Some(Instant::now());
                    }
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break Err(probe_error(format!(
                    "Claude exited before reporting usage ({status})"
                )));
            }
            Ok(None) => {}
            Err(error) => {
                break Err(probe_error(format!(
                    "failed to inspect Claude usage probe: {error}"
                )));
            }
        }
        if Instant::now() >= deadline {
            break Err(probe_error(format!(
                "Claude did not report usage within {} seconds",
                timeout.as_secs()
            )));
        }
        if !usage_modal_closed
            && usage_command_sent_at
                .is_some_and(|sent_at| sent_at.elapsed() >= CLAUDE_USAGE_MODAL_SETTLE_TIME)
        {
            // `/usage` is a provider-native account command and does not send
            // a model prompt. Closing its modal makes Claude publish the
            // refreshed values through the documented status-line payload.
            if let Err(error) = writer.write_all(b"\x1b").and_then(|()| writer.flush()) {
                break Err(probe_error(format!(
                    "failed to close Claude usage view: {error}"
                )));
            }
            usage_modal_closed = true;
        }
        thread::sleep(CLAUDE_USAGE_PROBE_POLL_INTERVAL);
    };

    let _ = child.kill();
    let _ = child.wait();
    drop(pair.master);
    let _ = reader_thread.join();
    drain_terminal_output(&output_rx, &mut terminal_output);
    let result = match (result, cleanup_probe_session(environment, &session_id)) {
        (Ok(usage), Ok(())) => Ok(usage),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(append_probe_error(
            error,
            format!("failed to clean the ephemeral Claude session: {cleanup_error}"),
        )),
    };
    result.map_err(|error| {
        let terminal = terminal_diagnostic(&terminal_output);
        let status_line = (!last_status_line_fields.is_empty())
            .then(|| format!("status-line fields: {}", last_status_line_fields.join(", ")));
        let diagnostic = match (terminal.is_empty(), status_line) {
            (true, None) => return error,
            (true, Some(status_line)) => status_line,
            (false, None) => format!("Claude terminal: {terminal}"),
            (false, Some(status_line)) => {
                format!("{status_line}; Claude terminal: {terminal}")
            }
        };
        append_probe_error(error, diagnostic)
    })
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

fn cleanup_probe_session(
    environment: &BTreeMap<String, String>,
    session_id: &str,
) -> Result<(), DaemonError> {
    let config_root = claude_config_root(
        environment,
        std::env::var_os("HOME").map(PathBuf::from),
        linux_profile_home_is_supported(),
    )
    .ok_or_else(|| probe_error("cannot locate Claude account storage".to_string()))?;
    let projects_root = config_root.join("projects");
    let metadata = match fs::symlink_metadata(&projects_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(probe_error(format!(
                "failed to inspect Claude projects storage: {error}"
            )))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(probe_error(format!(
            "Claude projects storage is not a regular directory: {}",
            projects_root.display()
        )));
    }
    let transcript_name = format!("{session_id}.jsonl");
    for entry in fs::read_dir(&projects_root)
        .map_err(|error| probe_error(format!("failed to inspect Claude projects: {error}")))?
    {
        let entry = entry
            .map_err(|error| probe_error(format!("failed to inspect Claude project: {error}")))?;
        let project_type = entry.file_type().map_err(|error| {
            probe_error(format!("failed to inspect Claude project type: {error}"))
        })?;
        if project_type.is_symlink() || !project_type.is_dir() {
            continue;
        }
        let transcript = entry.path().join(&transcript_name);
        let transcript_metadata = match fs::symlink_metadata(&transcript) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(probe_error(format!(
                    "failed to inspect Claude probe transcript {}: {error}",
                    transcript.display()
                )))
            }
        };
        if transcript_metadata.file_type().is_symlink() || !transcript_metadata.is_file() {
            return Err(probe_error(format!(
                "refusing to remove non-regular Claude probe transcript: {}",
                transcript.display()
            )));
        }
        fs::remove_file(&transcript).map_err(|error| {
            probe_error(format!(
                "failed to remove Claude probe transcript {}: {error}",
                transcript.display()
            ))
        })?;
    }
    Ok(())
}

fn claude_config_root(
    environment: &BTreeMap<String, String>,
    inherited_home: Option<PathBuf>,
    profile_home_is_supported: bool,
) -> Option<PathBuf> {
    environment
        .get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            profile_home_is_supported
                .then(|| {
                    environment
                        .get("HOME")
                        .map(|home| PathBuf::from(home).join(".claude"))
                })
                .flatten()
        })
        .or_else(|| inherited_home.map(|home| home.join(".claude")))
}

fn append_probe_error(error: DaemonError, diagnostic: String) -> DaemonError {
    match error {
        DaemonError::LocalTransport { operation, message } => DaemonError::LocalTransport {
            operation,
            message: format!("{message}; {diagnostic}"),
        },
        error => error,
    }
}

fn probe_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "refresh Claude usage",
        message,
    }
}

fn drain_terminal_output(receiver: &mpsc::Receiver<Vec<u8>>, output: &mut Vec<u8>) {
    for chunk in receiver.try_iter() {
        output.extend_from_slice(&chunk);
        if output.len() > CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES {
            output.drain(..output.len() - CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES);
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn probes_both_usage_windows_without_replacing_home() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
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
        let mut file = fs::File::create(&executable).expect("fake Claude executable");
        file.write_all(
            br#"#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
const settingsPath = process.argv[process.argv.indexOf("--settings") + 1]
const sessionId = process.argv[process.argv.indexOf("--session-id") + 1]
const settings = JSON.parse(readFileSync(settingsPath, "utf8"))
const onboarding = JSON.parse(readFileSync(join(process.env.CLAUDE_CONFIG_DIR, ".claude.json"), "utf8"))
if (onboarding.hasCompletedOnboarding !== true) throw new Error("Claude onboarding state is missing")
const projectDir = join(process.env.CLAUDE_CONFIG_DIR, "projects", "fixture")
const transcriptPath = join(projectDir, `${sessionId}.jsonl`)
mkdirSync(projectDir, { recursive: true })
writeFileSync(transcriptPath, "")
writeFileSync(process.env.CHARIOX_PROBE_TEST_ENV_FILE, JSON.stringify({
  home: process.env.HOME,
  claudeConfigDir: process.env.CLAUDE_CONFIG_DIR,
  anthropicApiKey: process.env.ANTHROPIC_API_KEY,
  anthropicAuthToken: process.env.ANTHROPIC_AUTH_TOKEN,
  anthropicBaseUrl: process.env.ANTHROPIC_BASE_URL,
  anthropicCustomHeaders: process.env.ANTHROPIC_CUSTOM_HEADERS,
  transcriptPath
}))
const emitStatusLine = (input) => spawnSync("/bin/sh", ["-c", settings.statusLine.command], {
  input: JSON.stringify(input)
})
emitStatusLine({
  session_id: sessionId,
  transcript_path: transcriptPath,
  model: { display_name: "Fixture" }
})
process.stdin.setRawMode?.(true)
process.stdin.on("data", (chunk) => {
  if (!chunk.toString().includes("/usage")) return
  emitStatusLine({ session_id: sessionId, transcript_path: transcriptPath, rate_limits: {
      five_hour: { used_percentage: 17, resets_at: 1800000000 },
      seven_day: { used_percentage: 41, resets_at: 1800100000 }
    }})
})
process.stdin.resume()
"#,
        )
        .expect("fake Claude source");
        drop(file);
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("fake Claude permissions");
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
            !Path::new(
                observed["transcriptPath"]
                    .as_str()
                    .expect("transcript path")
            )
            .exists(),
            "the probe must clean only its generated Claude transcript"
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_remove_a_symlinked_probe_transcript() {
        use std::os::unix::fs::symlink;

        let fixture = std::env::temp_dir().join(format!(
            "chariox-claude-usage-cleanup-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let project = fixture.join("projects/fixture");
        fs::create_dir_all(&project).expect("project directory");
        let target = fixture.join("preserve.jsonl");
        fs::write(&target, "preserve").expect("target transcript");
        let session_id = new_claude_session_id();
        symlink(&target, project.join(format!("{session_id}.jsonl"))).expect("transcript symlink");

        let error = cleanup_probe_session(
            &BTreeMap::from([(
                "CLAUDE_CONFIG_DIR".to_string(),
                fixture.display().to_string(),
            )]),
            &session_id,
        )
        .expect_err("symlink must be rejected");
        assert!(error.to_string().contains("refusing to remove non-regular"));
        assert_eq!(
            fs::read_to_string(&target).expect("preserved target"),
            "preserve"
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[test]
    fn terminal_diagnostics_strip_control_sequences_and_collapse_whitespace() {
        assert_eq!(
            terminal_diagnostic(b"\x1b[31mLogin expired\x1b[0m\r\n  run auth"),
            "Login expired run auth"
        );
    }

    #[test]
    fn rejects_account_environment_home_override() {
        let environment = BTreeMap::from([("HOME".to_string(), "/wrong/home".to_string())]);

        let error = validate_claude_probe_environment(&environment, false)
            .expect_err("profile HOME must be rejected");
        assert!(error.to_string().contains("supported only on Linux"));
        assert!(error.to_string().contains("refusing to override HOME"));
    }

    #[test]
    fn linux_profile_home_selects_the_matching_cleanup_root() {
        let environment = BTreeMap::from([("HOME".to_string(), "/profile/home".to_string())]);

        validate_claude_probe_environment(&environment, true)
            .expect("Linux profile HOME should be supported");
        assert_eq!(
            claude_config_root(&environment, Some(PathBuf::from("/inherited/home")), true),
            Some(PathBuf::from("/profile/home/.claude"))
        );
    }

    #[test]
    fn explicit_claude_config_dir_precedes_linux_profile_home() {
        let environment = BTreeMap::from([
            ("HOME".to_string(), "/profile/home".to_string()),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                "/profile/claude".to_string(),
            ),
        ]);

        assert_eq!(
            claude_config_root(&environment, Some(PathBuf::from("/inherited/home")), true),
            Some(PathBuf::from("/profile/claude"))
        );
    }
}
