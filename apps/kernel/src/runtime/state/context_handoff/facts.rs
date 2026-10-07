//! Kernel-computed facts about an agent's work, read from its history rather
//! than from any model: the files it modified and read, the commands it ran
//! with their outcome, and the worktree it worked in.

use serde_json::Value;

use crate::history::{HistoryEvent, HistoryEventKind};

use super::builder::{excerpt, single_line, truncate_bytes};

const MAX_FACTS_BYTES: usize = 8_000;
const MAX_COMMANDS: usize = 25;
const MAX_PATHS: usize = 60;
const COMMAND_BYTES: usize = 240;

/// One provider tool call in the Codex or Claude transcript shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolCall {
    pub(super) id: Option<String>,
    tool: String,
    status: String,
    command: Option<String>,
    modified: Vec<String>,
    read: Option<String>,
    exit_code: Option<i64>,
    output: String,
}

impl ToolCall {
    pub(super) fn parse(content: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(content).ok()?;
        let text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        let tool = text(&value, "tool")?;
        let input = value.get("input").cloned().unwrap_or(Value::Null);
        let path = text(&input, "file_path")
            .or_else(|| text(&input, "notebook_path"))
            .or_else(|| text(&input, "path"));
        let raw = text(&value, "raw").unwrap_or_default();
        let (mut modified, mut read) = (Vec::new(), None);
        match tool.to_ascii_lowercase().as_str() {
            "edit" | "write" | "multiedit" | "notebookedit" => modified.extend(path),
            "read" => read = path,
            // Codex reports a patch's files in its raw change list.
            "apply_patch" => {
                if let Ok(Value::Array(changes)) = serde_json::from_str::<Value>(&raw) {
                    for change in &changes {
                        modified.extend(text(change, "path"));
                        modified
                            .extend(change.get("kind").and_then(|kind| text(kind, "move_path")));
                    }
                }
            }
            _ => {}
        }
        Some(Self {
            id: text(&value, "id"),
            status: text(&value, "status").unwrap_or_default(),
            command: text(&input, "command"),
            modified,
            read,
            exit_code: raw
                .lines()
                .find_map(|line| line.strip_prefix("exit_code:")?.trim().parse().ok()),
            output: text(&value, "error")
                .or_else(|| text(&value, "output"))
                .unwrap_or_default(),
            tool,
        })
    }

    fn outcome(&self) -> String {
        match self.exit_code {
            Some(code) => format!("exit {code}"),
            None => self.status.clone(),
        }
    }

    /// The call as one transcript line: what ran, how it ended, and an
    /// excerpt of its output.
    pub(super) fn transcript_line(&self, max_output_bytes: usize) -> String {
        let action = self
            .command
            .clone()
            .or_else(|| self.read.clone())
            .unwrap_or_else(|| self.modified.join(", "));
        let mut line = format!(
            "{} {} -> {}",
            self.tool,
            truncate_bytes(&single_line(&action), COMMAND_BYTES * 2),
            self.outcome()
        );
        let output = excerpt(self.output.trim(), max_output_bytes);
        if !output.is_empty() {
            line.push_str(": ");
            line.push_str(&output);
        }
        line
    }
}

/// What the kernel saw an agent do, plus the session state a new provider
/// session needs: its open interactions and queued prompts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct KernelFacts {
    modified: Vec<String>,
    read: Vec<String>,
    commands: Vec<String>,
    command_count: usize,
    worktree: Option<String>,
    branch: Option<String>,
    pub(super) open_interactions: Vec<String>,
    pub(super) queued_prompts: Vec<String>,
}

impl KernelFacts {
    pub(super) fn from_events(events: &[HistoryEvent]) -> Self {
        let mut facts = Self::default();
        // A call's updates share its id; the last one holds its outcome.
        let mut calls: Vec<ToolCall> = Vec::new();
        for event in events {
            if let Some(path) = &event.worktree_path {
                facts.worktree = Some(path.clone());
            }
            if let Some(branch) = event.metadata.get("branch").and_then(Value::as_str) {
                facts.branch = Some(branch.to_string());
            }
            match event.kind {
                HistoryEventKind::ProviderTool => {
                    let Some(call) = event.content.as_deref().and_then(ToolCall::parse) else {
                        continue;
                    };
                    match calls
                        .iter_mut()
                        .find(|known| call.id.is_some() && known.id == call.id)
                    {
                        Some(known) => *known = call,
                        None => calls.push(call),
                    }
                }
                HistoryEventKind::GitCommitDetected => {
                    if let Some(Value::Array(paths)) = event.metadata.get("changed_paths") {
                        for path in paths.iter().filter_map(Value::as_str) {
                            push_unique(&mut facts.modified, path);
                        }
                    }
                }
                _ => {}
            }
        }
        for call in &calls {
            if call.status != "error" {
                for path in &call.modified {
                    push_unique(&mut facts.modified, path);
                }
            }
            if let Some(path) = &call.read {
                push_unique(&mut facts.read, path);
            }
        }
        let commands = calls
            .iter()
            .filter(|call| call.command.is_some())
            .collect::<Vec<_>>();
        facts.command_count = commands.len();
        facts.commands = commands
            .iter()
            .skip(commands.len().saturating_sub(MAX_COMMANDS))
            .map(|call| {
                format!(
                    "- `{}` -> {}",
                    truncate_bytes(
                        &single_line(call.command.as_deref().unwrap_or_default()),
                        COMMAND_BYTES
                    ),
                    call.outcome()
                )
            })
            .collect();
        facts
    }

    /// The facts as packet lines, in at most `max_bytes`.
    pub(super) fn render(&self, max_bytes: usize) -> String {
        let mut lines = Vec::new();
        if let Some(worktree) = &self.worktree {
            let branch = self
                .branch
                .as_deref()
                .map(|branch| format!(" (branch {branch})"))
                .unwrap_or_default();
            lines.push(format!("Worktree: {worktree}{branch}"));
        }
        for (label, paths) in [
            ("Files modified", &self.modified),
            ("Files read", &self.read),
        ] {
            if !paths.is_empty() {
                let shown = &paths[paths.len().saturating_sub(MAX_PATHS)..];
                lines.push(format!("{label} ({}): {}", paths.len(), shown.join(", ")));
            }
        }
        if !self.commands.is_empty() {
            lines.push(format!(
                "Commands run (last {} of {}):",
                self.commands.len(),
                self.command_count
            ));
            lines.extend(self.commands.iter().cloned());
        }
        for (label, items) in [
            (
                "Open interactions awaiting the user",
                &self.open_interactions,
            ),
            (
                "Prompts queued after the current request",
                &self.queued_prompts,
            ),
        ] {
            if !items.is_empty() {
                lines.push(format!("{label}:"));
                lines.extend(
                    items
                        .iter()
                        .map(|item| format!("- {}", truncate_bytes(&single_line(item), 500))),
                );
            }
        }
        truncate_bytes(&lines.join("\n"), max_bytes.min(MAX_FACTS_BYTES))
    }
}

fn push_unique(paths: &mut Vec<String>, path: &str) {
    paths.retain(|known| known != path);
    paths.push(path.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;

    fn tool(sequence: u64, payload: serde_json::Value) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session",
                "run-1",
                Some("agent"),
                TerminalOutputKind::ProviderTool,
                None,
                &payload.to_string(),
            ),
            HistoryEventTurnContext::default(),
        )
    }

    #[test]
    fn facts_come_from_codex_and_claude_tool_calls_and_commits() {
        let mut commit = tool(5, serde_json::json!({}));
        commit.kind = HistoryEventKind::GitCommitDetected;
        commit.worktree_path = Some("/work/app".to_string());
        commit
            .metadata
            .insert("branch".into(), serde_json::json!("feature/x"));
        commit.metadata.insert(
            "changed_paths".into(),
            serde_json::json!(["src/lib.rs", "README.md"]),
        );
        let events = vec![
            tool(
                1,
                serde_json::json!({"id":"exec-1","tool":"bash","status":"running","input":{"command":"cargo test"}}),
            ),
            tool(
                2,
                serde_json::json!({"id":"exec-1","tool":"bash","status":"completed","input":{"command":"cargo test"},"output":"ok","raw":"exit_code: 101\nduration_ms: 9"}),
            ),
            tool(
                3,
                serde_json::json!({"id":"exec-2","tool":"apply_patch","status":"completed","raw":r#"[{"diff":"@@","kind":{"type":"update","move_path":null},"path":"/work/app/src/main.rs"}]"#}),
            ),
            tool(
                4,
                serde_json::json!({"id":"toolu_1","tool":"Read","status":"completed","input":{"file_path":"/work/app/Cargo.toml"},"output":"[package]"}),
            ),
            tool(
                6,
                serde_json::json!({"id":"toolu_2","tool":"Edit","status":"error","input":{"file_path":"/work/app/src/lib.rs"},"error":"no match"}),
            ),
            commit,
        ];

        let rendered = KernelFacts::from_events(&events).render(MAX_FACTS_BYTES);

        assert!(
            rendered.contains("Worktree: /work/app (branch feature/x)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Files modified (3): src/lib.rs, README.md, /work/app/src/main.rs"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Files read (1): /work/app/Cargo.toml"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Commands run (last 1 of 1):\n- `cargo test` -> exit 101"),
            "{rendered}"
        );
    }

    #[test]
    fn a_tool_line_keeps_the_action_and_outcome_and_bounds_the_output() {
        let call = ToolCall::parse(
            &serde_json::json!({"tool":"Bash","status":"error","input":{"command":"ls /missing"},"error":"x".repeat(5_000)}).to_string(),
        )
        .unwrap();
        let line = call.transcript_line(300);
        assert!(line.starts_with("Bash ls /missing -> error: x"), "{line}");
        assert!(line.len() < 400, "{}", line.len());
    }
}
