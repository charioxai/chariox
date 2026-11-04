//! What the Project's own files declare: MCP servers, skills and the environment variables they and
//! `.env.example` refer to. Read by the kernel, with no model, for Draft and Check; names only, never a value.
use super::{
    environment_variable_name, extension_name, project_environment_protected_name, ExtensionEntry,
};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 256 * 1024;
const ENV_EXAMPLES: &[&str] = &[".env.example", ".env.sample", ".env.template"];
const SKILL_ROOTS: &[&str] = &[".claude/skills", ".agents/skills", ".opencode/skills"];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Detected {
    pub mcps: Vec<ExtensionEntry>,
    pub skills: Vec<ExtensionEntry>,
    /// Variable names the files refer to, with the file that does.
    pub variables: Vec<(String, String)>,
}

fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Names after `${` (up to `}` or `:`) and after `{env:` (up to `}`).
fn referenced_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (open, ends) in [("${", &['}', ':'][..]), ("{env:", &['}'][..])] {
        let mut rest = text;
        while let Some(at) = rest.find(open) {
            rest = &rest[at + open.len()..];
            let end = rest.find(ends).unwrap_or(rest.len());
            let name = &rest[..end];
            if environment_variable_name(name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

fn push_mcps(found: &mut Vec<ExtensionEntry>, names: impl Iterator<Item = String>, source: &str) {
    for name in names {
        if extension_name(&name) && !found.iter().any(|entry| entry.name == name) {
            found.push(ExtensionEntry {
                name,
                source: source.to_owned(),
            });
        }
    }
}

fn json_keys(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_object())
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// The MCP server names and the variable names one project file declares; empty for a file that is absent or unreadable.
pub(crate) fn mcps_in_file(checkout: &Path, source: &str) -> (Vec<String>, Vec<String>) {
    let Some(text) = read_small(&checkout.join(source)) else {
        return (Vec::new(), Vec::new());
    };
    let names = match source {
        ".mcp.json" => serde_json::from_str::<serde_json::Value>(&text)
            .map(|json| json_keys(json.get("mcpServers")))
            .unwrap_or_default(),
        "opencode.json" => serde_json::from_str::<serde_json::Value>(&text)
            .map(|json| match json.get("mcp") {
                Some(mcp)
                    if mcp
                        .get("servers")
                        .is_some_and(|servers| servers.is_object()) =>
                {
                    json_keys(mcp.get("servers"))
                }
                other => json_keys(other),
            })
            .unwrap_or_default(),
        ".codex/config.toml" => text
            .parse::<toml::Table>()
            .ok()
            .and_then(|table| match table.get("mcp_servers") {
                Some(toml::Value::Table(servers)) => Some(servers.keys().cloned().collect()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    (names, referenced_names(&text))
}

const MCP_FILES: &[&str] = &[".mcp.json", ".codex/config.toml", "opencode.json"];

fn skill_dirs(checkout: &Path) -> Vec<ExtensionEntry> {
    let mut found: Vec<ExtensionEntry> = Vec::new();
    for root in SKILL_ROOTS {
        let Ok(entries) = std::fs::read_dir(checkout.join(root)) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let source = format!("{root}/{name}/SKILL.md");
            if extension_name(&name)
                && checkout.join(&source).is_file()
                && !found.iter().any(|entry| entry.name == name)
            {
                found.push(ExtensionEntry { name, source });
            }
        }
    }
    found
}

pub(crate) fn detect(checkout: &Path) -> Detected {
    let mut detected = Detected::default();
    for source in MCP_FILES {
        let (names, variables) = mcps_in_file(checkout, source);
        push_mcps(&mut detected.mcps, names.into_iter(), source);
        for name in variables {
            if !project_environment_protected_name(&name)
                && !detected.variables.iter().any(|(known, _)| *known == name)
            {
                detected.variables.push((name, (*source).to_owned()));
            }
        }
    }
    detected.skills = skill_dirs(checkout);
    for example in ENV_EXAMPLES {
        let Some(text) = read_small(&checkout.join(example)) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim().trim_start_matches("export ").trim_start();
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if environment_variable_name(name)
                && !project_environment_protected_name(name)
                && !detected.variables.iter().any(|(known, _)| known == name)
            {
                detected
                    .variables
                    .push((name.to_owned(), (*example).to_owned()));
            }
        }
    }
    detected
}

/// The names one project file declares now, whichever kind it is: an MCP file, or a skill's SKILL.md.
pub(crate) fn declared_in(checkout: &Path, source: &str) -> Vec<String> {
    if MCP_FILES.contains(&source) {
        return mcps_in_file(checkout, source).0;
    }
    if source.ends_with("/SKILL.md") && checkout.join(source).is_file() {
        let name = source
            .trim_end_matches("/SKILL.md")
            .rsplit('/')
            .next()
            .unwrap_or_default();
        return vec![name.to_owned()];
    }
    Vec::new()
}

/// Names of the Project's MCPs and skills as text for an agent's prompt.
pub(crate) fn facts_text(detected: &Detected) -> String {
    let list = |entries: &[ExtensionEntry]| {
        entries
            .iter()
            .map(|entry| format!("{} ({})", entry.name, entry.source))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let variables = detected
        .variables
        .iter()
        .map(|(name, source)| format!("{name} ({source})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "MCP servers declared in project files: {}\nSkills declared in project files: {}\nEnvironment variables the project files refer to: {}",
        list(&detected.mcps),
        list(&detected.skills),
        variables
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chariox-env-detect-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn project_files_declare_mcps_skills_and_the_variables_they_refer_to() {
        let dir = scratch();
        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers":{"github":{"command":"gh-mcp","env":{"GITHUB_TOKEN":"${GITHUB_PAT}"}},"notion":{"url":"https://x"}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        std::fs::write(
            dir.join(".codex/config.toml"),
            "[mcp_servers.linear]\ncommand = \"x\"\n[other]\na = 1\n",
        )
        .unwrap();
        std::fs::write(dir.join("opencode.json"), r#"{"mcp":{"servers":{"sentry":{"command":["x"],"environment":{"T":"{env:SENTRY_TOKEN}"}}}}}"#).unwrap();
        std::fs::create_dir_all(dir.join(".claude/skills/dataviz")).unwrap();
        std::fs::write(
            dir.join(".claude/skills/dataviz/SKILL.md"),
            "---\nname: dataviz\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join(".agents/skills/empty")).unwrap();
        std::fs::write(
            dir.join(".env.example"),
            "STRIPE_SECRET_KEY=sk_example\nexport NOTION_TOKEN=\n# nothing=here\nPATH=/x\n",
        )
        .unwrap();
        let detected = detect(&dir);
        let names = |entries: &[ExtensionEntry]| {
            entries
                .iter()
                .map(|e| format!("{}@{}", e.name, e.source))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&detected.mcps),
            vec![
                "github@.mcp.json",
                "notion@.mcp.json",
                "linear@.codex/config.toml",
                "sentry@opencode.json"
            ]
        );
        assert_eq!(
            names(&detected.skills),
            vec!["dataviz@.claude/skills/dataviz/SKILL.md"],
            "a folder without SKILL.md is no skill"
        );
        let variables: Vec<&str> = detected
            .variables
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            variables,
            vec![
                "GITHUB_PAT",
                "SENTRY_TOKEN",
                "STRIPE_SECRET_KEY",
                "NOTION_TOKEN"
            ],
            "reserved names are left out"
        );
        assert!(
            !facts_text(&detected).contains("sk_example"),
            "a value is never in the facts"
        );
        assert_eq!(declared_in(&dir, ".mcp.json"), vec!["github", "notion"]);
        assert_eq!(
            declared_in(&dir, ".claude/skills/dataviz/SKILL.md"),
            vec!["dataviz"]
        );
        std::fs::remove_file(dir.join(".mcp.json")).unwrap();
        assert!(
            declared_in(&dir, ".mcp.json").is_empty(),
            "a file that is gone declares nothing"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
