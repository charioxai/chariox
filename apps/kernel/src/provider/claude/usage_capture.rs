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
  if (!input.rate_limits) process.exit(0)
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
    fn status_line_paths_are_single_quoted_for_the_shell() {
        assert_eq!(
            shell_quote_path(Path::new("/tmp/a'b$(`unsafe`)")),
            "'/tmp/a'\\''b$(`unsafe`)'"
        );
    }
}
