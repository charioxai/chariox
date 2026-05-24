use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaSkillPackage {
    pub metadata: ArrobaSkillMetadata,
    pub version_hash: String,
    pub files: Vec<ArrobaSkillPackageFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaSkillPackageFile {
    pub path: String,
    pub sha256: String,
    pub content_base64: String,
}

const MAX_SKILL_PACKAGE_FILES: usize = 512;
const MAX_SKILL_PACKAGE_BYTES: u64 = 10 * 1024 * 1024;
const SKILL_PACKAGE_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".DS_Store",
    "node_modules",
    ".venv",
    "venv",
    "target",
    "dist",
    "build",
];

impl ArrobaSkillRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        if let Some(root) = managed_capability_root() {
            return root
                .join("project")
                .join(workspace_registry_hash(workspace.as_ref()))
                .join("skills");
        }
        workspace.as_ref().join(".arroba").join("skills")
    }

    pub fn user_root() -> Option<PathBuf> {
        if let Some(root) = managed_capability_root() {
            return Some(root.join("user").join("skills"));
        }
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

    pub fn update_from_path(
        &self,
        source: &Path,
    ) -> Result<(ArrobaSkillMetadata, PathBuf), DaemonError> {
        if !source.is_dir() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.update",
                message: format!("skill source `{}` must be a directory", source.display()),
            });
        }
        let source_skill_md = source.join("SKILL.md");
        if !source_skill_md.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.update",
                message: format!("skill source `{}` must contain SKILL.md", source.display()),
            });
        }
        let metadata = parse_skill_metadata(&source_skill_md)?;
        let destination =
            self.find_skill_dir(&metadata.name)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "skill.update",
                    message: format!("skill `{}` is not installed", metadata.name),
                })?;
        if let (Ok(source_canonical), Ok(destination_canonical)) =
            (fs::canonicalize(source), fs::canonicalize(&destination))
        {
            if source_canonical == destination_canonical {
                return Err(DaemonError::LocalTransport {
                    operation: "skill.update",
                    message: "skill update source must not be the installed destination"
                        .to_string(),
                });
            }
        }
        fs::remove_dir_all(&destination).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.update",
            message: format!(
                "failed to replace skill `{}` at `{}`: {error}",
                metadata.name,
                destination.display()
            ),
        })?;
        copy_directory(source, &destination)?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn upsert_from_content(
        &self,
        skill_md: &str,
    ) -> Result<(ArrobaSkillMetadata, PathBuf), DaemonError> {
        let root = self.primary_root()?;
        fs::create_dir_all(root).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.upsert",
            message: format!(
                "failed to create skill registry `{}`: {error}",
                root.display()
            ),
        })?;
        let temp_dir = root.join(format!(
            ".skill-upsert-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).map_err(|error| DaemonError::LocalTransport {
                operation: "skill.upsert",
                message: format!(
                    "failed to remove stale skill temp dir `{}`: {error}",
                    temp_dir.display()
                ),
            })?;
        }
        fs::create_dir_all(&temp_dir).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.upsert",
            message: format!(
                "failed to create skill temp dir `{}`: {error}",
                temp_dir.display()
            ),
        })?;
        fs::write(temp_dir.join("SKILL.md"), skill_md).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "skill.upsert",
                message: format!("failed to write skill content: {error}"),
            }
        })?;
        let metadata = parse_skill_metadata(&temp_dir.join("SKILL.md"))?;
        let destination = root.join(&metadata.name);
        if destination.exists() {
            fs::remove_dir_all(&destination).map_err(|error| DaemonError::LocalTransport {
                operation: "skill.upsert",
                message: format!(
                    "failed to replace skill `{}` at `{}`: {error}",
                    metadata.name,
                    destination.display()
                ),
            })?;
        }
        fs::rename(&temp_dir, &destination).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.upsert",
            message: format!(
                "failed to publish skill `{}` at `{}`: {error}",
                metadata.name,
                destination.display()
            ),
        })?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn upsert_from_url(
        &self,
        url: &str,
    ) -> Result<(ArrobaSkillMetadata, PathBuf), DaemonError> {
        let url = skill_content_url(url)?;
        let response = ureq::get(&url)
            .call()
            .map_err(|error| skill_url_error("skill.upsert.url", error))?;
        let skill_md = response
            .into_string()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.upsert.url",
                message: format!("failed to read skill content: {error}"),
            })?;
        if skill_md.len() > 512 * 1024 {
            return Err(DaemonError::InvalidConfig {
                field: "skill url",
                message: "downloaded SKILL.md must be 512 KiB or smaller",
            });
        }
        self.upsert_from_content(&skill_md)
    }

    pub fn uninstall(&self, name: &str) -> Result<(ArrobaSkillMetadata, PathBuf), DaemonError> {
        validate_registry_name(name, "skill name")?;
        let destination =
            self.find_skill_dir(name)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "skill.uninstall",
                    message: format!("skill `{name}` is not installed"),
                })?;
        let metadata = parse_skill_metadata(&destination.join("SKILL.md"))?;
        fs::remove_dir_all(&destination).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.uninstall",
            message: format!(
                "failed to remove skill `{}`: {error}",
                destination.display()
            ),
        })?;
        Ok((metadata, destination))
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
        let Some(skill_dir) = self.find_skill_dir(name)? else {
            return Ok(None);
        };
        parse_skill_metadata(&skill_dir.join("SKILL.md")).map(Some)
    }

    pub fn package(&self, name: &str) -> Result<Option<ArrobaSkillPackage>, DaemonError> {
        validate_registry_name(name, "skill name")?;
        let Some(skill_dir) = self.find_skill_dir(name)? else {
            return Ok(None);
        };
        package_skill_directory(&skill_dir).map(Some)
    }

    fn find_skill_dir(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        validate_registry_name(name, "skill name")?;
        for root in &self.roots {
            let skill_dir = root.join(name);
            let skill_md = skill_dir.join("SKILL.md");
            if skill_md.exists() {
                return Ok(Some(skill_dir));
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

pub fn materialize_skill_package(
    base_dir: &Path,
    package: &ArrobaSkillPackage,
) -> Result<PathBuf, DaemonError> {
    validate_registry_name(&package.metadata.name, "skill name")?;
    let destination = base_dir
        .join(&package.metadata.name)
        .join(&package.version_hash);
    if destination.join("SKILL.md").exists() {
        return Ok(destination);
    }
    let temp_destination = base_dir.join(format!(
        ".{}.{}.tmp",
        package.metadata.name,
        std::process::id()
    ));
    if temp_destination.exists() {
        fs::remove_dir_all(&temp_destination).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "failed to clear stale skill materialization temp dir `{}`: {error}",
                temp_destination.display()
            ),
        })?;
    }
    fs::create_dir_all(&temp_destination).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.materialize",
        message: format!(
            "failed to create skill materialization temp dir `{}`: {error}",
            temp_destination.display()
        ),
    })?;
    for file in &package.files {
        let relative_path = validate_package_relative_path(&file.path, "skill.materialize")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&file.content_base64)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "skill package file `{}` has invalid base64 content: {error}",
                    file.path
                ),
            })?;
        let actual_hash = sha256_hex(&bytes);
        if actual_hash != file.sha256 {
            return Err(DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "skill package file `{}` hash mismatch: expected {}, got {}",
                    file.path, file.sha256, actual_hash
                ),
            });
        }
        let path = temp_destination.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
                operation: "skill.materialize",
                message: format!(
                    "failed to create skill materialization dir `{}`: {error}",
                    parent.display()
                ),
            })?;
        }
        fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!("failed to write skill file `{}`: {error}", path.display()),
        })?;
    }
    let materialized = package_skill_directory(&temp_destination)?;
    if materialized.version_hash != package.version_hash {
        return Err(DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "materialized skill package hash mismatch: expected {}, got {}",
                package.version_hash, materialized.version_hash
            ),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.materialize",
            message: format!(
                "failed to create skill materialization parent `{}`: {error}",
                parent.display()
            ),
        })?;
    }
    if destination.exists() {
        fs::remove_dir_all(&temp_destination).ok();
        return Ok(destination);
    }
    fs::rename(&temp_destination, &destination).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.materialize",
        message: format!(
            "failed to publish materialized skill `{}`: {error}",
            destination.display()
        ),
    })?;
    Ok(destination)
}

pub(crate) fn remote_skill_materialization_base(workspace: impl AsRef<Path>) -> PathBuf {
    if let Some(root) = managed_capability_root() {
        return root.join("remote").join("skills");
    }
    workspace
        .as_ref()
        .join(".arroba")
        .join("remote")
        .join("skills")
}

pub(crate) fn format_granted_skill_prompt_context(
    agent_ref: &str,
    skill_grants: &[String],
    workspace: impl AsRef<Path>,
    prompt: &str,
) -> Result<String, DaemonError> {
    if skill_grants.is_empty() {
        return Ok(String::new());
    }
    let workspace = workspace.as_ref();
    let mut roots = vec![ArrobaSkillRegistry::project_root(workspace)];
    if let Some(user_root) = ArrobaSkillRegistry::user_root() {
        roots.push(user_root);
    }
    let registry = ArrobaSkillRegistry::new(roots);
    let mut lines = vec![
        "Available Arroba skills for this agent:".to_string(),
        "Use these granted skills as routing hints when they match the task. If a skill is explicitly selected, mentioned, or requested below, follow its full instructions.".to_string(),
    ];
    let mut requested_skill_bodies = Vec::new();
    for grant in skill_grants {
        let Some(skill) = registry.get(grant)? else {
            return Err(DaemonError::LocalTransport {
                operation: "provider.prompt.skills",
                message: format!("agent `{agent_ref}` has missing skill grant `{grant}`"),
            });
        };
        let summary = skill
            .short_description
            .as_ref()
            .unwrap_or(&skill.description);
        lines.push(format!("- `{}`: {}", skill.name, summary));
        if prompt_explicitly_requests_skill(prompt, &skill.name) {
            let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "provider.prompt.skills",
                    message: format!(
                        "failed to read skill `{}` body at `{}`: {error}",
                        skill.name,
                        skill.path.display()
                    ),
                }
            })?;
            requested_skill_bodies.push((skill.name, body));
        }
    }
    if !requested_skill_bodies.is_empty() {
        lines.push(String::new());
        lines.push("Full instructions for explicitly requested Arroba skills:".to_string());
        for (name, body) in requested_skill_bodies {
            lines.push(format!("<arroba_skill name=\"{name}\">"));
            lines.push(body.trim().to_string());
            lines.push("</arroba_skill>".to_string());
        }
    }
    Ok(lines.join("\n"))
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

fn skill_content_url(input: &str) -> Result<String, DaemonError> {
    let trimmed = input.trim();
    let parsed = url::Url::parse(trimmed).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.url",
        message: format!("invalid skill url: {error}"),
    })?;
    if parsed.scheme() != "https" {
        return Err(DaemonError::InvalidConfig {
            field: "skill url",
            message: "skill URL must use https",
        });
    }
    if parsed.host_str() == Some("github.com") {
        let segments = parsed
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>())
            .unwrap_or_default();
        if segments.len() >= 5 && matches!(segments[2], "blob" | "tree") {
            let owner = segments[0];
            let repo = segments[1];
            let branch = segments[3];
            let mut path = segments[4..].join("/");
            if segments[2] == "tree" && !path.ends_with("SKILL.md") {
                path = format!("{}/SKILL.md", path.trim_end_matches('/'));
            }
            return Ok(format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}"
            ));
        }
    }
    if parsed.host_str() == Some("raw.githubusercontent.com") || parsed.path().ends_with("SKILL.md")
    {
        return Ok(trimmed.to_string());
    }
    Err(DaemonError::InvalidConfig {
        field: "skill url",
        message: "skill URL must point to SKILL.md or a GitHub skill directory",
    })
}

fn skill_url_error(operation: &'static str, error: ureq::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
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

fn package_skill_directory(skill_dir: &Path) -> Result<ArrobaSkillPackage, DaemonError> {
    let metadata = parse_skill_metadata(&skill_dir.join("SKILL.md"))?;
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    collect_skill_package_files(skill_dir, skill_dir, &mut files, &mut total_bytes)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    for file in &files {
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\0");
    }
    let version_hash = hex_digest(hasher.finalize().as_slice());
    Ok(ArrobaSkillPackage {
        metadata,
        version_hash,
        files,
    })
}

fn collect_skill_package_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<ArrobaSkillPackageFile>,
    total_bytes: &mut u64,
) -> Result<(), DaemonError> {
    for entry in fs::read_dir(dir).map_err(|error| DaemonError::LocalTransport {
        operation: "skill.package",
        message: format!(
            "failed to read skill directory `{}`: {error}",
            dir.display()
        ),
    })? {
        let entry = entry.map_err(|error| DaemonError::LocalTransport {
            operation: "skill.package",
            message: format!("failed to read skill directory entry: {error}"),
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!("failed to inspect skill path `{}`: {error}", path.display()),
            })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if SKILL_PACKAGE_IGNORED_DIRS
                .iter()
                .any(|ignored| *ignored == file_name)
            {
                continue;
            }
            collect_skill_package_files(root, &path, files, total_bytes)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if files.len() >= MAX_SKILL_PACKAGE_FILES {
            return Err(DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!(
                    "skill package exceeds maximum file count ({MAX_SKILL_PACKAGE_FILES})"
                ),
            });
        }
        let relative_path =
            path.strip_prefix(root)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "skill.package",
                    message: format!(
                        "skill path `{}` is outside package root `{}`: {error}",
                        path.display(),
                        root.display()
                    ),
                })?;
        let relative_path =
            validate_package_relative_path(&relative_path.to_string_lossy(), "skill.package")?;
        let bytes = fs::read(&path).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.package",
            message: format!("failed to read skill file `{}`: {error}", path.display()),
        })?;
        *total_bytes += bytes.len() as u64;
        if *total_bytes > MAX_SKILL_PACKAGE_BYTES {
            return Err(DaemonError::LocalTransport {
                operation: "skill.package",
                message: format!(
                    "skill package exceeds maximum byte size ({MAX_SKILL_PACKAGE_BYTES})"
                ),
            });
        }
        files.push(ArrobaSkillPackageFile {
            path: relative_path.to_string_lossy().replace('\\', "/"),
            sha256: sha256_hex(&bytes),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(())
}

fn validate_package_relative_path(
    path: &str,
    operation: &'static str,
) -> Result<PathBuf, DaemonError> {
    let relative_path = PathBuf::from(path);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("skill package path `{path}` must be relative and contained"),
        });
    }
    Ok(relative_path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(digest.as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn managed_capability_root() -> Option<PathBuf> {
    std::env::var_os("ARROBA_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn workspace_registry_hash(workspace: &Path) -> String {
    hex_digest(Sha256::digest(workspace.to_string_lossy().as_bytes()).as_slice())
}

fn prompt_explicitly_requests_skill(prompt: &str, skill_name: &str) -> bool {
    let prompt = prompt.to_lowercase();
    let skill_name = skill_name.to_lowercase();
    let explicit_markers = [
        format!("@{skill_name}"),
        format!("`{skill_name}`"),
        format!("/skill {skill_name}"),
        format!("skill {skill_name}"),
        format!("use {skill_name}"),
        format!("using {skill_name}"),
        format!("with {skill_name}"),
    ];
    explicit_markers
        .iter()
        .any(|marker| prompt.contains(marker))
        || contains_tokenish_skill_name(&prompt, &skill_name)
}

fn contains_tokenish_skill_name(prompt: &str, skill_name: &str) -> bool {
    prompt.match_indices(skill_name).any(|(index, _)| {
        let before = index
            .checked_sub(1)
            .and_then(|before| prompt.as_bytes().get(before))
            .copied();
        let after = prompt.as_bytes().get(index + skill_name.len()).copied();
        is_skill_boundary(before) && is_skill_boundary(after)
    })
}

fn is_skill_boundary(byte: Option<u8>) -> bool {
    byte.is_none_or(|byte| !byte.is_ascii_alphanumeric() && byte != b'-' && byte != b'_')
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
    fn registry_and_remote_materialization_roots_can_be_isolated_for_managed_slice_runtime() {
        let _guard = crate::env_lock::lock();
        let isolation_root = temp_root("managed-slice-isolation");
        std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &isolation_root);

        let project_root = ArrobaSkillRegistry::project_root("/workspace");
        let user_root = ArrobaSkillRegistry::user_root().expect("user root should resolve");
        let remote_root = remote_skill_materialization_base("/workspace");

        std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT");
        let _ = fs::remove_dir_all(&isolation_root);

        assert!(project_root.starts_with(isolation_root.join("project")));
        assert!(project_root.ends_with("skills"));
        assert_eq!(user_root, isolation_root.join("user").join("skills"));
        assert_eq!(remote_root, isolation_root.join("remote").join("skills"));
    }

    #[test]
    fn detects_explicit_skill_requests() {
        assert!(prompt_explicitly_requests_skill(
            "Use browser-qa to validate this flow",
            "browser-qa"
        ));
        assert!(prompt_explicitly_requests_skill(
            "Please apply @release_check",
            "release_check"
        ));
        assert!(prompt_explicitly_requests_skill(
            "Run the `security-review` skill",
            "security-review"
        ));
        assert!(!prompt_explicitly_requests_skill(
            "This browser-qa-extra text is another skill",
            "browser-qa"
        ));
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
    fn update_replaces_and_uninstall_removes_existing_skill() {
        let source_root = temp_root("update-source");
        let registry_root = temp_root("update-registry");
        let original_dir = source_root.join("original");
        let updated_dir = source_root.join("updated");
        fs::create_dir_all(&original_dir).unwrap();
        fs::write(
            original_dir.join("SKILL.md"),
            "---\nname: browser-qa\ndescription: Old QA\n---\nOld body.\n",
        )
        .unwrap();
        fs::write(original_dir.join("old.txt"), "old").unwrap();
        fs::create_dir_all(updated_dir.join("assets")).unwrap();
        fs::write(
            updated_dir.join("SKILL.md"),
            "---\nname: browser-qa\ndescription: New QA\n---\nNew body.\n",
        )
        .unwrap();
        fs::write(updated_dir.join("assets").join("new.txt"), "new").unwrap();

        let registry = ArrobaSkillRegistry::new(vec![registry_root.clone()]);
        registry.install_from_path(&original_dir).unwrap();
        let (updated, destination) = registry.update_from_path(&updated_dir).unwrap();
        assert_eq!(updated.description, "New QA");
        assert_eq!(destination, registry_root.join("browser-qa"));
        assert!(!destination.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(destination.join("assets").join("new.txt")).unwrap(),
            "new"
        );

        let (removed, removed_path) = registry.uninstall("browser-qa").unwrap();
        assert_eq!(removed.name, "browser-qa");
        assert_eq!(removed_path, registry_root.join("browser-qa"));
        assert_eq!(registry.get("browser-qa").unwrap(), None);
        assert!(!removed_path.exists());

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
    fn packages_and_materializes_complete_skill_directory() {
        let source_root = temp_root("package-source");
        let materialized_root = temp_root("package-materialized");
        let skill_dir = source_root.join("browser-qa");
        fs::create_dir_all(skill_dir.join("assets").join("nested")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: browser-qa\ndescription: Browser QA workflow\n---\nUse assets/prompt.txt.\n",
        )
        .unwrap();
        fs::write(skill_dir.join("assets").join("prompt.txt"), "qa checklist").unwrap();
        fs::write(
            skill_dir.join("assets").join("nested").join("fixture.json"),
            "{\"ok\":true}",
        )
        .unwrap();

        let package = package_skill_directory(&skill_dir).unwrap();
        assert_eq!(package.metadata.name, "browser-qa");
        assert!(package.files.iter().any(|file| file.path == "SKILL.md"));
        assert!(package
            .files
            .iter()
            .any(|file| file.path == "assets/prompt.txt"));
        assert!(package
            .files
            .iter()
            .any(|file| file.path == "assets/nested/fixture.json"));

        let destination = materialize_skill_package(&materialized_root, &package).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            fs::read_to_string(skill_dir.join("SKILL.md")).unwrap()
        );
        assert_eq!(
            fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
            "qa checklist"
        );
        assert_eq!(
            package_skill_directory(&destination).unwrap().version_hash,
            package.version_hash
        );

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(materialized_root);
    }

    #[test]
    fn package_skips_symlinks_and_ignored_directories() {
        let root = temp_root("package-symlink");
        let skill_dir = root.join("safe");
        fs::create_dir_all(skill_dir.join(".git")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: safe\ndescription: Safe skill\n---\nBody\n",
        )
        .unwrap();
        fs::write(skill_dir.join(".git").join("config"), "secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", skill_dir.join("passwd-link")).unwrap();

        let package = package_skill_directory(&skill_dir).unwrap();
        assert!(package
            .files
            .iter()
            .all(|file| !file.path.starts_with(".git")));
        assert!(package.files.iter().all(|file| file.path != "passwd-link"));

        let _ = fs::remove_dir_all(root);
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
