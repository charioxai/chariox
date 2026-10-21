//! MP-08 / MP-10 / MP-11: skill roots of the Claude plugins the resolved profile
//! has installed and enabled. Claude names plugin skills `<plugin>:<skill>`, so a
//! plugin skill never shadows an account or project skill of the same name.
//! Only `installed_plugins.json`, `enabledPlugins` and the plugin manifest's
//! name and skill paths are read.
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

pub(super) struct ClaudePluginSkillRoot {
    pub plugin: String,
    pub root: PathBuf,
}

/// `settings` are in increasing precedence (account, then project files); a
/// later `enabledPlugins` value for the same plugin wins. `project_dirs` are the
/// directories a project- or local-scope install may name as its project.
pub(super) fn claude_plugin_skill_roots(
    config: &Path,
    settings: &[PathBuf],
    project_dirs: &[&Path],
) -> Vec<ClaudePluginSkillRoot> {
    let mut enabled = BTreeMap::new();
    for value in settings.iter().filter_map(|file| read_json(file)) {
        if let Some(plugins) = value["enabledPlugins"].as_object() {
            for (key, state) in plugins {
                enabled.insert(key.clone(), state.as_bool() == Some(true));
            }
        }
    }
    let Some(installed) = read_json(&config.join("plugins/installed_plugins.json")) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    for (key, installs) in installed["plugins"].as_object().into_iter().flatten() {
        if enabled.get(key) != Some(&true) {
            continue;
        }
        // Version 2 lists one install per scope; version 1 kept a single object.
        let installs = installs
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![installs.clone()]);
        let Some(install) = installs.iter().find(|install| {
            install["projectPath"]
                .as_str()
                .is_none_or(|project| project_dirs.iter().any(|dir| Path::new(project) == *dir))
        }) else {
            continue;
        };
        let Some(path) = install["installPath"].as_str().map(PathBuf::from) else {
            continue;
        };
        if !path.is_absolute() {
            continue;
        }
        let manifest = read_json(&path.join(".claude-plugin/plugin.json"));
        let plugin = manifest
            .as_ref()
            .and_then(|manifest| manifest["name"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| key.split('@').next().unwrap_or(key).to_string());
        let mut skill_dirs = vec![path.join("skills")];
        if let Some(manifest) = &manifest {
            let custom = match &manifest["skills"] {
                serde_json::Value::String(dir) => vec![dir.as_str()],
                serde_json::Value::Array(dirs) => dirs.iter().filter_map(|d| d.as_str()).collect(),
                _ => Vec::new(),
            };
            skill_dirs.extend(
                custom
                    .into_iter()
                    .map(Path::new)
                    .filter(|dir| {
                        dir.components()
                            .all(|c| matches!(c, Component::CurDir | Component::Normal(_)))
                    })
                    .map(|dir| path.join(dir)),
            );
        }
        roots.extend(skill_dirs.into_iter().map(|root| ClaudePluginSkillRoot {
            plugin: plugin.clone(),
            root,
        }));
    }
    roots
}

fn read_json(file: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read(file)
        .ok()
        .filter(|raw| raw.len() <= 1 << 20)?;
    serde_json::from_slice(&raw).ok()
}
