use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

mod package;
mod provider_import;
#[cfg(test)]
mod tests;

use package::package_skill_directory;
pub(crate) use package::remote_skill_materialization_base;
pub use package::{materialize_skill_package, CharioxSkillPackage, CharioxSkillPackageFile};
pub use provider_import::{
    discover_provider_skill_import_candidates, import_claude_skills, import_codex_skills,
    import_opencode_skills, ProviderSkillImportCandidate, ProviderSkillImportDiscovery,
    SkillImportOutcome, SkillImportSkip,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxSkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharioxSkillRegistry {
    roots: Vec<PathBuf>,
}

impl CharioxSkillRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".chariox").join("skills")
    }

    pub fn user_root() -> Option<PathBuf> {
        if let Some(root) = managed_capability_root() {
            return Some(root.join("user").join("skills"));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".chariox").join("skills"))
    }

    pub fn install_from_path(
        &self,
        source: &Path,
    ) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
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
        let temp_dir = prepare_skill_directory_from_source(root, source, "skill.install")?;
        fs::rename(&temp_dir, &destination).map_err(|error| {
            let _ = fs::remove_dir_all(&temp_dir);
            DaemonError::LocalTransport {
                operation: "skill.install",
                message: format!(
                    "failed to publish skill `{}` at `{}`: {error}",
                    metadata.name,
                    destination.display()
                ),
            }
        })?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn update_from_path(
        &self,
        source: &Path,
    ) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
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
        let root = destination
            .parent()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "skill.update",
                message: format!(
                    "failed to resolve parent registry for skill `{}` at `{}`",
                    metadata.name,
                    destination.display()
                ),
            })?;
        let temp_dir = prepare_skill_directory_from_source(root, source, "skill.update")?;
        publish_prepared_skill_directory("skill.update", &temp_dir, &destination, &metadata.name)?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn upsert_from_path(
        &self,
        source: &Path,
    ) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
        if !source.is_dir() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.upsert.path",
                message: format!("skill source `{}` must be a directory", source.display()),
            });
        }
        let source_skill_md = source.join("SKILL.md");
        if !source_skill_md.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "skill.upsert.path",
                message: format!("skill source `{}` must contain SKILL.md", source.display()),
            });
        }
        let metadata = parse_skill_metadata(&source_skill_md)?;
        let root = self.primary_root()?;
        fs::create_dir_all(root).map_err(|error| DaemonError::LocalTransport {
            operation: "skill.upsert.path",
            message: format!(
                "failed to create skill registry `{}`: {error}",
                root.display()
            ),
        })?;
        let destination = root.join(&metadata.name);
        if let (Ok(source_canonical), Ok(destination_canonical)) =
            (fs::canonicalize(source), fs::canonicalize(&destination))
        {
            if source_canonical == destination_canonical {
                return Err(DaemonError::LocalTransport {
                    operation: "skill.upsert.path",
                    message: "skill upsert source must not be the installed destination"
                        .to_string(),
                });
            }
        }
        let temp_dir = prepare_skill_directory_from_source(root, source, "skill.upsert.path")?;
        publish_prepared_skill_directory(
            "skill.upsert.path",
            &temp_dir,
            &destination,
            &metadata.name,
        )?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn upsert_from_content(
        &self,
        skill_md: &str,
    ) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
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
        publish_prepared_skill_directory("skill.upsert", &temp_dir, &destination, &metadata.name)?;
        let installed = parse_skill_metadata(&destination.join("SKILL.md"))?;
        Ok((installed, destination))
    }

    pub fn upsert_from_url(
        &self,
        url: &str,
    ) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
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

    pub fn uninstall(&self, name: &str) -> Result<(CharioxSkillMetadata, PathBuf), DaemonError> {
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

    pub fn list(&self) -> Result<Vec<CharioxSkillMetadata>, DaemonError> {
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

    pub fn get(&self, name: &str) -> Result<Option<CharioxSkillMetadata>, DaemonError> {
        validate_registry_name(name, "skill name")?;
        let Some(skill_dir) = self.find_skill_dir(name)? else {
            return Ok(None);
        };
        parse_skill_metadata(&skill_dir.join("SKILL.md")).map(Some)
    }

    pub fn package(&self, name: &str) -> Result<Option<CharioxSkillPackage>, DaemonError> {
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

pub(crate) fn format_granted_skill_prompt_context(
    agent_ref: &str,
    skill_grants: &[String],
    workspace: impl AsRef<Path>,
    prompt: &str,
) -> Result<String, DaemonError> {
    if skill_grants.is_empty() {
        return Ok(String::new());
    }
    let _ = workspace.as_ref();
    let roots = CharioxSkillRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    let registry = CharioxSkillRegistry::new(roots);
    let mut summaries = Vec::new();
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
        summaries.push(format!("- `{}`: {}", skill.name, summary));
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
    let full_instructions = requested_skill_bodies
        .iter()
        .map(|(name, body)| {
            format!(
                "<chariox_skill name=\"{name}\">\n{}\n</chariox_skill>",
                body.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rendered = crate::prompt_assembly::render_configured_prompt(
        "runtime/skill-context",
        crate::prompt_assembly::bundled_skill_context_template(),
        &[
            ("AGENT_SCOPE", "this agent"),
            ("SKILL_SUMMARIES", &summaries.join("\n")),
            ("FULL_INSTRUCTIONS", &full_instructions),
        ],
    );
    Ok(crate::prompt_assembly::prompt_component(
        "skill-context-instructions",
        &rendered,
    ))
}

pub fn parse_skill_metadata(skill_md: &Path) -> Result<CharioxSkillMetadata, DaemonError> {
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
    Ok(CharioxSkillMetadata {
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

fn prepare_skill_directory_from_source(
    root: &Path,
    source: &Path,
    operation: &'static str,
) -> Result<PathBuf, DaemonError> {
    let temp_dir = unique_skill_temp_dir(root, operation);
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to remove stale skill temp dir `{}`: {error}",
                temp_dir.display()
            ),
        })?;
    }
    if let Err(error) = copy_directory(source, &temp_dir) {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(error);
    }
    let _ = parse_skill_metadata(&temp_dir.join("SKILL.md")).map_err(|error| {
        let _ = fs::remove_dir_all(&temp_dir);
        error
    })?;
    Ok(temp_dir)
}

fn publish_prepared_skill_directory(
    operation: &'static str,
    temp_dir: &Path,
    destination: &Path,
    skill_name: &str,
) -> Result<(), DaemonError> {
    let backup_dir = destination.with_file_name(format!(
        ".skill-backup-{}-{}-{}",
        skill_name,
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let had_existing = destination.exists();
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir).map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to remove stale skill backup `{}`: {error}",
                backup_dir.display()
            ),
        })?;
    }
    if had_existing {
        fs::rename(destination, &backup_dir).map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to stage replacement for skill `{skill_name}` at `{}`: {error}",
                destination.display()
            ),
        })?;
    }
    match fs::rename(temp_dir, destination) {
        Ok(()) => {
            if had_existing {
                let _ = fs::remove_dir_all(&backup_dir);
            }
            Ok(())
        }
        Err(error) => {
            if had_existing {
                let _ = fs::rename(&backup_dir, destination);
            }
            let _ = fs::remove_dir_all(temp_dir);
            Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "failed to publish skill `{skill_name}` at `{}`: {error}",
                    destination.display()
                ),
            })
        }
    }
}

fn unique_skill_temp_dir(root: &Path, operation: &str) -> PathBuf {
    let label = operation.replace(['.', ':'], "-");
    root.join(format!(
        ".skill-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ))
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

fn managed_capability_root() -> Option<PathBuf> {
    std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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
