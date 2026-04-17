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
