//! The session rule behind "Allow for session" on a Claude permission prompt.
//!
//! Claude's `--permission-prompt-tool` input carries no suggestions, so the
//! kernel states the narrowest rule itself. Only shapes Claude was observed to
//! honour for the rest of its run are listed; for anything else no rule can be
//! stated and the prompt offers no session choice.

use serde_json::{json, Value};

/// Characters Claude matches literally in a rule. Quotes, redirects, parentheses
/// and `*` are interpreted or escaped by its rule parser, so a command holding
/// one would get a rule that never matches.
fn is_literal(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            ' ' | '_' | '.' | ',' | '/' | ':' | '=' | '@' | '%' | '+' | '-'
        )
}

/// A shell command Claude can match exactly: literal characters, joined by `&&`.
fn exact_bash_command(command: &str) -> Option<&str> {
    let parts_are_literal = command
        .split("&&")
        .all(|part| !part.trim().is_empty() && part.chars().all(is_literal));
    (command == command.trim() && parts_are_literal).then_some(command)
}

/// An absolute path Claude can match exactly in an `Edit` rule (`//` + path).
fn exact_edit_path(path: &str) -> Option<&str> {
    let absolute = path.starts_with('/')
        && path.len() > 1
        && path.chars().all(|ch| is_literal(ch) && ch != ' ')
        && path.split('/').all(|part| part != "..");
    absolute.then_some(path)
}

/// The rule for one tool call, as Claude's `updatedPermissions` rule object.
pub(super) fn session_allow_rule(tool_name: &str, input: &Value) -> Option<Value> {
    match tool_name {
        "Bash" => {
            let command = exact_bash_command(input.get("command")?.as_str()?)?;
            Some(json!({ "toolName": "Bash", "ruleContent": command }))
        }
        // Claude folds Write, Edit and MultiEdit into the `Edit` rule family; a
        // `Write(path)` rule does not match.
        "Write" | "Edit" | "MultiEdit" => {
            let path = exact_edit_path(input.get("file_path")?.as_str()?)?;
            Some(json!({ "toolName": "Edit", "ruleContent": format!("/{path}") }))
        }
        _ => None,
    }
}

/// The allow answer that also remembers `rule` for this Claude run only.
pub(super) fn allow_for_session(input: Value, rule: Value) -> Value {
    json!({
        "behavior": "allow",
        "updatedInput": input,
        "updatedPermissions": [{
            "type": "addRules",
            "behavior": "allow",
            "destination": "session",
            "rules": [rule],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_gets_its_exact_command_as_the_rule() {
        assert_eq!(
            session_allow_rule(
                "Bash",
                &json!({"command": "touch b1.txt", "description": "x"})
            ),
            Some(json!({"toolName": "Bash", "ruleContent": "touch b1.txt"}))
        );
        assert_eq!(
            session_allow_rule("Bash", &json!({"command": "cd a && make test"})),
            Some(json!({"toolName": "Bash", "ruleContent": "cd a && make test"}))
        );
    }

    #[test]
    fn bash_commands_claude_cannot_match_literally_get_no_rule() {
        for command in [
            "echo \"a b\" > f",
            "echo hi > f",
            "echo $(id)",
            "git commit -m 'x'",
            "rm *.log",
            "a & b",
            "a && ",
            " leading",
            "trailing ",
            "",
            "line\nbreak",
        ] {
            assert_eq!(
                session_allow_rule("Bash", &json!({ "command": command })),
                None,
                "{command:?}"
            );
        }
        assert_eq!(session_allow_rule("Bash", &json!({})), None);
        assert_eq!(session_allow_rule("Bash", &json!({"command": 7})), None);
    }

    #[test]
    fn file_tools_get_an_edit_rule_with_the_double_slash_path() {
        for tool in ["Write", "Edit", "MultiEdit"] {
            assert_eq!(
                session_allow_rule(tool, &json!({"file_path": "/work/app/c.txt"})),
                Some(json!({"toolName": "Edit", "ruleContent": "//work/app/c.txt"})),
                "{tool}"
            );
        }
    }

    #[test]
    fn file_paths_that_are_not_exact_get_no_rule() {
        for path in [
            "c.txt",
            "/",
            "/work/../etc/passwd",
            "/work/*.txt",
            "/a b/c",
            "/a/[x]",
            "",
        ] {
            assert_eq!(
                session_allow_rule("Write", &json!({ "file_path": path })),
                None,
                "{path:?}"
            );
        }
    }

    #[test]
    fn other_tools_get_no_rule() {
        for tool in ["WebFetch", "Task", "NotebookEdit", "mcp__x__y", "Read"] {
            assert_eq!(
                session_allow_rule(tool, &json!({"command": "ls", "file_path": "/a"})),
                None
            );
        }
    }

    #[test]
    fn the_session_answer_names_session_as_the_only_destination() {
        let answer = allow_for_session(
            json!({"command": "touch a"}),
            json!({"toolName": "Bash", "ruleContent": "touch a"}),
        );
        assert_eq!(
            answer,
            json!({
                "behavior": "allow",
                "updatedInput": {"command": "touch a"},
                "updatedPermissions": [{
                    "type": "addRules",
                    "behavior": "allow",
                    "destination": "session",
                    "rules": [{"toolName": "Bash", "ruleContent": "touch a"}],
                }],
            })
        );
    }
}
