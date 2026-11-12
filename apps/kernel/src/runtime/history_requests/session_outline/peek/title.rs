//! The words a pane row is titled with, for clients and agents that read titles without the
//! pane: "Ran ls -la", "Read README.md", "Thought Checking the docs", "Timed out after 30 min".
//! The pane's own wording lives in `@chariox/tool-display` and the web's `tool-call-title.ts`;
//! these follow it for the common calls and fall back to the tool's name.
use serde_json::{Map, Value};

const TITLE_CHARS: usize = 80;

/// A tool call's title from its merged snapshot (`tool`, `status`, `input`, ...).
pub(super) fn tool_title(snapshot: &Map<String, Value>, running: bool) -> String {
    let verb = |done: &str, active: &str| (if running { active } else { done }).to_string();
    let input = snapshot.get("input").and_then(Value::as_object);
    let raw_name = snapshot
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if raw_name == super::WAIT_TOOL {
        return wait_title(snapshot);
    }
    let name = compact_tool_name(raw_name);
    let field = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            input
                .and_then(|input| input.get(*key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(short)
        })
    };
    if let Some(command) = command(input) {
        let description = field(&["description"])
            .or_else(|| {
                snapshot
                    .get("description")
                    .and_then(Value::as_str)
                    .map(short)
            })
            .filter(|words| !words.is_empty() && !words.starts_with("cwd "));
        return description.unwrap_or_else(|| shell_title(&command, running));
    }
    if SHELL_TOOLS.contains(&name.as_str()) {
        return verb("Ran a command", "Running a command");
    }
    let path = field(&["file_path", "filePath", "path", "notebook_path", "file"]);
    let pattern = field(&["pattern", "query", "q", "regex"]);
    let object = |what: Option<String>, otherwise: &str| what.unwrap_or_else(|| otherwise.into());
    match name.as_str() {
        "read" | "readfile" | "view" | "charioxreadartifact" | "readartifact" => {
            format!("{} {}", verb("Read", "Reading"), object(path, "a file"))
        }
        "edit"
        | "multiedit"
        | "strreplace"
        | "strreplacebasededittool"
        | "notebookedit"
        | "applypatch"
        | "patch"
        | "charioxeditartifact"
        | "charioxapplypatch"
        | "charioxpatchartifact"
        | "editartifact"
        | "patchartifact" => {
            format!("{} {}", verb("Edited", "Editing"), object(path, "files"))
        }
        "write" | "writefile" | "create" | "charioxwriteartifact" | "writeartifact" => {
            format!("{} {}", verb("Wrote", "Writing"), object(path, "a file"))
        }
        "grep" | "search" | "rg" | "codebasesearch" | "searchfiles" => format!(
            "{} {}{}",
            verb("Searched", "Searching"),
            pattern.map_or("files".into(), |pattern| format!("for {pattern}")),
            path.map_or(String::new(), |path| format!(" in {path}")),
        ),
        "glob" | "find" | "findfiles" => format!(
            "{} files{}{}",
            verb("Searched", "Searching"),
            pattern.map_or(String::new(), |pattern| format!(" matching {pattern}")),
            path.map_or(String::new(), |path| format!(" in {path}")),
        ),
        "ls" | "list" | "listdir" | "listfiles" | "listdirectory" => {
            format!("{} {}", verb("Listed", "Listing"), object(path, "files"))
        }
        "websearch" | "webquery" => match pattern {
            Some(pattern) => format!("{} the web for {pattern}", verb("Searched", "Searching")),
            None => verb("Searched the web", "Searching the web"),
        },
        "webfetch" | "fetch" | "fetchurl" => format!(
            "{} {}",
            verb("Fetched", "Fetching"),
            object(field(&["url", "uri"]), "a page")
        ),
        "todowrite" | "updateplan" | "todo" => verb("Updated plan", "Updating plan"),
        "todoread" => verb("Read plan", "Reading plan"),
        "task" | "agent" | "spawnagent" => format!(
            "{} {}",
            verb("Delegated", "Delegating"),
            object(field(&["description", "prompt"]), "a task")
        ),
        _ => format!(
            "{} {}",
            verb("Called", "Calling"),
            called_name(snapshot, raw_name)
        ),
    }
}

/// The kernel's row for an agent's wait: what it waits for, then how long it waited and what
/// ended it. A wait its deadline ended says it timed out.
fn wait_title(snapshot: &Map<String, Value>) -> String {
    let reason = snapshot
        .get("input")
        .and_then(|input| input.get("reason"))
        .and_then(Value::as_str)
        .map(short)
        .filter(|reason| !reason.is_empty());
    let Some(finished) = snapshot.get("finished_at_ms").and_then(Value::as_u64) else {
        return match reason {
            Some(reason) if reason.to_ascii_lowercase().starts_with("wait") => reason,
            Some(reason) => format!("Waiting: {reason}"),
            None => "Waiting".into(),
        };
    };
    let waited = snapshot
        .get("started_at_ms")
        .and_then(Value::as_u64)
        .map(|started| format!(" {}", minutes_and_seconds(finished.saturating_sub(started))))
        .unwrap_or_default();
    match woken_by(snapshot) {
        Some("deadline_reached") => format!("Timed out after{waited}"),
        Some(kind) => format!("Waited{waited} · woken by {}", kind.replace('_', " ")),
        None => format!("Waited{waited}"),
    }
}

pub(super) fn woken_by(snapshot: &Map<String, Value>) -> Option<&str> {
    snapshot
        .get("woken_by")
        .and_then(|woken| woken.get("kind"))
        .and_then(Value::as_str)
}

fn minutes_and_seconds(ms: u64) -> String {
    let seconds = ms / 1_000;
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

/// A thought is titled by its Markdown heading or bold first line, as the pane titles it.
pub(super) fn thought_title(text: &str, running: bool) -> String {
    let word = if running { "Thinking" } else { "Thought" };
    let line = text.lines().map(str::trim).find(|line| !line.is_empty());
    let heading = line.and_then(|line| {
        let hashes = line.trim_start_matches('#');
        if hashes.len() < line.len() && hashes.starts_with(' ') {
            return Some(hashes.trim());
        }
        ["**", "__"].iter().find_map(|mark| {
            line.strip_prefix(mark)
                .and_then(|rest| rest.strip_suffix(mark))
                .filter(|inner| !inner.is_empty() && !inner.contains(mark))
        })
    });
    match heading {
        Some(heading) => format!("{word} {}", short(heading)),
        None => word.into(),
    }
}

/// A message's first line that says something.
pub(super) fn text_title(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(short)
        .unwrap_or_default()
}

fn command(input: Option<&Map<String, Value>>) -> Option<String> {
    let command = input?.get("command").or_else(|| input?.get("cmd"))?;
    let command = match command {
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(command) => command.trim().to_string(),
        _ => return None,
    };
    (!command.is_empty()).then_some(command)
}

/// Codex runs commands as `/bin/bash -lc '<script>'`: the title names the script's first line.
fn shell_title(command: &str, running: bool) -> String {
    let script = unwrap_shell(command);
    let first = script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(script);
    let more = if first != script.trim() { " …" } else { "" };
    let simple =
        !script.contains('\n') && !first.contains(['|', ';', '&', '<', '>', '`', '$', '(', ')']);
    let mut words = first.split_whitespace();
    let program = words.next().and_then(|word| word.rsplit('/').next());
    let operands = words
        .filter(|word| !word.starts_with('-'))
        .map(|word| word.trim_matches(['\'', '"']))
        .collect::<Vec<_>>();
    let (done, active) = match program {
        Some("cat" | "bat" | "less") if simple && !operands.is_empty() => {
            return format!(
                "{} {}",
                if running { "Reading" } else { "Read" },
                operands.join(", ")
            )
        }
        Some("ls" | "tree") if simple => {
            let what = if operands.is_empty() {
                "files".into()
            } else {
                operands.join(", ")
            };
            return format!("{} {what}", if running { "Listing" } else { "Listed" });
        }
        _ => ("Ran", "Running"),
    };
    format!(
        "{} {}{more}",
        if running { active } else { done },
        short(first)
    )
}

fn unwrap_shell(command: &str) -> &str {
    let mut words = command.trim().splitn(3, char::is_whitespace);
    let shell = words.next().and_then(|word| word.rsplit('/').next());
    let flag = words.next();
    match (shell, flag, words.next()) {
        (Some("bash" | "sh" | "zsh"), Some("-lc" | "-c"), Some(script)) => {
            let script = script.trim();
            ['\'', '"']
                .iter()
                .find_map(|quote| {
                    script
                        .strip_prefix(*quote)
                        .and_then(|rest| rest.strip_suffix(*quote))
                })
                .unwrap_or(script)
                .trim()
        }
        _ => command.trim(),
    }
}

/// "mcp__server__tool" reads "server.tool" (once, when the tool repeats the server's name);
/// Codex names an MCP call's server in `title`.
fn called_name(snapshot: &Map<String, Value>, raw: &str) -> String {
    let words = |value: &str| {
        value
            .split('_')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if let Some((server, tool)) = raw
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        let tool = tool.strip_prefix(&format!("{server}_")).unwrap_or(tool);
        return format!("{}.{}", words(server), words(tool));
    }
    let tool = if raw.is_empty() {
        "a tool".into()
    } else {
        words(raw)
    };
    match snapshot.get("title").and_then(Value::as_str).map(str::trim) {
        Some(server)
            if !raw.is_empty()
                && !server.is_empty()
                && server
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) =>
        {
            format!("{server}.{tool}")
        }
        _ => tool,
    }
}

const SHELL_TOOLS: &[&str] = &[
    "bash",
    "shell",
    "sh",
    "shellcommand",
    "execcommand",
    "runcommand",
    "runshellcommand",
    "localshell",
    "terminal",
    "commandexecution",
    "exec",
    "unifiedexec",
];

fn compact_tool_name(tool: &str) -> String {
    let compact = tool
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '_' | '-'))
        .collect::<String>()
        .to_lowercase();
    let compact = compact
        .strip_prefix("mcp")
        .filter(|rest| rest.starts_with("chariox"))
        .unwrap_or(&compact);
    let mut compact = compact;
    while compact.starts_with("charioxchariox") {
        compact = &compact["chariox".len()..];
    }
    compact.to_string()
}

fn short(value: &str) -> String {
    let line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > TITLE_CHARS {
        format!(
            "{}…",
            line.chars().take(TITLE_CHARS - 1).collect::<String>()
        )
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(value: Value, running: bool) -> String {
        tool_title(value.as_object().unwrap(), running)
    }

    #[test]
    fn tool_rows_are_titled_as_the_pane_titles_them() {
        // MP-08 / MP-10 / MP-11 PEEK-01/08: the pane's words, not "1 tool called".
        assert_eq!(
            tool(
                serde_json::json!({"tool":"bash","input":{"command":"ls -la src"}}),
                false
            ),
            "Listed src"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"exec_command","input":{"command":["/bin/bash","-lc","cargo test -p kernel"]}}),
                true
            ),
            "Running cargo test -p kernel"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"Bash","input":{"command":"npm test","description":"Run the tests"}}),
                false
            ),
            "Run the tests"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"Read","input":{"file_path":"README.md"}}),
                false
            ),
            "Read README.md"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"Grep","input":{"pattern":"fn main","path":"src"}}),
                false
            ),
            "Searched for fn main in src"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"mcp__chariox__chariox_send_agent_message","input":{}}),
                false
            ),
            "Called chariox.send agent message"
        );
        assert_eq!(
            tool(
                serde_json::json!({"tool":"lookup","title":"docs","input":{}}),
                true
            ),
            "Calling docs.lookup"
        );
    }

    #[test]
    fn a_wait_its_deadline_ended_says_it_timed_out() {
        // MP-08 / MP-10 / MP-11 PEEK-07.
        let running = serde_json::json!({"tool":"chariox.wait","input":{"reason":"the build"},"started_at_ms":0});
        assert_eq!(tool(running, true), "Waiting: the build");
        let timed_out = serde_json::json!({"tool":"chariox.wait","started_at_ms":0,"finished_at_ms":1_800_000,"woken_by":{"kind":"deadline_reached"}});
        assert_eq!(tool(timed_out, false), "Timed out after 30 min 0 s");
        let woken = serde_json::json!({"tool":"chariox.wait","started_at_ms":0,"finished_at_ms":5_000,"woken_by":{"kind":"source_completed"}});
        assert_eq!(tool(woken, false), "Waited 5 s · woken by source completed");
    }

    #[test]
    fn thoughts_are_titled_by_their_heading() {
        assert_eq!(
            thought_title("**Checking the docs**\nThe docs say", false),
            "Thought Checking the docs"
        );
        assert_eq!(
            thought_title("## Planning\nfirst", true),
            "Thinking Planning"
        );
        assert_eq!(thought_title("Plain prose", false), "Thought");
    }
}
