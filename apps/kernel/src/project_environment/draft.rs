//! Draft with agent: the kernel reads the Project's manifest files and lists the files Git ignores,
//! the agent turns that into the Environment fields, and the kernel checks the answer like any Save.
//! The agent gets text and names only: never the contents of an ignored file, never a Secret value.
use super::ProjectEnvironmentRecord;
use std::path::Path;

/// Files whose content tells how a Project is set up; the first of each kind is sent, shortened.
const MANIFESTS: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "package.json",
    "pnpm-workspace.yaml",
    "mise.toml",
    ".mise.toml",
    ".tool-versions",
    ".nvmrc",
    "Makefile",
    "justfile",
    "Cargo.toml",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "Gemfile",
    "docker-compose.yml",
    ".env.example",
    ".env.sample",
    ".mcp.json",
];
const MANIFEST_BYTES: usize = 6 * 1024;
const TOTAL_BYTES: usize = 48 * 1024;
const LISTED_NAMES: usize = 120;
/// Folders that hold generated or downloaded files, never a Project's own private inputs.
const SKIPPED_FOLDERS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    ".cache",
    "__pycache__",
    ".git",
    ".turbo",
    "coverage",
];

fn skipped(path: &str) -> bool {
    path.split('/')
        .any(|part| SKIPPED_FOLDERS.contains(&part.trim_end_matches('/')))
}

/// Names of files Git ignores in `checkout`; empty for a folder that is not a Git checkout.
fn ignored_files(checkout: &Path) -> Vec<String> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| !path.ends_with('/') && !skipped(path))
        .take(LISTED_NAMES)
        .map(str::to_owned)
        .collect()
}

fn top_level_names(checkout: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(checkout) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            (!skipped(&name)).then(|| if folder { format!("{name}/") } else { name })
        })
        .collect();
    names.sort();
    names.truncate(LISTED_NAMES);
    names
}

fn manifest_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MANIFEST_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// What the Project contains as text: its entries, the names of ignored files and its manifest files.
pub(crate) fn draft_facts(checkout: &Path) -> String {
    let mut facts = String::new();
    let mut used = 0;
    for name in MANIFESTS {
        let Some(text) = manifest_text(&checkout.join(name)) else {
            continue;
        };
        if used + text.len() > TOTAL_BYTES {
            break;
        }
        used += text.len();
        facts.push_str(&format!("--- {name} ---\n{text}\n"));
    }
    let ignored = ignored_files(checkout);
    format!(
        "Project folder entries:\n{}\n\nFiles that Git ignores (names only, contents are private):\n{}\n\nManifest files:\n{}",
        top_level_names(checkout).join("\n"),
        ignored.join("\n"),
        facts,
    )
}

/// The prompt the agent answers: the task and the answer's shape first, then what the Project contains.
pub(crate) fn draft_prompt(checkout: &Path) -> String {
    format!(
        "Write the development Environment of the Project below, from its metadata. Answer with one JSON object and nothing else, in this shape:\n\
{{\"setup\": \"bash script\", \"variables\": \"KEY=value lines\", \"secrets\": [\"NAME\"], \"files\": [\"path\"]}}\n\
- setup: the shortest idempotent bash script that installs what the Project needs after a fresh clone: toolchain, dependencies and the generation or build steps its README, AGENTS.md or manifests ask for (for example `pnpm install`).\n\
- variables: non-secret settings with their usual development values, one KEY=value per line.\n\
- secrets: names of environment variables that hold credentials or tokens, taken from .env.example and the manifests. Names only.\n\
- files: paths from the ignored list that a developer must copy to a new checkout for the Project to work (local config, env files). Only paths from that list.\n\
Fill every field the metadata supports; use an empty value only for a field it says nothing about.\n\n\
=== Project metadata ===\n{}",
        draft_facts(checkout),
    )
}

/// The record the agent's answer describes; the first `{...}` object of the text.
pub(crate) fn parse_draft(output: &str) -> Result<ProjectEnvironmentRecord, String> {
    let start = output
        .find('{')
        .ok_or("The agent did not answer with an Environment")?;
    let end = output
        .rfind('}')
        .ok_or("The agent did not answer with an Environment")?;
    let value: serde_json::Value = serde_json::from_str(&output[start..=end])
        .map_err(|_| "The agent's answer is not valid JSON".to_owned())?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let names = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut secrets = names("secrets");
    secrets.dedup();
    let variables = text("variables");
    // A name the agent put in both places is a Secret: its value stays out of the record.
    let variable_names: Vec<String> = super::parse_variables(&variables)
        .map_err(|reason| reason.to_owned())?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    secrets.retain(|name| !variable_names.contains(name));
    let record = ProjectEnvironmentRecord {
        schema: super::PROJECT_ENVIRONMENT_SCHEMA,
        setup_script: text("setup"),
        variables,
        secrets,
        files: names("files"),
        ..Default::default()
    };
    record.validate().map_err(str::to_owned)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chariox-env-draft-{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_prompt_carries_manifests_and_names_of_ignored_files_but_not_their_contents() {
        let dir = scratch();
        std::fs::write(dir.join("package.json"), "{\"name\":\"demo\"}").unwrap();
        std::fs::write(dir.join(".env.example"), "NOTION_TOKEN=\n").unwrap();
        std::fs::write(dir.join(".gitignore"), ".env\nnode_modules/\n").unwrap();
        std::fs::write(dir.join(".env"), "DATABASE_URL=synthetic-private-value\n").unwrap();
        std::fs::create_dir(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules/x.js"), "x").unwrap();
        for args in [vec!["init", "-q"], vec!["add", "-A"]] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(&args)
                .status()
                .unwrap()
                .success());
        }
        let prompt = draft_prompt(&dir);
        assert!(prompt.contains("--- package.json ---") && prompt.contains("NOTION_TOKEN="));
        assert!(prompt.contains("\n.env\n"), "{prompt}");
        assert!(!prompt.contains("synthetic-private-value"));
        assert!(!prompt.contains("x.js"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_answer_becomes_a_valid_record_and_a_bad_one_is_refused() {
        let record = parse_draft(
            "Here you go:\n```json\n{\"setup\":\"pnpm install\",\"variables\":\"LOG=debug\",\"secrets\":[\"NOTION_TOKEN\",\"LOG\"],\"files\":[\".env\"]}\n```",
        )
        .unwrap();
        assert_eq!(record.setup_script, "pnpm install");
        assert_eq!(record.secrets, vec!["NOTION_TOKEN".to_string()]);
        assert_eq!(record.files, vec![".env".to_string()]);
        assert!(parse_draft("no json").is_err());
        assert!(parse_draft("{\"files\":[\"../escape\"]}").is_err());
        assert!(parse_draft("{\"variables\":\"PATH=/x\"}").is_err());
    }
}
