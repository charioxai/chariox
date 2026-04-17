use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaSkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrobaSkillRegistry {
    roots: Vec<PathBuf>,
}

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

impl ArrobaSkillRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".arroba").join("skills")
    }

    pub fn user_root() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".arroba").join("skills"))
    }

    pub fn install_from_path(
        &self,
        source: &Path,
    ) -> Result<(ArrobaSkillMetadata, PathBuf), DaemonError> {
        if !source.is_dir() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.install",
                message: format!("skill source `{}` must be a directory", source.display()),
            });
        }
        let source_skill_md = source.join("SKILL.md");
        if !source_skill_md.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.install",
                message: format!("skill source `{}` must contain SKILL.md", source.display()),
            });
        }
        let metadata = parse_skill_metadata(&source_skill_md)?;
        let root = self.primary_root()?;
        fs::create_dir_all(root).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.install",
            message: format!(
                "failed to create skill registry `{}`: {error}",
                root.display()
            ),
        })?;
        let destination = root.join(&metadata.name);
        if destination.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.install",
                message: format!(
                    "skill `{}` already exists at `{}`",
                    metadata.name,
                    destination.display()
                ),
            });
        }
        copy_directory(source, &destination)?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn list(&self) -> Result<Vec<ArrobaSkillMetadata>, DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for skill_dir in immediate_child_dirs(root)? {
                let skill_md = skill_dir.join("SKILL.md");
                if !skill_md.exists() {
                    continue;
                }
                let metadata = parse_skill_metadata(&skill_md)?;
                entries.entry(metadata.name.clone()).or_insert(metadata);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaSkillMetadata>, DaemonError> {
        validate_registry_name(name, "skill name")?;
        for root in &self.roots {
            let skill_md = root.join(name).join("SKILL.md");
            if skill_md.exists() {
                return parse_skill_metadata(&skill_md).map(Some);
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, DaemonError> {
        self.roots
            .first()
            .ok_or_else(|| DaemonError::InvalidConfig {
                field: "skill registry roots",
                message: "must include at least one root",
            })
    }
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

pub fn parse_skill_metadata(skill_md: &Path) -> Result<ArrobaSkillMetadata, DaemonError> {
    let body = fs::read_to_string(skill_md).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.read",
        message: format!("failed to read skill `{}`: {error}", skill_md.display()),
    })?;
    let frontmatter = extract_frontmatter(&body).ok_or_else(|| DaemonError::LocalTransport {
        operation: "skill.read",
        message: format!("`{}` must start with YAML frontmatter", skill_md.display()),
    })?;
    let values = parse_simple_yaml_map(frontmatter);
    let fallback_name = skill_md
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();
    let name = values.get("name").cloned().unwrap_or(fallback_name);
    validate_registry_name(&name, "skill name")?;
    let description = values.get("description").cloned().unwrap_or_default();
    if description.trim().is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "skill.read",
            message: format!("skill `{name}` must define a non-empty description"),
        });
    }
    Ok(ArrobaSkillMetadata {
        name,
        description,
        short_description: values
            .get("short-description")
            .or_else(|| values.get("short_description"))
            .cloned(),
        path: skill_md.to_path_buf(),
    })
}

fn extract_frontmatter(body: &str) -> Option<&str> {
    let rest = body.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn parse_simple_yaml_map(frontmatter: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        values.insert(key.trim().to_string(), value);
    }
    values
}

fn immediate_child_dirs(root: &Path) -> Result<Vec<PathBuf>, DaemonError> {
    let entries = fs::read_dir(root).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.list",
        message: format!(
            "failed to read skill registry `{}`: {error}",
            root.display()
        ),
    })?;
    let mut dirs = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.list",
                message: format!("failed to read skill registry entry: {error}"),
            })?
            .path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(destination).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.install",
        message: format!(
            "failed to create skill destination `{}`: {error}",
            destination.display()
        ),
    })?;
    for entry in fs::read_dir(source).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.install",
        message: format!(
            "failed to read skill source `{}`: {error}",
            source.display()
        ),
    })? {
        let entry = entry.map_err(|error| DaemonError::LocalTransport {
            operation: "skill.install",
            message: format!("failed to read skill source entry: {error}"),
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.install",
                message: format!(
                    "failed to inspect skill source `{}`: {error}",
                    source_path.display()
                ),
            })?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "skill.install",
                    message: format!(
                        "failed to copy skill file `{}` to `{}`: {error}",
                        source_path.display(),
                        destination_path.display()
                    ),
                }
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-skill-registry-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn parses_codex_style_skill_metadata() {
        let root = temp_root("parse");
        let skill_dir = root.join("browser-qa");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: browser-qa\ndescription: Browser QA workflow\nshort-description: QA\n---\nUse the browser.\n",
        )
        .unwrap();

        let registry = ArrobaSkillRegistry::new(vec![root.clone()]);
        let skills = registry.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "browser-qa");
        assert_eq!(skills[0].description, "Browser QA workflow");
        assert_eq!(skills[0].short_description.as_deref(), Some("QA"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn install_copies_skill_directory_to_primary_root() {
        let source_root = temp_root("install-source");
        let registry_root = temp_root("install-registry");
        let skill_dir = source_root.join("browser-qa");
        fs::create_dir_all(skill_dir.join("assets")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: browser-qa\ndescription: Browser QA workflow\n---\nUse the browser.\n",
        )
        .unwrap();
        fs::write(skill_dir.join("assets").join("prompt.txt"), "qa checklist").unwrap();

        let registry = ArrobaSkillRegistry::new(vec![registry_root.clone()]);
        let (metadata, destination) = registry.install_from_path(&skill_dir).unwrap();

        assert_eq!(metadata.name, "browser-qa");
        assert_eq!(destination, registry_root.join("browser-qa"));
        assert_eq!(
            fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
            "qa checklist"
        );
        assert_eq!(registry.get("browser-qa").unwrap(), Some(metadata));

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(registry_root);
    }

    #[test]
    fn imports_codex_and_opencode_skill_roots() {
        let workspace = temp_root("import-workspace");
        let registry_root = temp_root("import-registry");
        let codex_skill = workspace.join(".codex").join("skills").join("codex-qa");
        let opencode_skill = workspace
            .join(".opencode")
            .join("skills")
            .join("opencode-qa");
        fs::create_dir_all(codex_skill.join("assets")).unwrap();
        fs::create_dir_all(&opencode_skill).unwrap();
        fs::write(
            codex_skill.join("SKILL.md"),
            "---\nname: codex-qa\ndescription: Codex QA\n---\nUse Codex QA.\n",
        )
        .unwrap();
        fs::write(codex_skill.join("assets").join("checklist.txt"), "check").unwrap();
        fs::write(
            opencode_skill.join("SKILL.md"),
            "---\nname: opencode-qa\ndescription: OpenCode QA\n---\nUse OpenCode QA.\n",
        )
        .unwrap();

        let registry = ArrobaSkillRegistry::new(vec![registry_root.clone()]);
        let codex = import_codex_skills(&registry, &workspace, Some("codex-qa")).unwrap();
        assert_eq!(codex.imported.len(), 1);
        assert_eq!(codex.imported[0].name, "codex-qa");
        assert_eq!(
            fs::read_to_string(
                registry_root
                    .join("codex-qa")
                    .join("assets")
                    .join("checklist.txt")
            )
            .unwrap(),
            "check"
        );

        let opencode = import_opencode_skills(&registry, &workspace, Some("opencode-qa")).unwrap();
        assert_eq!(opencode.imported.len(), 1);
        assert_eq!(opencode.imported[0].name, "opencode-qa");

        let duplicate = import_codex_skills(&registry, &workspace, Some("codex-qa")).unwrap();
        assert!(duplicate.imported.is_empty());
        assert_eq!(duplicate.skipped[0].name, "codex-qa");
        assert!(duplicate.skipped[0].reason.contains("already installed"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(registry_root);
    }

    #[test]
    fn rejects_skill_without_description() {
        let root = temp_root("bad");
        let skill_dir = root.join("bad-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: bad-skill\n---\nBody\n",
        )
        .unwrap();

        let err = parse_skill_metadata(&skill_dir.join("SKILL.md")).unwrap_err();
        assert!(format!("{err}").contains("description"));

        let _ = fs::remove_dir_all(root);
    }
}
