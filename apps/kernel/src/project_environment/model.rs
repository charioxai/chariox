//! MP-08: Value-free discovery contract. Unknown fields fail closed.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentClassification {
    #[default]
    Secret,
    NonSecret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentEntryKind {
    Variable,
    ConfigFile,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentEntryStatus {
    Found,
    #[default]
    Missing,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentUse {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectEnvironmentLocator {
    EnvFile { path: String, key: String },
    WorkspaceEnvironment { name: String },
    ConfigFile { path: String },
    Vault { service: String, key: String },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentEntry {
    pub name: String,
    pub workspace_id: String,
    pub kind: ProjectEnvironmentEntryKind,
    #[serde(default)]
    pub classification: ProjectEnvironmentClassification,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub excluded: bool,
    pub uses: Vec<ProjectEnvironmentUse>,
    pub locator: ProjectEnvironmentLocator,
    #[serde(default)]
    pub status: ProjectEnvironmentEntryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPrivateFileDecision {
    pub workspace_id: String,
    pub path: String,
    pub bring: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret_looking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentManifest {
    pub schema_version: u32,
    pub project_id: String,
    pub evidence_digest: String,
    pub entries: Vec<ProjectEnvironmentEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_files: Vec<ProjectPrivateFileDecision>,
    #[serde(default)]
    pub toolchain_hints: Vec<String>,
    #[serde(default)]
    pub package_hints: Vec<String>,
    #[serde(default)]
    pub service_hints: Vec<String>,
}

pub fn relative_environment_path(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\0', '\n', '\r', '\\'])
        || Path::new(value)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || value.split('/').any(|part| {
            part == ".git"
                || (part == ".chariox"
                    && !crate::workspace_live_sync_ignore::workspace_source_capability_path(value))
        })
    {
        return Err("invalid Project environment relative path");
    }
    Ok(())
}

pub fn environment_variable_name(value: &str) -> bool {
    let mut chars = value.bytes();
    matches!(chars.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && value.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

impl ProjectEnvironmentManifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.project_id.is_empty()
            || self.project_id.len() > 256
            || self.entries.len() > 512
            || self.evidence_digest.len() != 64
            || !self.evidence_digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid Project environment manifest identity or bounds");
        }
        if self.private_files.len() > 20_000 {
            return Err("private file decision bounds exceeded");
        }
        let mut paths = BTreeSet::new();
        for file in &self.private_files {
            relative_environment_path(&file.path)?;
            if file.workspace_id.is_empty()
                || file.workspace_id.len() > 4096
                || !paths.insert((&file.workspace_id, &file.path))
                || file.reason.is_empty()
                || file.reason.len() > 512
                || file.reason.contains(['\0', '\n', '\r'])
                || (file.bring && file.secret_looking)
                || (file.bring
                    && crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(
                        &file.path,
                    ))
            {
                return Err("invalid private file decision");
            }
        }
        let mut identities = BTreeSet::new();
        let mut assignments = BTreeSet::new();
        for entry in &self.entries {
            if entry.workspace_id.is_empty()
                || entry.workspace_id.len() > 4096
                || !identities.insert((&entry.workspace_id, &entry.name))
                || entry.uses.is_empty()
                || entry.uses.len() > 128
            {
                return Err("invalid Project environment entry binding or evidence");
            }
            match entry.kind {
                ProjectEnvironmentEntryKind::Variable
                    if !environment_variable_name(&entry.name) =>
                {
                    return Err("invalid Project environment variable name")
                }
                ProjectEnvironmentEntryKind::ConfigFile => relative_environment_path(&entry.name)?,
                _ => {}
            }
            for usage in &entry.uses {
                relative_environment_path(&usage.path)?;
                if usage.line == 0 {
                    return Err("invalid Project environment use line");
                }
            }
            match (&entry.kind, &entry.locator) {
                (
                    ProjectEnvironmentEntryKind::Variable,
                    ProjectEnvironmentLocator::EnvFile { path, key },
                ) => {
                    relative_environment_path(path)?;
                    if !assignments.insert((&entry.workspace_id, path, key)) {
                        return Err("duplicate Project environment file assignment");
                    }
                    if !environment_variable_name(key) {
                        return Err("invalid environment file key");
                    }
                }
                (
                    ProjectEnvironmentEntryKind::Variable,
                    ProjectEnvironmentLocator::WorkspaceEnvironment { name },
                ) => {
                    if !environment_variable_name(name) {
                        return Err("invalid workspace environment name");
                    }
                }
                (
                    ProjectEnvironmentEntryKind::ConfigFile,
                    ProjectEnvironmentLocator::ConfigFile { path },
                ) => relative_environment_path(path)?,
                (_, ProjectEnvironmentLocator::Vault { service, key })
                    if !service.is_empty()
                        && !key.is_empty()
                        && service.len() <= 256
                        && key.len() <= 256 => {}
                (_, ProjectEnvironmentLocator::Missing) => {}
                _ => return Err("Project environment locator does not match entry kind"),
            }
        }
        for hint in self
            .toolchain_hints
            .iter()
            .chain(&self.package_hints)
            .chain(&self.service_hints)
        {
            if hint.len() > 512 || hint.contains(['\0', '\n', '\r', '=']) {
                return Err("invalid Project environment hint");
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| "invalid Project environment manifest")?
            .len()
            > 1024 * 1024
        {
            return Err("Project environment manifest exceeds bounds");
        }
        Ok(())
    }
}

// MP-08 / MP-10: Host/provider process controls are recreated by the target, never exported.
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
