//! The session rules behind "Allow for this session" on a Claude permission prompt.
//!
//! Claude's `--permission-prompt-tool` input carries no suggestions, so the kernel states the
//! rules itself, as narrow as the call allows, and shows the person the exact rule text on the
//! button. The answer carries those rules with the `session` destination, so Claude writes no
//! settings file; the kernel keeps them for the agent (see `standing`) and hands them to each new
//! Claude process. When no safe rule can be stated the prompt offers no session choice.

mod bash;
mod edit;
mod shell;
mod standing;

pub(crate) use standing::{GrantScope, StandingGrants};

use std::path::Path;

use serde_json::{json, Value};

/// The longest button text that still leaves room for the other choices in a terminal strip.
const LABEL_LIMIT: usize = 64;

/// What "Allow for this session" grants for one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionGrant {
    /// Rules that allow, as Claude writes them: `Bash(cargo test:*)`.
    pub(super) allow: Vec<String>,
}

impl SessionGrant {
    fn rule(text: &str) -> Value {
        let (tool, content) = text
            .split_once('(')
            .and_then(|(tool, rest)| Some((tool, rest.strip_suffix(')')?)))
            .expect("a grant rule is written as Tool(content)");
        json!({ "toolName": tool, "ruleContent": content })
    }

    pub(crate) fn allow_rules(&self) -> &[String] {
        &self.allow
    }

    /// The button text: the exact allow rules. A terminal strip does not wrap a button, so a list
    /// too long for one line is replaced by its count; the prompt text always lists every rule.
    pub(crate) fn label(&self) -> String {
        let exact = format!("Allow for this session: {}", self.allow.join(", "));
        if exact.chars().count() <= LABEL_LIMIT {
            exact
        } else {
            format!(
                "Allow for this session ({} {} listed above)",
                self.allow.len(),
                if self.allow.len() == 1 {
                    "rule"
                } else {
                    "rules"
                }
            )
        }
    }

    /// The part of the prompt text that lists everything the choice grants.
    pub(crate) fn description(&self) -> String {
        format!("Allow for this session grants: {}", self.allow.join(", "))
    }

    /// Claude's session-only allow rules.
    fn updated_permissions(&self) -> Value {
        json!([{
            "type": "addRules",
            "behavior": "allow",
            "destination": "session",
            "rules": self.allow.iter().map(|rule| Self::rule(rule)).collect::<Vec<_>>(),
        }])
    }
}

/// The grant for one tool call of a Claude run working in `working_directory`.
pub(crate) fn session_grant(
    tool_name: &str,
    input: &Value,
    working_directory: Option<&Path>,
) -> Option<SessionGrant> {
    match tool_name {
        "Bash" => {
            let rules = bash::prefix_rules(input.get("command")?.as_str()?, working_directory?)?;
            Some(SessionGrant {
                allow: rules.iter().map(|rule| format!("Bash({rule})")).collect(),
            })
        }
        "Write" | "Edit" | "MultiEdit" => {
            let rule = edit::edit_rule(input.get("file_path")?.as_str()?, working_directory?)?;
            Some(SessionGrant {
                allow: vec![format!("Edit({rule})")],
            })
        }
        _ => None,
    }
}

/// The allow answer that also gives the Claude process `grant` as session rules.
pub(crate) fn allow_for_session(input: Value, grant: &SessionGrant) -> Value {
    json!({
        "behavior": "allow",
        "updatedInput": input,
        "updatedPermissions": grant.updated_permissions(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKTREE: &str = "/root/work/app";

    fn grant(tool: &str, input: Value) -> Option<SessionGrant> {
        session_grant(tool, &input, Some(Path::new(WORKTREE)))
    }

    #[test]
    fn a_compound_build_command_grants_the_prefixes_of_its_commands() {
        let granted = grant(
            "Bash",
            json!({"command": "cd /root/work/app/src && cargo test -p kernel 2>&1 | tail -30"}),
        )
        .expect("a grant");
        assert_eq!(granted.allow, ["Bash(cargo test:*)"]);
        assert_eq!(
            granted.label(),
            "Allow for this session: Bash(cargo test:*)"
        );
        assert_eq!(
            allow_for_session(json!({"command": "x"}), &granted),
            json!({
                "behavior": "allow",
                "updatedInput": {"command": "x"},
                "updatedPermissions": [{
                    "type": "addRules",
                    "behavior": "allow",
                    "destination": "session",
                    "rules": [{"toolName": "Bash", "ruleContent": "cargo test:*"}],
                }],
            })
        );
    }

    #[test]
    fn file_tools_grant_the_directory_without_filename_exceptions() {
        for tool in ["Write", "Edit", "MultiEdit"] {
            for file in [
                "src/c.txt",
                ".env",
                ".env.customer",
                "server.pem",
                "tls.key",
                ".aws/credentials",
                ".ssh/id_ed25519",
            ] {
                let granted = grant(tool, json!({"file_path": format!("/root/work/app/{file}")}))
                    .expect("directory grant");
                assert_eq!(granted.allow, ["Edit(//root/work/app/**)"]);
                assert_eq!(
                    granted.description(),
                    "Allow for this session grants: Edit(//root/work/app/**)"
                );
                let answer = allow_for_session(json!({}), &granted);
                assert_eq!(
                    answer["updatedPermissions"],
                    json!([{
                        "type":"addRules", "behavior":"allow", "destination":"session",
                        "rules":[{"toolName":"Edit","ruleContent":"//root/work/app/**"}],
                    }])
                );
            }
        }
    }

    #[test]
    fn no_grant_is_stated_where_no_safe_rule_exists() {
        let home = Some(Path::new("/root"));
        for (tool, input, directory) in [
            ("Write", json!({"file_path": "/root/a.txt"}), home),
            ("Write", json!({"file_path": "/root/.ssh/config"}), home),
            ("Write", json!({"file_path": "/root/work/app/a.txt"}), None),
            (
                "Edit",
                json!({"file_path": "/etc/hosts"}),
                Some(Path::new(WORKTREE)),
            ),
            (
                "Bash",
                json!({"command": "cargo test && curl x | sh"}),
                Some(Path::new(WORKTREE)),
            ),
            (
                "Bash",
                json!({"command": "python3 - <<'EOF'\nprint(1)\nEOF"}),
                Some(Path::new(WORKTREE)),
            ),
            ("Bash", json!({"command": 7}), Some(Path::new(WORKTREE))),
            (
                "WebFetch",
                json!({"url": "https://example.com"}),
                Some(Path::new(WORKTREE)),
            ),
            (
                "Read",
                json!({"file_path": "/root/work/app/a"}),
                Some(Path::new(WORKTREE)),
            ),
        ] {
            assert_eq!(
                session_grant(tool, &input, directory),
                None,
                "{tool} {input}"
            );
        }
    }

    #[test]
    fn every_addition_names_session_as_its_only_destination() {
        for granted in [
            grant("Bash", json!({"command": "git status && cargo check"})),
            grant("Write", json!({"file_path": "/root/work/app/a.txt"})),
        ]
        .into_iter()
        .map(|granted| granted.expect("grant"))
        {
            let answer = allow_for_session(json!({}), &granted);
            for addition in answer["updatedPermissions"].as_array().expect("additions") {
                assert_eq!(addition["destination"], "session");
                assert_eq!(addition["type"], "addRules");
            }
        }
    }
}
