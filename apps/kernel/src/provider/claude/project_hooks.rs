//! MP-08 / MP-10 / MP-11: the Claude hooks a folder ships in its own project settings.
//!
//! Claude Code keeps project hooks off until the folder is trusted, so these
//! are exactly the commands that trusting the folder switches on.
use std::fs;
use std::io::Read;
use std::path::Path;

const PROJECT_SETTINGS_FILES: [&str; 2] = [".claude/settings.json", ".claude/settings.local.json"];
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectHook {
    pub(crate) event: String,
    /// The command line, or the hook type for a hook that is not a command.
    pub(crate) action: String,
}

/// Hooks Claude would run in `workspace` once it is trusted. A settings file
/// Claude cannot read or parse is ignored by Claude too, so it adds none.
pub(crate) fn project_hooks(workspace: &Path) -> Vec<ProjectHook> {
    PROJECT_SETTINGS_FILES
        .iter()
        .filter_map(|relative| read_settings(&workspace.join(relative)))
        .flat_map(|settings| hooks_in(&settings))
        .collect()
}

fn read_settings(path: &Path) -> Option<serde_json::Value> {
    let file = fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn hooks_in(settings: &serde_json::Value) -> Vec<ProjectHook> {
    if settings.get("disableAllHooks").and_then(|v| v.as_bool()) == Some(true) {
        return Vec::new();
    }
    let Some(events) = settings.get("hooks").and_then(|hooks| hooks.as_object()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (event, matchers) in events {
        let Some(matchers) = matchers.as_array() else {
            continue;
        };
        for hook in matchers
            .iter()
            .filter_map(|matcher| matcher.get("hooks")?.as_array())
            .flatten()
        {
            let action = hook
                .get("command")
                .and_then(|command| command.as_str())
                .map(str::to_owned)
                .or_else(|| {
                    hook.get("type")
                        .and_then(|kind| kind.as_str())
                        .map(|kind| format!("{kind} hook"))
                });
            if let Some(action) = action {
                found.push(ProjectHook {
                    event: event.clone(),
                    action,
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chariox-project-hooks-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    #[test]
    fn lists_command_hooks_from_both_project_settings_files() {
        let dir = folder("both");
        fs::write(
            dir.join(".claude/settings.json"),
            br#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./check.sh"}]}]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/settings.local.json"),
            br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"make setup"}]}]}}"#,
        )
        .unwrap();
        let hooks = project_hooks(&dir);
        assert_eq!(
            hooks,
            vec![
                ProjectHook {
                    event: "PreToolUse".into(),
                    action: "./check.sh".into()
                },
                ProjectHook {
                    event: "SessionStart".into(),
                    action: "make setup".into()
                },
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_without_hooks_or_with_hooks_disabled_add_none() {
        let dir = folder("none");
        fs::write(
            dir.join(".claude/settings.json"),
            br#"{"permissions":{"allow":["Bash(ls)"]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/settings.local.json"),
            br#"{"disableAllHooks":true,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"x"}]}]}}"#,
        )
        .unwrap();
        assert!(project_hooks(&dir).is_empty());
        assert!(project_hooks(&dir.join("missing")).is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
