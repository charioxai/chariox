//! MP-08: Bounded deterministic name/locator index; source contents never reach discovery.
use super::resolver::{environment_error, read_environment_file};
use super::*;
use crate::error::DaemonError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct ProjectEnvironmentIndex {
    pub evidence: ProjectEnvironmentEvidence,
    pub references: Vec<ProjectEnvironmentEntry>,
    pub private_files: Vec<ProjectPrivateFileCandidate>,
}

pub fn index_project_environment(
    workspaces: &BTreeMap<String, PathBuf>,
    environment_names: &BTreeMap<String, BTreeSet<String>>,
) -> Result<ProjectEnvironmentIndex, DaemonError> {
    let mut paths = BTreeMap::new();
    let mut references = Vec::new();
    let mut private_files = Vec::new();
    for (workspace, root) in workspaces {
        let mut files = Vec::new();
        walk(root, root, &mut files)?;
        files.sort();
        let ignored = ignored_files(root)?;
        for path in &files {
            if ignored.contains(path) {
                private_files.push(ProjectPrivateFileCandidate {
                    workspace_id: workspace.clone(),
                    path: path.clone(),
                    bytes: std::fs::symlink_metadata(root.join(path))
                        .map_err(|_| environment_error("private file inventory unavailable"))?
                        .len(),
                    secret_looking: secret_looking_project_path(path),
                });
            }
        }
        let mut uses: BTreeMap<String, Vec<ProjectEnvironmentUse>> = BTreeMap::new();
        let mut config_uses: BTreeMap<String, Vec<ProjectEnvironmentUse>> = BTreeMap::new();
        let mut evidence_paths = Vec::new();
        let mut locators = BTreeMap::new();
        // Precedence mirrors dotenv development usage: .env.local before .env.
        for path in [".env.local", ".env", ".envrc"] {
            if root.join(path).symlink_metadata().is_ok() {
                let contents = read_environment_file(root, path)?;
                for line in contents.lines() {
                    let assignment = line.trim().strip_prefix("export ").unwrap_or(line.trim());
                    if let Some((name, _)) = assignment.split_once('=') {
                        let name = name.trim();
                        if environment_variable_name(name) {
                            locators.entry(name.to_string()).or_insert_with(|| {
                                ProjectEnvironmentLocator::EnvFile {
                                    path: path.into(),
                                    key: name.into(),
                                }
                            });
                        }
                    }
                }
            }
        }
        for path in files {
            let name = Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if name == ".envrc"
                || (name.starts_with(".env")
                    && ![".example", ".sample", ".template"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix)))
            {
                continue;
            }
            // No code or example contents are included in the utility prompt. Even a
            // source file containing a literal secret contributes names/lines only.
            let contents = match read_environment_file(root, &path) {
                Ok(contents) => contents,
                Err(_) => continue, // binaries/large files are not a reference source
            };
            let secret_contents = contains_secret_configuration(&contents);
            let private_secret = private_files
                .iter_mut()
                .find(|candidate| candidate.workspace_id == *workspace && candidate.path == path)
                .map(|candidate| {
                    candidate.secret_looking |= secret_contents;
                    candidate.secret_looking
                })
                .unwrap_or(false);
            if !private_secret && !secret_looking_project_path(&path) {
                evidence_paths.push(path.clone());
            }
            for (number, line) in contents.lines().enumerate() {
                if line.contains("readFile")
                    || line.contains("open(")
                    || line.contains("open (")
                    || line.contains("File.read")
                {
                    for referenced in quoted_paths(line) {
                        if relative_environment_path(&referenced).is_ok()
                            && !referenced.starts_with(".env")
                            && root.join(&referenced).symlink_metadata().is_ok()
                        {
                            let sites = config_uses.entry(referenced).or_default();
                            let usage = ProjectEnvironmentUse {
                                path: path.clone(),
                                line: (number + 1) as u32,
                            };
                            if !sites.contains(&usage) {
                                sites.push(usage);
                            }
                            if sites.len() > 128 {
                                return Err(environment_error(
                                    "configuration reference use bounds exceeded",
                                ));
                            }
                        }
                    }
                }
                for name in referenced_names(line) {
                    if project_environment_protected_name(&name) {
                        continue;
                    }
                    let sites = uses.entry(name).or_default();
                    let usage = ProjectEnvironmentUse {
                        path: path.clone(),
                        line: (number + 1) as u32,
                    };
                    if !sites.contains(&usage) {
                        sites.push(usage);
                    }
                    if sites.len() > 128 {
                        return Err(environment_error(
                            "environment reference use bounds exceeded",
                        ));
                    }
                }
            }
        }
        for (name, uses) in config_uses {
            references.push(ProjectEnvironmentEntry {
                name: name.clone(),
                workspace_id: workspace.clone(),
                kind: ProjectEnvironmentEntryKind::ConfigFile,
                classification: ProjectEnvironmentClassification::Secret,
                excluded: false,
                uses,
                locator: ProjectEnvironmentLocator::ConfigFile { path: name },
                status: ProjectEnvironmentEntryStatus::Missing,
            });
        }
        for (name, uses) in uses {
            let locator = locators.get(&name).cloned().unwrap_or_else(|| {
                if environment_names
                    .get(workspace)
                    .is_some_and(|names| names.contains(&name))
                {
                    ProjectEnvironmentLocator::WorkspaceEnvironment { name: name.clone() }
                } else {
                    ProjectEnvironmentLocator::Missing
                }
            });
            references.push(ProjectEnvironmentEntry {
                name,
                workspace_id: workspace.clone(),
                kind: ProjectEnvironmentEntryKind::Variable,
                classification: ProjectEnvironmentClassification::Secret,
                excluded: false,
                uses,
                locator,
                status: ProjectEnvironmentEntryStatus::Missing,
            });
        }
        paths.insert(workspace.clone(), evidence_paths);
    }
    if references.len() > 512 {
        return Err(environment_error("environment reference bounds exceeded"));
    }
    let mut evidence = ProjectEnvironmentEvidence::capture(workspaces, &paths)?;
    for file in &private_files {
        evidence
            .private_inventory
            .entry(file.workspace_id.clone())
            .or_default()
            // MP-08 / MP-10 / MP-11: Value-only edits in sealed configuration
            // refresh bindings, without reclassifying unchanged metadata.
            .insert(
                file.path.clone(),
                if file.secret_looking { 0 } else { file.bytes },
            );
    }
    Ok(ProjectEnvironmentIndex {
        evidence,
        references,
        private_files,
    })
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<(), DaemonError> {
    if directory
        .components()
        .count()
        .saturating_sub(root.components().count())
        > 32
    {
        return Err(environment_error("environment index depth exceeded"));
    }
    for item in std::fs::read_dir(directory)
        .map_err(|_| environment_error("environment index directory unavailable"))?
    {
        let item = item.map_err(|_| environment_error("environment index entry unavailable"))?;
        let name = item.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| environment_error("environment index path must be UTF-8"))?;
        if directory == root.join(".chariox") && name != "project.json" {
            continue;
        }
        if directory == root && name == ".chariox" {
            if item
                .file_type()
                .map_err(|_| environment_error("environment index type unavailable"))?
                .is_dir()
            {
                walk(root, &item.path(), files)?;
            }
            continue;
        }
        if [
            ".git",
            ".chariox",
            "node_modules",
            "target",
            "dist",
            "build",
            ".venv",
            "vendor",
            ".next",
            "__pycache__",
        ]
        .contains(&name)
        {
            continue;
        }
        let kind = item
            .file_type()
            .map_err(|_| environment_error("environment index type unavailable"))?;
        if kind.is_dir() {
            walk(root, &item.path(), files)?;
        } else if kind.is_file() {
            let path = item.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .ok_or_else(|| environment_error("environment index path must be UTF-8"))?
                .to_owned();
            relative_environment_path(&relative).map_err(environment_error)?;
            files.push(relative);
            if files.len() > 20_000 {
                return Err(environment_error("environment index file bounds exceeded"));
            }
        }
    }
    Ok(())
}

pub(super) fn referenced_names(line: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for prefix in [
        "process.env.",
        "import.meta.env.",
        "env::var(\"",
        "env::var_os(\"",
        "env!(\"",
        "option_env!(\"",
        "getenv(\"",
        "getenv('",
        "getenv(\"",
        "environ[\"",
        "environ['",
        "ENV[\"",
        "ENV['",
        "env[\"",
        "env['",
        "System.getenv(\"",
        "process.env[\"",
        "process.env['",
        "${",
        "$",
    ] {
        let mut rest = line;
        while let Some(position) = rest.find(prefix) {
            rest = &rest[position + prefix.len()..];
            let length = rest
                .bytes()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
                .count();
            let name = &rest[..length];
            if environment_variable_name(name) && name.bytes().any(|c| c.is_ascii_uppercase()) {
                names.insert(name.into());
            }
        }
    }
    names
}

fn ignored_files(root: &Path) -> Result<BTreeSet<String>, DaemonError> {
    // Filename-only Git plumbing. Values and contents never cross the provider boundary.
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(root)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
            "--",
        ])
        .output()
        .map_err(|_| environment_error("private file inventory could not run"))?;
    if !output.status.success() {
        return Ok(BTreeSet::new());
    } // ordinary directory Workspace
    if output.stdout.len() > 8 * 1024 * 1024 {
        return Err(environment_error("private file inventory exceeds bounds"));
    }
    let names: Vec<_> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| std::str::from_utf8(s).map(str::to_string))
        .collect::<Result<_, _>>()
        .map_err(|_| environment_error("private file names must be UTF-8"))?;
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    Ok(files
        .into_iter()
        .filter(|file| {
            names
                .iter()
                .any(|name| name == file || (name.ends_with('/') && file.starts_with(name)))
        })
        .collect())
}

pub fn secret_looking_project_path(path: &str) -> bool {
    path.split('/').any(|part| {
        part.starts_with(".env")
            || part.ends_with(".pem")
            || part.ends_with(".key")
            || part.ends_with(".tfvars")
            || part.starts_with("id_rsa")
            || part.starts_with("id_ed25519")
            || part.to_ascii_lowercase().contains("credential")
            || part == "secrets"
            || part == ".ssh"
    })
}

// MP-08 / MP-10: Literal path/key tokens only, never file values or source contents.
fn quoted_paths(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut characters = line.chars();
    while let Some(quote) = characters.next() {
        if !matches!(quote, '\'' | '"') {
            continue;
        }
        let mut literal = String::new();
        let mut escaped = false;
        for character in characters.by_ref() {
            if character == quote {
                if !escaped && literal.len() <= 1024 {
                    result.push(literal);
                }
                break;
            }
            if character == '\\' || character == '$' || character == '`' {
                escaped = true;
            }
            literal.push(character);
            if literal.len() > 1024 {
                escaped = true;
                literal.clear();
            }
        }
    }
    result
}
pub(crate) fn contains_secret_configuration(text: &str) -> bool {
    text.lines().any(|line| {
        let key = line
            .trim()
            .split_once(':')
            .map(|(key, _)| key)
            .or_else(|| line.trim().split_once('=').map(|(key, _)| key));
        key.is_some_and(|key| {
            let key = key
                .trim()
                .trim_matches(['\'', '"', '{', ' '])
                .to_ascii_lowercase();
            key.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && [
                    "password",
                    "secret",
                    "token",
                    "private_key",
                    "api_key",
                    "apikey",
                    "credential",
                ]
                .iter()
                .any(|name| key.contains(name))
        })
    })
}

// Imported left-behind files stay adjustable even though they are absent locally.
// This is receipt metadata, never a source poll or an ongoing synchronization.
pub(crate) fn retain_imported_private_candidates(
    index: &mut ProjectEnvironmentIndex,
    previous: Option<&StoredProjectEnvironment>,
) {
    let Some(previous) = previous.filter(|state| state.source.is_some()) else {
        return;
    };
    for file in &previous.manifest.private_files {
        if index.private_files.iter().any(|candidate| {
            candidate.workspace_id == file.workspace_id && candidate.path == file.path
        }) {
            continue;
        }
        let bytes = previous
            .evidence
            .private_inventory
            .get(&file.workspace_id)
            .and_then(|paths| paths.get(&file.path))
            .copied()
            .unwrap_or(0);
        index.private_files.push(ProjectPrivateFileCandidate {
            workspace_id: file.workspace_id.clone(),
            path: file.path.clone(),
            bytes,
            secret_looking: file.secret_looking,
        });
        index
            .evidence
            .private_inventory
            .entry(file.workspace_id.clone())
            .or_default()
            .insert(file.path.clone(), bytes);
    }
}
