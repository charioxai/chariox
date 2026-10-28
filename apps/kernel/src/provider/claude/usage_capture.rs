use std::fs;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;

use super::mcp_config::ClaudeRuntimeFilesRoot;

pub(super) struct ClaudeUsageCapture {
    usage_file: PathBuf,
    handler_file: PathBuf,
}

impl ClaudeUsageCapture {
    pub(super) fn usage_file(&self) -> &Path {
        &self.usage_file
    }

    pub(super) fn command(&self) -> String {
        format!(
            "CHARIOX_CLAUDE_USAGE_FILE={} node {}",
            shell_quote_path(&self.usage_file),
            shell_quote_path(&self.handler_file),
        )
    }
}

fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

pub(super) fn materialize_claude_usage_capture(
    root: &ClaudeRuntimeFilesRoot,
) -> Result<ClaudeUsageCapture, DaemonError> {
    let usage_file = root.path().join("usage.json");
    let handler_file = root.path().join("usage-handler.mjs");
    fs::write(&usage_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude usage file",
        message: error.to_string(),
    })?;
    fs::write(&handler_file, CLAUDE_USAGE_HANDLER).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "prepare claude usage handler",
            message: error.to_string(),
        }
    })?;
    Ok(ClaudeUsageCapture {
        usage_file,
        handler_file,
    })
}

const CLAUDE_USAGE_HANDLER: &str = r#"#!/usr/bin/env node
import { renameSync, writeFileSync } from "node:fs"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const raw = Buffer.concat(chunks).toString("utf8").trim()
if (!raw) process.exit(0)

try {
  const input = JSON.parse(raw)
  if (!input.rate_limits && !input.context_window) process.exit(0)
  const target = process.env.CHARIOX_CLAUDE_USAGE_FILE
  if (!target) process.exit(0)
  const temporary = `${target}.${process.pid}.tmp`
  writeFileSync(temporary, JSON.stringify(input))
  renameSync(temporary, target)
} catch {}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provusage_claude_status_line_captures_context_without_rate_limits() {
        use std::io::Write;
        let worktree = crate::test_support::TestWorktree::new("claude-context-capture");
        let handler = worktree.path().join("capture.mjs");
        let output = worktree.path().join("usage.json");
        fs::write(&handler, CLAUDE_USAGE_HANDLER).unwrap();
        let mut child = crate::process_spawn::command("node")
            .arg(&handler)
            .env("CHARIOX_CLAUDE_USAGE_FILE", &output)
            .env("NODE_OPTIONS", "--max-old-space-size=4096")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let payload = serde_json::json!({ "context_window": { "context_window_size": 200000, "current_usage": { "input_tokens": 500, "cache_read_input_tokens": 1500 } } });
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert!(output.exists(), "the official status-line context must be captured even when the provider omits account limits");
        let captured: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(captured, payload);
    }

    #[test]
    fn status_line_paths_are_single_quoted_for_the_shell() {
        assert_eq!(
            shell_quote_path(Path::new("/tmp/a'b$(`unsafe`)")),
            "'/tmp/a'\\''b$(`unsafe`)'"
        );
    }
}
