//! The saved Environment of a Project: names and text only, never a secret value.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub const PROJECT_ENVIRONMENT_SCHEMA: u32 = 2;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_ENTRIES: usize = 256;

/// Where an MCP server or skill comes from: a file of the Project, which Git carries, or a registration a kernel holds
/// (`~/.chariox/mcps`, `~/.chariox/skills`), which only the Environment moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ExtensionSource {
    /// The project file (relative path) that declares it, for example `.mcp.json`.
    File {
        path: String,
    },
    Registration,
}

/// An MCP server or skill of the Project: its name and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionEntry {
    pub name: String,
    pub source: ExtensionSource,
}

impl ExtensionEntry {
    pub fn file(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: ExtensionSource::File { path: path.into() },
        }
    }

    pub fn registration(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: ExtensionSource::Registration,
        }
    }

    /// The declaring file of a project-file entry.
    pub fn file_path(&self) -> Option<&str> {
        match &self.source {
            ExtensionSource::File { path } => Some(path),
            ExtensionSource::Registration => None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentRecord {
    pub schema: u32,
    /// Optimistic concurrency token: Save must carry the revision it read.
    pub revision: u64,
    pub setup_script: String,
    /// `.env` text: `KEY=value` lines and `#` comments.
    pub variables: String,
    pub secrets: Vec<String>,
    pub files: Vec<String>,
    #[serde(default)]
    pub mcps: Vec<ExtensionEntry>,
    #[serde(default)]
    pub skills: Vec<ExtensionEntry>,
    pub changed_at_ms: u64,
}

/// MCP servers and skills are named like directories and JSON keys: letters, digits, `_`, `-` and `.`.
pub fn extension_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

pub fn environment_variable_name(value: &str) -> bool {
    let mut chars = value.bytes();
    matches!(chars.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && value.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Host and provider process controls are recreated by each kernel, never carried by a Project.
pub fn project_environment_protected_name(name: &str) -> bool {
    name.starts_with("CHARIOX_")
        || name.starts_with("DYLD_")
        || matches!(
            name,
            "HOME"
                | "PATH"
                | "USER"
                | "LOGNAME"
                | "SHELL"
                | "TMPDIR"
                | "PWD"
                | "OLDPWD"
                | "TERM"
                | "COLORTERM"
                | "CODEX_HOME"
                | "CLAUDE_CONFIG_DIR"
                | "OPENCODE_CONFIG_DIR"
                | "XDG_CONFIG_HOME"
                | "XDG_DATA_HOME"
                | "XDG_STATE_HOME"
                | "LD_PRELOAD"
                | "LD_AUDIT"
                | "NODE_OPTIONS"
                | "BASH_ENV"
                | "ENV"
        )
}

pub fn relative_project_path(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\0', '\n', '\r', '\\'])
        || Path::new(value)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || value.split('/').any(|part| part == ".git")
    {
        return Err("Files outside Git take a path inside the project folder");
    }
    Ok(())
}

/// One `KEY=value` line. Values may be wrapped in one pair of quotes, which is removed.
fn parse_assignment(line: &str) -> Result<(&str, String), &'static str> {
    let (name, value) = line
        .split_once('=')
        .ok_or("Variables take KEY=value lines")?;
    let name = name.trim();
    if !environment_variable_name(name) || project_environment_protected_name(name) {
        return Err(
            "Variables take names that start with a letter or underscore and are not reserved",
        );
    }
    let value = value.trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|quote| {
            value
                .strip_prefix(*quote)
                .and_then(|rest| rest.strip_suffix(*quote))
        })
        .unwrap_or(value);
    if unquoted.contains('\0') {
        return Err("Variables cannot contain NUL");
    }
    Ok((name, unquoted.to_owned()))
}

/// The assignments of a Variables text, in order; later duplicates win like a shell would.
pub fn parse_variables(text: &str) -> Result<Vec<(String, String)>, &'static str> {
    let mut assignments: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = parse_assignment(line)?;
        assignments.retain(|(existing, _)| existing != name);
        assignments.push((name.to_owned(), value));
    }
    Ok(assignments)
}

impl ProjectEnvironmentRecord {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != PROJECT_ENVIRONMENT_SCHEMA {
            return Err("unsupported Project environment schema");
        }
        if self.setup_script.len() > MAX_TEXT_BYTES
            || self.variables.len() > MAX_TEXT_BYTES
            || self.setup_script.contains('\0')
        {
            return Err("Setup and Variables are limited to 64 KiB");
        }
        let variables = parse_variables(&self.variables)?;
        if self.secrets.len() > MAX_ENTRIES || self.files.len() > MAX_ENTRIES {
            return Err("an Environment holds at most 256 Secrets and 256 Files");
        }
        let mut names = BTreeSet::new();
        for name in &self.secrets {
            if !environment_variable_name(name)
                || project_environment_protected_name(name)
                || !names.insert(name.as_str())
                || variables.iter().any(|(variable, _)| variable == name)
            {
                return Err("Secrets take unique names that are not Variables or reserved");
            }
        }
        let mut paths = BTreeSet::new();
        for path in &self.files {
            relative_project_path(path)?;
            if !paths.insert(path.as_str()) {
                return Err("Files outside Git list each path once");
            }
        }
        for entries in [&self.mcps, &self.skills] {
            if entries.len() > MAX_ENTRIES {
                return Err("an Environment holds at most 256 MCPs and 256 Skills");
            }
            let mut names = BTreeSet::new();
            for entry in entries {
                if let Some(path) = entry.file_path() {
                    relative_project_path(path)?;
                }
                if !extension_name(&entry.name) || !names.insert(entry.name.as_str()) {
                    return Err("MCPs and Skills take unique names of letters, digits, _, - and .");
                }
            }
        }
        Ok(())
    }

    /// Identifies the script a Setup run executed; a changed script makes the last run stale.
    pub fn setup_digest(&self) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(self.setup_script.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ProjectEnvironmentRecord {
        ProjectEnvironmentRecord {
            schema: PROJECT_ENVIRONMENT_SCHEMA,
            ..Default::default()
        }
    }

    #[test]
    fn variables_parse_comments_quotes_and_duplicates() {
        let parsed = parse_variables("# c\nA=1\n\nB=\"two words\"\nA=3\n").unwrap();
        assert_eq!(
            parsed,
            vec![("B".into(), "two words".into()), ("A".into(), "3".into())]
        );
        assert!(parse_variables("PATH=/x").is_err());
        assert!(parse_variables("1A=x").is_err());
        assert!(parse_variables("novalue").is_err());
    }

    #[test]
    fn a_secret_cannot_also_be_a_variable_or_repeat() {
        let mut value = record();
        value.variables = "TOKEN=x".into();
        value.secrets = vec!["TOKEN".into()];
        assert!(value.validate().is_err());
        value.variables.clear();
        value.secrets = vec!["TOKEN".into(), "TOKEN".into()];
        assert!(value.validate().is_err());
        value.secrets = vec!["TOKEN".into()];
        assert!(value.validate().is_ok());
    }

    #[test]
    fn files_stay_inside_the_project_folder() {
        for bad in ["/etc/passwd", "../x", "a/../b", ".git/config", "a\\b", ""] {
            assert!(relative_project_path(bad).is_err(), "{bad}");
        }
        assert!(relative_project_path("config/secrets.json").is_ok());
    }

    #[test]
    fn mcps_and_skills_take_unique_names_and_a_project_file() {
        let mut value = record();
        value.mcps = vec![ExtensionEntry::file("github", ".mcp.json")];
        value.skills = vec![ExtensionEntry::file(
            "dataviz",
            ".claude/skills/dataviz/SKILL.md",
        )];
        assert!(value.validate().is_ok());
        value.mcps.push(value.mcps[0].clone());
        assert!(value.validate().is_err(), "a name is listed once");
        value.mcps = vec![ExtensionEntry::file("git hub", ".mcp.json")];
        assert!(value.validate().is_err());
        value.mcps = vec![ExtensionEntry::file("github", "../.mcp.json")];
        assert!(
            value.validate().is_err(),
            "the source stays inside the project"
        );
        value.mcps = vec![ExtensionEntry::registration("notion")];
        assert!(value.validate().is_ok(), "a registration has no file");
    }

    #[test]
    fn the_digest_follows_the_script_only() {
        let mut a = record();
        a.setup_script = "make".into();
        let mut b = a.clone();
        b.variables = "X=1".into();
        assert_eq!(a.setup_digest(), b.setup_digest());
        b.setup_script = "make all".into();
        assert_ne!(a.setup_digest(), b.setup_digest());
    }
}
