use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

use super::package::package_skill_directory;
use super::{ArrobaSkillMetadata, ArrobaSkillRegistry, parse_skill_metadata};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportSkip {
    pub name: String,
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportOutcome {
    pub imported: Vec<ArrobaSkillMetadata>,
    pub skipped: Vec<SkillImportSkip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSkillImportCandidate {
    pub provider: String,
    pub name: String,
    pub source: String,
    pub source_path: PathBuf,
    pub skill_md_path: PathBuf,
    pub source_modified_ms: u64,
    pub version_hash: String,
    pub metadata: ArrobaSkillMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderSkillImportDiscovery {
    pub candidates: Vec<ProviderSkillImportCandidate>,
    pub skipped: Vec<SkillImportSkip>,
}

pub fn import_codex_skills(
    registry: &ArrobaSkillRegistry,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<SkillImportOutcome, DaemonError> {
    import_skills_from_roots(registry, codex_skill_roots(workspace), requested_name)
}

pub fn import_opencode_skills(
    registry: &ArrobaSkillRegistry,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<SkillImportOutcome, DaemonError> {
    import_skills_from_roots(registry, opencode_skill_roots(workspace), requested_name)
}

pub fn import_claude_skills(
    registry: &ArrobaSkillRegistry,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<SkillImportOutcome, DaemonError> {
    import_skills_from_roots(registry, claude_skill_roots(workspace), requested_name)
}

pub fn discover_provider_skill_import_candidates(
    provider: &str,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<ProviderSkillImportDiscovery, DaemonError> {
    let Some(canonical_provider) = crate::provider::canonical_provider_family(provider) else {
        return Err(DaemonError::InvalidConfig {
            field: "provider",
            message: "only Codex, OpenCode, and Claude skill import are supported",
        });
    };
    let roots = match canonical_provider {
        "codex" => codex_skill_roots(workspace),
        "opencode" => opencode_skill_roots(workspace),
        "claude" => claude_skill_roots(workspace),
        _ => {
            return Err(DaemonError::InvalidConfig {
                field: "provider",
                message: "only Codex, OpenCode, and Claude skill import are supported",
            });
        }
    };
    discover_provider_skill_import_candidates_from_roots(canonical_provider, roots, requested_name)
}

fn discover_provider_skill_import_candidates_from_roots(
    provider: &str,
    roots: Vec<PathBuf>,
    requested_name: Option<&str>,
) -> Result<ProviderSkillImportDiscovery, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "skill name")?;
    }
    let mut discovery = ProviderSkillImportDiscovery::default();
    let mut found_requested = false;
    for root in roots {
        if !root.exists() {
            continue;
        }
        for skill_md in find_skill_markdown_files(&root)? {
            let source_dir = skill_md.parent().unwrap_or(&root).to_path_buf();
            let package = match package_skill_directory(&source_dir) {
                Ok(package) => package,
                Err(error) => {
                    discovery.skipped.push(SkillImportSkip {
                        name: source_dir
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        path: skill_md,
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            if requested_name.is_some_and(|requested| requested != package.metadata.name) {
                continue;
            }
            found_requested = true;
            discovery.candidates.push(ProviderSkillImportCandidate {
                provider: provider.to_string(),
                name: package.metadata.name.clone(),
                source: source_dir.display().to_string(),
                source_modified_ms: source_modified_ms(&source_dir),
                source_path: source_dir,
                skill_md_path: skill_md,
                version_hash: package.version_hash,
                metadata: package.metadata,
            });
        }
    }
    if let Some(name) = requested_name {
        if !found_requested {
            discovery.skipped.push(SkillImportSkip {
                name: name.to_string(),
                path: PathBuf::new(),
                reason: "not found in provider skill roots".to_string(),
            });
        }
    }
    Ok(discovery)
}

fn import_skills_from_roots(
    registry: &ArrobaSkillRegistry,
    roots: Vec<PathBuf>,
    requested_name: Option<&str>,
) -> Result<SkillImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "skill name")?;
    }
    let mut outcome = SkillImportOutcome::default();
    let mut found_requested = false;
    for root in roots {
        if !root.exists() {
            continue;
        }
        for skill_md in find_skill_markdown_files(&root)? {
            let source_dir = skill_md.parent().unwrap_or(&root).to_path_buf();
            let metadata = match parse_skill_metadata(&skill_md) {
                Ok(metadata) => metadata,
                Err(error) => {
                    outcome.skipped.push(SkillImportSkip {
                        name: source_dir
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        path: skill_md,
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            if requested_name.is_some_and(|requested| requested != metadata.name) {
                continue;
            }
            found_requested = true;
            if registry.get(&metadata.name)?.is_some() {
                outcome.skipped.push(SkillImportSkip {
                    name: metadata.name,
                    path: skill_md,
                    reason: "already installed in Arroba registry".to_string(),
                });
                continue;
            }
            match registry.install_from_path(&source_dir) {
                Ok((installed, _)) => outcome.imported.push(installed),
                Err(error) => outcome.skipped.push(SkillImportSkip {
                    name: metadata.name,
                    path: skill_md,
                    reason: error.to_string(),
                }),
            }
        }
    }
    if let Some(name) = requested_name {
        if !found_requested {
            outcome.skipped.push(SkillImportSkip {
                name: name.to_string(),
                path: PathBuf::new(),
                reason: "not found in provider skill roots".to_string(),
            });
        }
    }
    Ok(outcome)
}

fn codex_skill_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        workspace.join(".codex").join("skills"),
        workspace.join(".agents").join("skills"),
    ];
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        roots.push(PathBuf::from(codex_home).join("skills"));
    } else if let Some(home) = home_dir() {
        roots.push(home.join(".codex").join("skills"));
    }
    if let Some(home) = home_dir() {
        roots.push(home.join(".agents").join("skills"));
    }
    roots
}

fn opencode_skill_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        workspace.join(".opencode").join("skill"),
        workspace.join(".opencode").join("skills"),
        workspace.join(".agents").join("skills"),
    ];
    if let Some(home) = home_dir() {
        roots.push(home.join(".agents").join("skills"));
        roots.push(home.join(".config").join("opencode").join("skill"));
        roots.push(home.join(".config").join("opencode").join("skills"));
    }
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        let opencode = PathBuf::from(config_home).join("opencode");
        roots.push(opencode.join("skill"));
        roots.push(opencode.join("skills"));
    }
    for config_path in opencode_config_paths(workspace) {
        roots.extend(opencode_extra_skill_paths(&config_path, workspace));
    }
    roots
}

fn claude_skill_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = vec![workspace.join(".claude").join("skills")];
    if let Some(claude_home) = std::env::var_os("CLAUDE_HOME") {
        roots.push(PathBuf::from(claude_home).join("skills"));
    } else if let Some(home) = home_dir() {
        roots.push(home.join(".claude").join("skills"));
    }
    roots
}

fn find_skill_markdown_files(root: &Path) -> Result<Vec<PathBuf>, DaemonError> {
    fn visit(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), DaemonError> {
        if depth > 8 {
            return Ok(());
        }
        for entry in fs::read_dir(dir).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.import",
            message: format!("failed to read skill root `{}`: {error}", dir.display()),
        })? {
            let entry = entry.map_err(|error| DaemonError::LocalTransport {
                operation: "skill.import",
                message: format!("failed to read skill root entry: {error}"),
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "skill.import",
                    message: format!("failed to inspect `{}`: {error}", path.display()),
                })?;
            if file_type.is_file() && entry.file_name() == "SKILL.md" {
                out.push(path);
            } else if file_type.is_dir() {
                if entry.file_name() == ".system" {
                    continue;
                }
                visit(&path, depth + 1, out)?;
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    visit(root, 0, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn opencode_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(custom));
    }
    if let Some(config_dir) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        paths.extend(opencode_config_files_in_dir(Path::new(&config_dir)));
    }
    paths.extend([
        workspace.join("opencode.jsonc"),
        workspace.join("opencode.json"),
        workspace.join(".opencode").join("opencode.jsonc"),
        workspace.join(".opencode").join("opencode.json"),
    ]);
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.extend(opencode_config_files_in_dir(
            &PathBuf::from(config_home).join("opencode"),
        ));
    } else if let Some(home) = home_dir() {
        paths.extend(opencode_config_files_in_dir(
            &home.join(".config").join("opencode"),
        ));
    }
    paths
}

fn opencode_config_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    vec![
        dir.join("opencode.jsonc"),
        dir.join("opencode.json"),
        dir.join("config.json"),
    ]
}

fn opencode_extra_skill_paths(config_path: &Path, workspace: &Path) -> Vec<PathBuf> {
    let Ok(payload) = fs::read_to_string(config_path) else {
        return Vec::new();
    };
    let json_payload = remove_json_trailing_commas(&strip_jsonc_comments(&payload));
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_payload) else {
        return Vec::new();
    };
    let Some(paths) = parsed
        .get("skills")
        .and_then(|skills| skills.get("paths"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|path| expand_provider_path(path, workspace))
        .collect()
}

fn expand_provider_path(value: &str, workspace: &Path) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        output.push(ch);
    }
    output
}

fn remove_json_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        output.push(ch);
    }
    output
}

fn source_modified_ms(path: &Path) -> u64 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
